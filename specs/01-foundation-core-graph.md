# 01 -- Foundation: Core Graph & Pipeline

## PRD Reference

Phase 1 calls for a foundation layer comprising:

- Parser integration (Rust)
- Symbol index
- Module graph
- Call graph
- CFG (basic)
- Findings system (covered separately in spec 02)
- CLI interface

The PRD's core data model specifies:

**Nodes**: Files, Modules, Packages, Functions/Methods, Types/Traits/Interfaces,
Variables/Fields, External resources (DB, network, FS).

**Edges**: Calls, Imports/Dependencies, Data flow, Control flow, Ownership/lifetime,
Taint flow, Lock relationships.

---

## Current Implementation

### 1. Identity System (`spectron-core/src/id.rs`)

Four strongly-typed ID newtypes sharing a single atomic counter:

```rust
CrateId(u64), ModuleId(u64), SymbolId(u64), FileId(u64)
```

`IdGenerator` provides thread-safe monotonic allocation across all ID types.

**Status**: Complete and well-tested (concurrent generation, type safety, serde roundtrip).

### 2. Symbol Model (`spectron-core/src/symbol.rs`)

```rust
pub struct Symbol {
    pub id: SymbolId,
    pub name: String,
    pub kind: SymbolKind,       // Function, Method, Struct, Enum, Trait, ImplBlock, Constant, Static, TypeAlias
    pub module_id: ModuleId,
    pub file_id: FileId,
    pub span: SourceSpan,
    pub visibility: Visibility, // Public, Crate, Restricted, Private
    pub signature: Option<String>,
    pub attributes: SymbolAttributes,
}
```

`SymbolAttributes` tracks: `is_async`, `is_unsafe`, `is_extern`, `is_test`,
`has_unsafe_block`, `doc_comment`, `attribute_paths`.

**Status**: Complete. Covers all Rust declaration types relevant to analysis.

### 3. Project Hierarchy (`spectron-core/src/project.rs`)

- `ProjectInfo` -- root project metadata (name, path, is_workspace)
- `CrateInfo` -- per-crate (name, path, type, module_ids, dependencies)
- `ModuleInfo` -- per-module (name, path, file_path, parent, children, symbol_ids)
- `ModulePath` -- qualified path like `"my_crate::foo::bar"`

**Status**: Complete.

### 4. Graph Model (`spectron-core/src/graph.rs`)

```rust
pub type ArchGraph = DiGraph<GraphNode, GraphEdge>;

pub enum GraphNode {
    Crate(CrateId),
    Module(ModuleId),
    File(FileId),
    Symbol(SymbolId),
}

pub struct GraphEdge {
    pub kind: RelationshipKind,
    pub weight: f32,
}

pub enum RelationshipKind {
    Contains, Imports, Calls, References, Implements, DependsOn,
}
```

A flat `Relationship` struct (source `SymbolId` -> target `SymbolId` + kind + optional span)
exists for pre-graph-building data.

**Status**: Complete for current needs.

### 5. Graph Builder (`spectron-graph/src/builder.rs`)

`build_graphs(load_result, parse_result) -> GraphSet` produces:

- **Structure graph**: All crates, modules, files, symbols as nodes.
  Edges: Contains, Imports, Implements, References, DependsOn.
- **Call graph**: Functions/methods only, Calls edges only.
- **CallGraphData**: callers/callees HashMaps for O(1) lookup.
- **GraphIndex**: domain ID -> NodeIndex maps for O(1) graph access.
- **Per-function CFGs**: HashMap<SymbolId, ControlFlowGraph>.

Orphan symbols are assigned to a synthetic `UNRESOLVED_MODULE_ID = ModuleId(u64::MAX)`.

**Status**: Complete with extensive tests.

### 6. Control Flow Graphs (`spectron-graph/src/cfg.rs`)

```rust
pub enum CfgNode { Entry, Exit, Statement, Branch, Loop, Await, Return }
pub enum CfgEdge { Sequential, TrueBranch, FalseBranch, LoopBack, LoopExit }
```

CFGs are built per-function from `syn` AST by walking statements. Handles
if/else, match, loop/while/for, return, await.

**Status**: Complete for Phase 1 scope.

### 7. Graph Algorithms (`spectron-graph/src/algorithms.rs`)

- `find_paths(graph, from, to, max_depth)` -- all simple paths via DFS
- `descendants(graph, node)` -- BFS reachability
- `reachable_subgraph(graph, roots)` -- induced subgraph from reachable nodes
- `components(graph)` -- connected components (undirected projection)
- `DataFlowInfo` -- caller/callee + argument count + return usage

**Status**: Complete.

### 8. Loader (`spectron-loader/`)

- `load_project(path) -> LoadResult` -- workspace + single crate support
- Discovers Cargo manifests, crate targets, modules (via line scanning), files
- SHA-256 file hashing, line counting
- Skips build.rs, tests/, examples/, benches/
- Handles dual-target crates (lib+bin) with module deduplication

