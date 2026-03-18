//! Control Flow Graph (CFG) construction for Rust functions.
//!
//! This module provides best-effort CFG construction for functions and methods.
//! It walks the function body AST (parsed via `syn`) and builds a directed graph
//! of [`CfgNode`] and [`CfgEdge`] values using `petgraph::DiGraph`.
//!
//! ## Phase 1 Scope
//!
//! - Sequential statements become `Statement` nodes with `Sequential` edges.
//! - `if` / `match` expressions become `Branch` nodes with `TrueBranch` / `FalseBranch` edges.
//! - `loop` / `while` / `for` become `Loop` nodes with `LoopBack` / `LoopExit` edges.
//! - `.await` expressions become `Await` nodes.
//! - `return` statements become `Return` nodes with an edge to `Exit`.
//! - Closures, `?` operator chains, and other complex constructs are simplified.

use std::collections::HashMap;

use petgraph::graph::{DiGraph, NodeIndex};
use syn::visit::Visit;

use spectron_core::{FileId, SourceSpan, SymbolId};

// ---------------------------------------------------------------------------
// CfgNode
// ---------------------------------------------------------------------------

/// A node in a control flow graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CfgNode {
    /// Entry point of the function.
    Entry,
    /// Exit point of the function.
    Exit,
    /// A sequential statement.
    Statement { span: SourceSpan },
    /// A branch point (if/match).
    Branch { span: SourceSpan },
    /// A loop construct (loop/while/for).
    Loop { span: SourceSpan },
    /// An await point.
    Await { span: SourceSpan },
    /// A return statement.
    Return { span: SourceSpan },
}

// ---------------------------------------------------------------------------
// CfgEdge
// ---------------------------------------------------------------------------

/// An edge in a control flow graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CfgEdge {
    /// Sequential flow from one statement to the next.
    Sequential,
    /// True branch of a conditional (if-then, match arm taken).
    TrueBranch,
    /// False branch of a conditional (else, match fallthrough).
    FalseBranch,
    /// Back edge from the end of a loop body to the loop head.
    LoopBack,
    /// Edge from the loop head to after the loop (loop exit condition).
    LoopExit,
}

// ---------------------------------------------------------------------------
// ControlFlowGraph
// ---------------------------------------------------------------------------

/// A control flow graph for a single function or method.
#[derive(Clone, Debug)]
pub struct ControlFlowGraph {
    /// The symbol ID of the function this CFG belongs to.
    pub function_id: SymbolId,
    /// The directed graph of CFG nodes and edges.
    pub graph: DiGraph<CfgNode, CfgEdge>,
}

// ---------------------------------------------------------------------------
// CfgBuilder (internal)
// ---------------------------------------------------------------------------

/// Internal builder that constructs a CFG for a single function body.
struct CfgBuilder {
    graph: DiGraph<CfgNode, CfgEdge>,
    /// The Entry node index.
    entry: NodeIndex,
    /// The Exit node index.
    exit: NodeIndex,
    /// File ID for spans.
    file_id: FileId,
}

impl CfgBuilder {
    /// Create a new builder with Entry and Exit nodes already added.
    fn new(file_id: FileId) -> Self {
        let mut graph = DiGraph::new();
        let entry = graph.add_node(CfgNode::Entry);
        let exit = graph.add_node(CfgNode::Exit);
        Self {
            graph,
            entry,
            exit,
            file_id,
        }
    }

    /// Build a span from a `syn` spanned item.
    fn make_span(&self, spanned: &impl syn::spanned::Spanned) -> SourceSpan {
        let span = spanned.span();
        let start = span.start();
        let end = span.end();
        SourceSpan::new(
            self.file_id,
            start.line as u32,
            start.column as u32,
            end.line as u32,
            end.column as u32,
        )
    }

    /// Process a block of statements and return the last reachable node(s).
    ///
    /// `predecessors` is the set of nodes that flow into the first statement.
    /// Returns the set of nodes that flow out of the last statement
    /// (i.e., nodes that should connect to whatever comes next).
    fn process_block(
        &mut self,
        stmts: &[syn::Stmt],
        predecessors: Vec<NodeIndex>,
    ) -> Vec<NodeIndex> {
        let mut current = predecessors;

        for stmt in stmts {
            if current.is_empty() {
                // Unreachable code after a return -- skip remaining statements.
                break;
            }
            current = self.process_stmt(stmt, current);
        }

        current
    }

    /// Process a single statement, returning the outgoing node(s).
    fn process_stmt(
        &mut self,
        stmt: &syn::Stmt,
        predecessors: Vec<NodeIndex>,
    ) -> Vec<NodeIndex> {
        match stmt {
            syn::Stmt::Local(local) => {
                // `let x = expr;` -- treat as a statement node.
                // Check if the initializer contains a control flow expression.
                if let Some(init) = &local.init {
                    if let Some(result) = self.try_process_control_flow_expr(&init.expr, &predecessors) {
                        return result;
                    }
                }
                let span = self.make_span(local);
                let node = self.graph.add_node(CfgNode::Statement { span });
                for &pred in &predecessors {
                    self.graph.add_edge(pred, node, CfgEdge::Sequential);
                }
                vec![node]
            }
            syn::Stmt::Item(_) => {
                // Nested items (fn, struct, etc.) don't affect control flow.
                predecessors
            }
            syn::Stmt::Expr(expr, _semi) => {
                self.process_expr(expr, predecessors)
            }
            syn::Stmt::Macro(mac) => {
                // Macro invocations are treated as simple statements.
                let span = self.make_span(mac);
                let node = self.graph.add_node(CfgNode::Statement { span });
                for &pred in &predecessors {
                    self.graph.add_edge(pred, node, CfgEdge::Sequential);
                }
                vec![node]
            }
        }
    }

    /// Process an expression, handling control flow constructs.
    /// Returns the outgoing node(s) from this expression.
    fn process_expr(
        &mut self,
        expr: &syn::Expr,
        predecessors: Vec<NodeIndex>,
    ) -> Vec<NodeIndex> {
        match expr {
            syn::Expr::If(expr_if) => self.process_if(expr_if, predecessors),
            syn::Expr::Match(expr_match) => self.process_match(expr_match, predecessors),
            syn::Expr::Loop(expr_loop) => self.process_loop(expr_loop, predecessors),
            syn::Expr::While(expr_while) => self.process_while(expr_while, predecessors),
            syn::Expr::ForLoop(expr_for) => self.process_for(expr_for, predecessors),
            syn::Expr::Return(expr_return) => self.process_return(expr_return, predecessors),
            syn::Expr::Await(expr_await) => self.process_await(expr_await, predecessors),
            syn::Expr::Block(expr_block) => {
                self.process_block(&expr_block.block.stmts, predecessors)
            }
            _ => {
                // Check for .await inside any expression
                if contains_await(expr) {
                    let span = self.make_span(expr);
                    let node = self.graph.add_node(CfgNode::Await { span });
                    for &pred in &predecessors {
                        self.graph.add_edge(pred, node, CfgEdge::Sequential);
                    }
                    return vec![node];
                }

                // Generic expression -- treat as a statement node.
                let span = self.make_span(expr);
                let node = self.graph.add_node(CfgNode::Statement { span });
                for &pred in &predecessors {
                    self.graph.add_edge(pred, node, CfgEdge::Sequential);
                }
                vec![node]
            }
        }
    }

    /// Try to process an expression as a control flow construct.
    /// Returns `Some(outgoing_nodes)` if the expression is a control flow
    /// construct, `None` otherwise.
    fn try_process_control_flow_expr(
        &mut self,
        expr: &syn::Expr,
        predecessors: &[NodeIndex],
    ) -> Option<Vec<NodeIndex>> {
        match expr {
            syn::Expr::If(_) | syn::Expr::Match(_) | syn::Expr::Loop(_)
            | syn::Expr::While(_) | syn::Expr::ForLoop(_) | syn::Expr::Return(_)
            | syn::Expr::Await(_) => {
                Some(self.process_expr(expr, predecessors.to_vec()))
            }
            _ => None,
        }
    }