**Status**: Complete with extensive integration tests and malformed-input fixtures.

### 9. Parser (`spectron-parser/`)

- `parse_project(load_result) -> ParseResult` -- parallel file parsing via rayon
- `SymbolVisitor` -- syn::visit::Visit implementation extracting all symbol types
- `SymbolTable` -- (ModuleId, name) -> SymbolId resolution
- Resolves imports and relationships (name -> SymbolId)
- Extracts: functions, structs, enums, traits, impl blocks, methods, constants, statics, type aliases
- Extracts relationships: imports, calls, references, implements

**Status**: Complete with integration tests.

### 10. CLI (`spectron-app/src/main.rs`)

- `spectron <path>` -- full pipeline -> GUI
- `spectron <path> --json` -- JSON output (project, crates, modules, files)
- `spectron <path> --cli` -- text tree to terminal

**Status**: Complete.

---

## Gap Analysis

### Missing Node Types (PRD vs Current)

| PRD Node Type | Current Status | Action |
|---|---|---|
| Files | `GraphNode::File(FileId)` | Done |
| Modules / Packages | `GraphNode::Module(ModuleId)` + `GraphNode::Crate(CrateId)` | Done |
| Functions / Methods | `GraphNode::Symbol(SymbolId)` where kind is Function/Method | Done |
| Types / Traits / Interfaces | `GraphNode::Symbol(SymbolId)` where kind is Struct/Enum/Trait | Done |
| Variables / Fields | **Not extracted** | Phase 3+ (needed for data flow) |
| External resources (DB, network, FS) | Detected via `SecurityIndicator` but not as graph nodes | Phase 2 (needed for taint sinks) |

### Missing Edge Types (PRD vs Current)

| PRD Edge Type | Current Status | Action |
|---|---|---|
| Calls | `RelationshipKind::Calls` | Done |
| Imports / Dependencies | `Imports` + `DependsOn` | Done |
| Data flow | **Not implemented** | Phase 3 (interprocedural summaries) |
| Control flow | CFG exists per-function, not as ArchGraph edges | Keep separate; CFG is inherently intra-procedural |
| Ownership / lifetime | **Not implemented** | Phase 4+ (resource analyzer) |
| Taint flow | **Not implemented** | Phase 2 (taint engine) |
| Lock relationships | **Not implemented** | Phase 4+ (resource analyzer) |

### Other Gaps

1. **No Variable/Field-level symbols**: Current parser extracts declaration-level symbols only. Variable tracking requires expression-level analysis in the visitor.
2. **Unresolved symbol tracking**: `SymbolId(u64::MAX)` is used as a sentinel for unresolved names. This works but is not surfaced as a diagnostic.
3. **JSON output is loader-only**: `--json` mode outputs LoadResult, not the full analysis pipeline.

---

## Design Decisions

1. **Keep CFG separate from ArchGraph**: Control flow is inherently intra-procedural. Mixing CFG edges into the global ArchGraph would conflate granularity levels. CFGs remain in `GraphSet.control_flow_graphs`.

2. **Add new RelationshipKind variants incrementally**: Rather than adding all PRD edge types upfront, add them when the consuming analysis module is built:
   - `DataFlow` when interprocedural summaries land (Phase 3)
   - `TaintFlow` when the taint engine lands (Phase 2)
   - Ownership/lock edges when the resource analyzer lands (Phase 4)

3. **Variables/Fields are Phase 3+**: Extracting variable-level symbols requires tracking local bindings in function bodies. This is a significant parser extension best deferred until data-flow analysis needs it.

4. **External resource nodes as virtual graph nodes**: Rather than creating real file/module/symbol entries for DB/network/FS, model them as tagged `GraphNode::ExternalResource` variants. This avoids polluting the symbol table.

5. **Extend JSON output to include full analysis**: The `--json` mode should serialize the complete `AnalysisOutput` (metrics, findings, security, entrypoints), not just the loader result.

---

## Tasks

- [ ] **T-01.1** Add `GraphNode::ExternalResource(String)` variant for modeling sinks (DB, network, FS) as graph nodes
- [ ] **T-01.2** Add `RelationshipKind::TaintFlow` variant (needed by spec 03)
- [ ] **T-01.3** Add `RelationshipKind::DataFlow` variant (needed by spec 08)
- [ ] **T-01.4** Extend `--json` CLI mode to serialize full `AnalysisOutput` (metrics, security, entrypoints, flags)
- [ ] **T-01.5** Surface unresolved symbol count as a diagnostic (log warning + include in analysis output)
- [ ] **T-01.6** Add cycle detection to `spectron-graph/src/algorithms.rs` using Tarjan's SCC algorithm (needed by spec 06)
- [ ] **T-01.7** Add topological sort to `spectron-graph/src/algorithms.rs` (needed by specs 05, 08 for bottom-up propagation)
- [ ] **T-01.8** Ensure `GraphSet` and `AnalysisOutput` implement `Serialize` for storage/caching support