    /// Process an `if` expression, creating Branch, TrueBranch, FalseBranch.
    fn process_if(
        &mut self,
        expr_if: &syn::ExprIf,
        predecessors: Vec<NodeIndex>,
    ) -> Vec<NodeIndex> {
        let span = self.make_span(expr_if);
        let branch_node = self.graph.add_node(CfgNode::Branch { span });
        for &pred in &predecessors {
            self.graph.add_edge(pred, branch_node, CfgEdge::Sequential);
        }

        // True branch (then block)
        let then_entry = self.graph.add_node(CfgNode::Statement {
            span: self.make_span(&expr_if.then_branch),
        });
        self.graph.add_edge(branch_node, then_entry, CfgEdge::TrueBranch);
        let then_exits = self.process_block(&expr_if.then_branch.stmts, vec![then_entry]);

        // False branch (else block, if present)
        let else_exits = if let Some((_, else_expr)) = &expr_if.else_branch {
            let else_node = self.graph.add_node(CfgNode::Statement {
                span: self.make_span(else_expr.as_ref()),
            });
            self.graph.add_edge(branch_node, else_node, CfgEdge::FalseBranch);
            match else_expr.as_ref() {
                syn::Expr::Block(block) => {
                    self.process_block(&block.block.stmts, vec![else_node])
                }
                syn::Expr::If(nested_if) => {
                    self.process_if(nested_if, vec![else_node])
                }
                _ => {
                    // Some other expression as else body
                    vec![else_node]
                }
            }
        } else {
            // No else branch -- the false path goes directly to the join point.
            vec![branch_node]
        };

        // Merge both branches
        let mut exits = then_exits;
        exits.extend(else_exits);
        exits
    }

    /// Process a `match` expression as a branch with true/false edges.
    fn process_match(
        &mut self,
        expr_match: &syn::ExprMatch,
        predecessors: Vec<NodeIndex>,
    ) -> Vec<NodeIndex> {
        let span = self.make_span(expr_match);
        let branch_node = self.graph.add_node(CfgNode::Branch { span });
        for &pred in &predecessors {
            self.graph.add_edge(pred, branch_node, CfgEdge::Sequential);
        }

        let mut all_exits = Vec::new();

        for (i, arm) in expr_match.arms.iter().enumerate() {
            let arm_span = self.make_span(arm);
            let arm_node = self.graph.add_node(CfgNode::Statement { span: arm_span });

            // First arm gets TrueBranch, rest get FalseBranch
            // (simplified model for match)
            let edge = if i == 0 {
                CfgEdge::TrueBranch
            } else {
                CfgEdge::FalseBranch
            };
            self.graph.add_edge(branch_node, arm_node, edge);

            let arm_exits = self.process_expr(&arm.body, vec![arm_node]);
            all_exits.extend(arm_exits);
        }

        if all_exits.is_empty() {
            // Empty match -- flow through the branch node
            vec![branch_node]
        } else {
            all_exits
        }
    }

    /// Process a `loop { ... }` expression.
    fn process_loop(
        &mut self,
        expr_loop: &syn::ExprLoop,
        predecessors: Vec<NodeIndex>,
    ) -> Vec<NodeIndex> {
        let span = self.make_span(expr_loop);
        let loop_node = self.graph.add_node(CfgNode::Loop { span });
        for &pred in &predecessors {
            self.graph.add_edge(pred, loop_node, CfgEdge::Sequential);
        }

        // Process the loop body
        let body_exits = self.process_block(&expr_loop.body.stmts, vec![loop_node]);

        // LoopBack edges from body end back to loop head
        for &exit in &body_exits {
            self.graph.add_edge(exit, loop_node, CfgEdge::LoopBack);
        }

        // `loop` without break is infinite -- only breaks can exit.
        // For Phase 1, add a LoopExit edge from loop_node to represent
        // potential breaks (simplified).
        vec![loop_node]
    }

    /// Process a `while expr { ... }` expression.
    fn process_while(
        &mut self,
        expr_while: &syn::ExprWhile,
        predecessors: Vec<NodeIndex>,
    ) -> Vec<NodeIndex> {
        let span = self.make_span(expr_while);
        let loop_node = self.graph.add_node(CfgNode::Loop { span });
        for &pred in &predecessors {
            self.graph.add_edge(pred, loop_node, CfgEdge::Sequential);
        }

        // Process the loop body
        let body_exits = self.process_block(&expr_while.body.stmts, vec![loop_node]);

        // LoopBack edges from body end back to loop head
        for &exit in &body_exits {
            self.graph.add_edge(exit, loop_node, CfgEdge::LoopBack);
        }

        // While loops exit when the condition is false.
        // The LoopExit edge goes from the loop_node to whatever follows.
        vec![loop_node]
    }

    /// Process a `for pat in expr { ... }` expression.
    fn process_for(
        &mut self,
        expr_for: &syn::ExprForLoop,
        predecessors: Vec<NodeIndex>,
    ) -> Vec<NodeIndex> {
        let span = self.make_span(expr_for);
        let loop_node = self.graph.add_node(CfgNode::Loop { span });
        for &pred in &predecessors {
            self.graph.add_edge(pred, loop_node, CfgEdge::Sequential);
        }

        // Process the loop body
        let body_exits = self.process_block(&expr_for.body.stmts, vec![loop_node]);

        // LoopBack edges from body end back to loop head
        for &exit in &body_exits {
            self.graph.add_edge(exit, loop_node, CfgEdge::LoopBack);
        }

        // For loops exit when the iterator is exhausted.
        vec![loop_node]
    }

    /// Process a `return expr` statement.
    fn process_return(
        &mut self,
        expr_return: &syn::ExprReturn,
        predecessors: Vec<NodeIndex>,
    ) -> Vec<NodeIndex> {
        let span = self.make_span(expr_return);
        let return_node = self.graph.add_node(CfgNode::Return { span });
        for &pred in &predecessors {
            self.graph.add_edge(pred, return_node, CfgEdge::Sequential);
        }
        // Connect return to exit
        self.graph.add_edge(return_node, self.exit, CfgEdge::Sequential);

        // Return terminates this path -- no outgoing nodes for the next statement.
        vec![]
    }

    /// Process an `.await` expression.
    fn process_await(
        &mut self,
        expr_await: &syn::ExprAwait,
        predecessors: Vec<NodeIndex>,
    ) -> Vec<NodeIndex> {
        let span = self.make_span(expr_await);
        let await_node = self.graph.add_node(CfgNode::Await { span });
        for &pred in &predecessors {
            self.graph.add_edge(pred, await_node, CfgEdge::Sequential);
        }
        vec![await_node]
    }

    /// Finalize the CFG by connecting remaining exits to the Exit node.
    fn finalize(mut self, last_nodes: Vec<NodeIndex>) -> DiGraph<CfgNode, CfgEdge> {
        for &node in &last_nodes {
            self.graph.add_edge(node, self.exit, CfgEdge::Sequential);
        }

        // If entry has no outgoing edges (empty function), connect directly to exit.
        if self.graph.neighbors(self.entry).count() == 0 {
            self.graph.add_edge(self.entry, self.exit, CfgEdge::Sequential);
        }

        self.graph
    }
}

// ---------------------------------------------------------------------------
// Await detector
// ---------------------------------------------------------------------------

/// Check if an expression contains a `.await` somewhere within it.
fn contains_await(expr: &syn::Expr) -> bool {
    struct AwaitDetector {
        found: bool,
    }
    impl<'ast> Visit<'ast> for AwaitDetector {
        fn visit_expr_await(&mut self, _node: &'ast syn::ExprAwait) {
            self.found = true;
        }
    }
    let mut detector = AwaitDetector { found: false };
    detector.visit_expr(expr);
    detector.found
}

// ---------------------------------------------------------------------------
// Public API: build CFG for a function body
// ---------------------------------------------------------------------------

/// Build a control flow graph from a function body's block of statements.
///
/// This is the low-level builder used internally. `stmts` should be the
/// statements from the function's body block.
pub fn build_cfg_from_stmts(
    function_id: SymbolId,
    file_id: FileId,
    stmts: &[syn::Stmt],
) -> ControlFlowGraph {
    let mut builder = CfgBuilder::new(file_id);
    let last_nodes = builder.process_block(stmts, vec![builder.entry]);
    let graph = builder.finalize(last_nodes);
    ControlFlowGraph {
        function_id,
        graph,
    }
}

/// Build control flow graphs for all functions/methods found in a Rust source string.
///
/// Parses the source with `syn` and walks all function and method bodies.
/// Returns a map from `SymbolId` to `ControlFlowGraph`.
///
/// The `symbol_lookup` function maps `(name, SymbolKind)` to `SymbolId`,
/// allowing the caller to match discovered functions to their symbol IDs.
///
/// Functions whose symbols cannot be resolved are skipped with a warning.
pub fn build_cfgs_from_source(
    source: &str,
    file_id: FileId,
    symbol_lookup: &HashMap<String, SymbolId>,
) -> HashMap<SymbolId, ControlFlowGraph> {
    let file = match syn::parse_file(source) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %e, "failed to parse source for CFG construction");
            return HashMap::new();
        }
    };

    let mut cfgs = HashMap::new();
    let mut collector = FunctionCollector {
        file_id,
        symbol_lookup,
        cfgs: &mut cfgs,
        current_impl_type: None,
    };
    collector.visit_file(&file);

    cfgs
}

/// Internal visitor that collects function/method bodies and builds CFGs.
struct FunctionCollector<'a> {
    file_id: FileId,
    symbol_lookup: &'a HashMap<String, SymbolId>,
    cfgs: &'a mut HashMap<SymbolId, ControlFlowGraph>,
    current_impl_type: Option<String>,
}

impl<'ast, 'a> Visit<'ast> for FunctionCollector<'a> {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        let name = node.sig.ident.to_string();
        if let Some(&sym_id) = self.symbol_lookup.get(&name) {
            let cfg = build_cfg_from_stmts(sym_id, self.file_id, &node.block.stmts);
            self.cfgs.insert(sym_id, cfg);
        } else {
            tracing::debug!(
                name = %name,
                "skipping CFG for function with no symbol mapping"
            );
        }
        // Continue visiting nested items (but not recursing into nested fn bodies
        // for CFG purposes -- they get their own CFGs).
        syn::visit::visit_item_fn(self, node);
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        let prev = self.current_impl_type.take();
        self.current_impl_type = Some(
            node.self_ty
                .as_ref()
                .to_token_stream_string(),
        );
        syn::visit::visit_item_impl(self, node);
        self.current_impl_type = prev;
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        let method_name = node.sig.ident.to_string();

        // Try qualified name first (Type::method), then just method name.
        let sym_id = self
            .current_impl_type
            .as_ref()
            .and_then(|ty| {
                let qualified = format!("{}::{}", ty, method_name);
                self.symbol_lookup.get(&qualified).copied()
            })
            .or_else(|| self.symbol_lookup.get(&method_name).copied());

        if let Some(sym_id) = sym_id {
            let cfg = build_cfg_from_stmts(sym_id, self.file_id, &node.block.stmts);
            self.cfgs.insert(sym_id, cfg);
        } else {
            tracing::debug!(
                name = %method_name,
                "skipping CFG for method with no symbol mapping"
            );
        }

        syn::visit::visit_impl_item_fn(self, node);
    }
}

/// Helper trait to convert a syn type to a string.
trait ToTokenStreamString {
    fn to_token_stream_string(&self) -> String;
}

impl<T: quote::ToTokens> ToTokenStreamString for T {
    fn to_token_stream_string(&self) -> String {
        self.to_token_stream().to_string()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use spectron_core::{FileId, SymbolId};

    /// Helper: parse a function body source and build a CFG.
    fn build_test_cfg(body_source: &str) -> DiGraph<CfgNode, CfgEdge> {
        let full_source = format!("fn test_fn() {{ {} }}", body_source);
        let file = syn::parse_file(&full_source).expect("test source should parse");

        // Extract the function body
        let item = &file.items[0];
        let stmts = match item {
            syn::Item::Fn(item_fn) => &item_fn.block.stmts,
            _ => panic!("expected a function item"),
        };

        let mut builder = CfgBuilder::new(FileId(0));
        let entry = builder.entry;
        let last = builder.process_block(stmts, vec![entry]);
        builder.finalize(last)
    }

    /// Count nodes of a specific type in the graph.
    fn count_nodes(graph: &DiGraph<CfgNode, CfgEdge>, pred: impl Fn(&CfgNode) -> bool) -> usize {
        graph.node_indices().filter(|&i| pred(&graph[i])).count()
    }

    /// Count edges of a specific type in the graph.
    fn count_edges(graph: &DiGraph<CfgNode, CfgEdge>, pred: impl Fn(&CfgEdge) -> bool) -> usize {
        graph.edge_indices().filter(|&i| pred(&graph[i])).count()
    }

    /// Check if an edge exists between two nodes with a specific type.
    fn has_edge(
        graph: &DiGraph<CfgNode, CfgEdge>,
        from: NodeIndex,
        to: NodeIndex,
        edge: &CfgEdge,
    ) -> bool {
        graph
            .edges_connecting(from, to)
            .any(|e| e.weight() == edge)
    }

    // -------------------------------------------------------------------
    // Test: Linear function with 3 statements
    // -------------------------------------------------------------------

    #[test]
    fn linear_function_three_statements() {
        // 3 sequential statements
        let graph = build_test_cfg(
            r#"
                let a = 1;
                let b = 2;
                let c = 3;
            "#,
        );

        // Expected: Entry, 3 Statement, Exit = 5 nodes total
        assert_eq!(graph.node_count(), 5, "expected 5 nodes (Entry + 3 Stmt + Exit)");

        // Expected: Entry->S1, S1->S2, S2->S3, S3->Exit = 4 Sequential edges
        assert_eq!(
            count_edges(&graph, |e| *e == CfgEdge::Sequential),
            4,
            "expected 4 sequential edges"
        );

        // Verify Entry and Exit exist
        assert_eq!(count_nodes(&graph, |n| *n == CfgNode::Entry), 1);
        assert_eq!(count_nodes(&graph, |n| *n == CfgNode::Exit), 1);

        // Verify 3 Statement nodes
        assert_eq!(
            count_nodes(&graph, |n| matches!(n, CfgNode::Statement { .. })),
            3,
            "expected 3 statement nodes"
        );
    }

    // -------------------------------------------------------------------
    // Test: Function with if/else
    // -------------------------------------------------------------------

    #[test]
    fn function_with_if_else() {
        let graph = build_test_cfg(
            r#"
                if true {
                    let a = 1;
                } else {
                    let b = 2;
                }
            "#,
        );

        // Should have a Branch node
        assert!(
            count_nodes(&graph, |n| matches!(n, CfgNode::Branch { .. })) >= 1,
            "expected at least 1 branch node"
        );

        // Should have TrueBranch and FalseBranch edges
        assert!(
            count_edges(&graph, |e| *e == CfgEdge::TrueBranch) >= 1,
            "expected at least 1 TrueBranch edge"
        );
        assert!(
            count_edges(&graph, |e| *e == CfgEdge::FalseBranch) >= 1,
            "expected at least 1 FalseBranch edge"
        );

        // Entry and Exit should exist
        assert_eq!(count_nodes(&graph, |n| *n == CfgNode::Entry), 1);
        assert_eq!(count_nodes(&graph, |n| *n == CfgNode::Exit), 1);
    }

    // -------------------------------------------------------------------
    // Test: Function with loop
    // -------------------------------------------------------------------

    #[test]
    fn function_with_loop() {
        let graph = build_test_cfg(
            r#"
                loop {
                    let x = 1;
                }
            "#,
        );

        // Should have a Loop node
        assert!(
            count_nodes(&graph, |n| matches!(n, CfgNode::Loop { .. })) >= 1,
            "expected at least 1 Loop node"
        );

        // Should have a LoopBack edge
        assert!(
            count_edges(&graph, |e| *e == CfgEdge::LoopBack) >= 1,
            "expected at least 1 LoopBack edge"
        );

        // Entry and Exit should exist
        assert_eq!(count_nodes(&graph, |n| *n == CfgNode::Entry), 1);
        assert_eq!(count_nodes(&graph, |n| *n == CfgNode::Exit), 1);
    }

    // -------------------------------------------------------------------
    // Test: Function with while loop
    // -------------------------------------------------------------------

    #[test]
    fn function_with_while_loop() {
        let graph = build_test_cfg(
            r#"
                while true {
                    let x = 1;
                }
            "#,
        );

        assert!(
            count_nodes(&graph, |n| matches!(n, CfgNode::Loop { .. })) >= 1,
            "expected at least 1 Loop node for while"
        );

        assert!(
            count_edges(&graph, |e| *e == CfgEdge::LoopBack) >= 1,
            "expected at least 1 LoopBack edge for while"
        );
    }

    // -------------------------------------------------------------------
    // Test: Function with for loop
    // -------------------------------------------------------------------

    #[test]
    fn function_with_for_loop() {
        let graph = build_test_cfg(
            r#"
                for i in 0..10 {
                    let x = i;
                }
            "#,
        );

        assert!(
            count_nodes(&graph, |n| matches!(n, CfgNode::Loop { .. })) >= 1,
            "expected at least 1 Loop node for for-loop"
        );

        assert!(
            count_edges(&graph, |e| *e == CfgEdge::LoopBack) >= 1,
            "expected at least 1 LoopBack edge for for-loop"
        );
    }

    // -------------------------------------------------------------------
    // Test: Function with return
    // -------------------------------------------------------------------

    #[test]
    fn function_with_return() {
        let graph = build_test_cfg(
            r#"
                let a = 1;
                return;
            "#,
        );

        // Should have a Return node
        assert!(
            count_nodes(&graph, |n| matches!(n, CfgNode::Return { .. })) >= 1,
            "expected at least 1 Return node"
        );

        // Return should connect to Exit
        let return_idx = graph
            .node_indices()
            .find(|&i| matches!(&graph[i], CfgNode::Return { .. }))
            .expect("should have a Return node");
        let exit_idx = graph
            .node_indices()
            .find(|&i| graph[i] == CfgNode::Exit)
            .expect("should have Exit node");

        assert!(
            has_edge(&graph, return_idx, exit_idx, &CfgEdge::Sequential),
            "Return should have Sequential edge to Exit"
        );
    }

    // -------------------------------------------------------------------
    // Test: Empty function
    // -------------------------------------------------------------------

    #[test]
    fn empty_function() {
        let graph = build_test_cfg("");

        // Just Entry and Exit
        assert_eq!(graph.node_count(), 2, "expected 2 nodes (Entry + Exit)");

        // Entry -> Exit
        assert_eq!(
            count_edges(&graph, |e| *e == CfgEdge::Sequential),
            1,
            "expected 1 sequential edge (Entry -> Exit)"
        );
    }

    // -------------------------------------------------------------------
    // Test: Function with match
    // -------------------------------------------------------------------

    #[test]
    fn function_with_match() {
        let graph = build_test_cfg(
            r#"
                match x {
                    1 => { let a = 1; },
                    _ => { let b = 2; },
                }
            "#,
        );

        // Should have a Branch node for the match
        assert!(
            count_nodes(&graph, |n| matches!(n, CfgNode::Branch { .. })) >= 1,
            "expected at least 1 Branch node for match"
        );

        // Should have TrueBranch (first arm) and FalseBranch (other arms)
        assert!(
            count_edges(&graph, |e| *e == CfgEdge::TrueBranch) >= 1,
            "expected at least 1 TrueBranch edge"
        );
        assert!(
            count_edges(&graph, |e| *e == CfgEdge::FalseBranch) >= 1,
            "expected at least 1 FalseBranch edge"
        );
    }

    // -------------------------------------------------------------------
    // Test: Function with return in middle (unreachable code)
    // -------------------------------------------------------------------

    #[test]
    fn return_terminates_flow() {
        let graph = build_test_cfg(
            r#"
                let a = 1;
                return;
                let b = 2;
            "#,
        );

        // The `let b = 2;` should be unreachable.
        // We should have: Entry, Statement(a), Return, Exit = 4 nodes
        // (the unreachable statement is skipped)
        assert_eq!(
            count_nodes(&graph, |n| matches!(n, CfgNode::Statement { .. })),
            1,
            "expected 1 statement node (unreachable code skipped)"
        );

        assert_eq!(
            count_nodes(&graph, |n| matches!(n, CfgNode::Return { .. })),
            1,
            "expected 1 return node"
        );
    }

    // -------------------------------------------------------------------
    // Test: build_cfg_from_stmts API
    // -------------------------------------------------------------------

    #[test]
    fn build_cfg_from_stmts_api() {
        let source = "fn test_fn() { let a = 1; let b = 2; }";
        let file = syn::parse_file(source).expect("should parse");
        let stmts = match &file.items[0] {
            syn::Item::Fn(item_fn) => &item_fn.block.stmts,
            _ => panic!("expected fn"),
        };

        let cfg = build_cfg_from_stmts(SymbolId(42), FileId(0), stmts);
        assert_eq!(cfg.function_id, SymbolId(42));
        // Entry + 2 statements + Exit = 4 nodes
        assert_eq!(cfg.graph.node_count(), 4);
    }

    // -------------------------------------------------------------------
    // Test: build_cfgs_from_source API
    // -------------------------------------------------------------------

    #[test]
    fn build_cfgs_from_source_api() {
        let source = r#"
            fn foo() {
                let a = 1;
                let b = 2;
            }

            fn bar() {
                let x = 10;
            }
        "#;

        let mut lookup = HashMap::new();
        lookup.insert("foo".to_string(), SymbolId(1));
        lookup.insert("bar".to_string(), SymbolId(2));

        let cfgs = build_cfgs_from_source(source, FileId(0), &lookup);

        assert_eq!(cfgs.len(), 2, "expected 2 CFGs");
        assert!(cfgs.contains_key(&SymbolId(1)), "should have CFG for foo");
        assert!(cfgs.contains_key(&SymbolId(2)), "should have CFG for bar");

        // foo: Entry + 2 stmts + Exit = 4 nodes
        assert_eq!(cfgs[&SymbolId(1)].graph.node_count(), 4);
        // bar: Entry + 1 stmt + Exit = 3 nodes
        assert_eq!(cfgs[&SymbolId(2)].graph.node_count(), 3);
    }

    // -------------------------------------------------------------------
    // Test: if without else
    // -------------------------------------------------------------------

    #[test]
    fn if_without_else() {
        let graph = build_test_cfg(
            r#"
                if true {
                    let a = 1;
                }
            "#,
        );

        // Branch should exist
        assert!(
            count_nodes(&graph, |n| matches!(n, CfgNode::Branch { .. })) >= 1,
            "expected Branch node"
        );

        // TrueBranch should exist
        assert!(
            count_edges(&graph, |e| *e == CfgEdge::TrueBranch) >= 1,
            "expected TrueBranch edge"
        );

        // Entry and Exit should exist
        assert_eq!(count_nodes(&graph, |n| *n == CfgNode::Entry), 1);
        assert_eq!(count_nodes(&graph, |n| *n == CfgNode::Exit), 1);
    }

    // -------------------------------------------------------------------
    // Test: nested if
    // -------------------------------------------------------------------

    #[test]
    fn nested_if_else() {
        let graph = build_test_cfg(
            r#"
                if true {
                    if false {
                        let a = 1;
                    } else {
                        let b = 2;
                    }
                } else {
                    let c = 3;
                }
            "#,
        );

        // Should have at least 2 Branch nodes
        assert!(
            count_nodes(&graph, |n| matches!(n, CfgNode::Branch { .. })) >= 2,
            "expected at least 2 Branch nodes for nested if"
        );
    }

    // -------------------------------------------------------------------
    // Test: complex function with mixed control flow
    // -------------------------------------------------------------------

    #[test]
    fn mixed_control_flow() {
        let graph = build_test_cfg(
            r#"
                let a = 1;
                if a > 0 {
                    let b = 2;
                }
                for i in 0..10 {
                    let c = i;
                }
                return;
            "#,
        );

        // Should have at least 1 of each: Statement, Branch, Loop, Return
        assert!(
            count_nodes(&graph, |n| matches!(n, CfgNode::Statement { .. })) >= 1,
            "expected Statement nodes"
        );
        assert!(
            count_nodes(&graph, |n| matches!(n, CfgNode::Branch { .. })) >= 1,
            "expected Branch node"
        );
        assert!(
            count_nodes(&graph, |n| matches!(n, CfgNode::Loop { .. })) >= 1,
            "expected Loop node"
        );
        assert!(
            count_nodes(&graph, |n| matches!(n, CfgNode::Return { .. })) >= 1,
            "expected Return node"
        );
    }
}
