//! Inspector panel: detailed view for a selected graph node.

use egui::{Color32, RichText, Ui};
use petgraph::graph::{EdgeIndex, NodeIndex};
use spectron_core::{
    CrateId, CrateType, GraphNode, ModuleId, RelationshipKind, SymbolId, SymbolKind, Visibility,
};

use crate::ProjectData;

const DIM: Color32 = Color32::from_rgb(150, 150, 150);
const BLUE: Color32 = Color32::from_rgb(77, 84, 245); // #4D54F5
const RED: Color32 = Color32::from_rgb(254, 75, 66); // #FE4B42
const GREEN: Color32 = Color32::from_rgb(82, 242, 132); // #52F284
const PURPLE: Color32 = Color32::from_rgb(182, 83, 249); // #B653F9
const YELLOW_GREEN: Color32 = Color32::from_rgb(171, 240, 18); // #ABF012

// ---------------------------------------------------------------------------
// Inspector target
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub enum InspectorTarget {
    Symbol(SymbolId),
    Module(ModuleId),
    Crate(CrateId),
    Edge(EdgeIndex),
}

impl InspectorTarget {
    pub fn from_graph_node(node: &GraphNode) -> Option<Self> {
        match node {
            GraphNode::Symbol(id) => Some(Self::Symbol(*id)),
            GraphNode::Module(id) => Some(Self::Module(*id)),
            GraphNode::Crate(id) => Some(Self::Crate(*id)),
            GraphNode::File(_) => None,
        }
    }
}

/// Result of inspector actions.
pub struct InspectorAction {
    pub navigate_to: Option<SymbolId>,
    pub focus_node: Option<NodeIndex>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn show_inspector(
    ui: &mut Ui,
    target: &InspectorTarget,
    data: &ProjectData,
    callers_clicked: &mut Option<SymbolId>,
) {
    match target {
        InspectorTarget::Symbol(id) => show_symbol(ui, *id, data, callers_clicked),
        InspectorTarget::Module(id) => show_module(ui, *id, data),
        InspectorTarget::Crate(id) => show_crate(ui, *id, data),
        InspectorTarget::Edge(eidx) => show_edge(ui, *eidx, data),
    }
}

/// Show inspector with focus action support.
pub fn show_inspector_with_actions(
    ui: &mut Ui,
    target: &InspectorTarget,
    data: &ProjectData,
    callers_clicked: &mut Option<SymbolId>,
) -> Option<NodeIndex> {
    let mut focus_request = None;

    match target {
        InspectorTarget::Symbol(id) => {
            show_symbol(ui, *id, data, callers_clicked);
            if let Some(&ni) = data.graph_set.index.symbol_nodes.get(id) {
                ui.add_space(4.0);
                if ui.small_button("Focus on this node").clicked() {
                    focus_request = Some(ni);
                }
            }
        }
        InspectorTarget::Module(id) => {
            show_module(ui, *id, data);
            if let Some(&ni) = data.graph_set.index.module_nodes.get(id) {
                ui.add_space(4.0);
                if ui.small_button("Focus on this node").clicked() {
                    focus_request = Some(ni);
                }
            }
        }
        InspectorTarget::Crate(id) => {
            show_crate(ui, *id, data);
            if let Some(&ni) = data.graph_set.index.crate_nodes.get(id) {
                ui.add_space(4.0);
                if ui.small_button("Focus on this node").clicked() {
                    focus_request = Some(ni);
                }
            }
        }
        InspectorTarget::Edge(eidx) => {
            show_edge(ui, *eidx, data);
        }
    }

    focus_request
}

// ---------------------------------------------------------------------------
// Symbol inspector
// ---------------------------------------------------------------------------

fn show_symbol(
    ui: &mut Ui,
    sid: SymbolId,
    data: &ProjectData,
    callers_clicked: &mut Option<SymbolId>,
) {
    let Some(sym) = data.symbols.get(&sid) else {
        ui.label("Symbol not found.");
        return;
    };

    let color = symbol_kind_color(&sym.kind);
    ui.label(RichText::new(&sym.name).heading().strong().color(color));
    ui.label(
        RichText::new(format!("{}", symbol_kind_label(&sym.kind)))
            .color(color)
            .small(),
    );
    ui.add_space(4.0);

    if let Some(ref sig) = sym.signature {
        ui.label(RichText::new(sig).monospace().small());
        ui.add_space(4.0);
    }

    ui.separator();
    ui.add_space(4.0);

    egui::Grid::new("sym_info")
        .num_columns(2)
        .spacing([24.0, 4.0])
        .show(ui, |ui| {
            row(ui, "Visibility", visibility_label(&sym.visibility));

            if let Some(m) = data.modules.get(&sym.module_id) {
                row(ui, "Module", m.path.as_str());
                if let Some(ref fp) = m.file_path {
                    row(ui, "File", &fp.display().to_string());
                }
            }

            row(
                ui,
                "Span",
                &format!("{}:{} \u{2013} {}:{}", sym.span.start_line, sym.span.start_col, sym.span.end_line, sym.span.end_col),
            );
        });

    // Metrics
    if let Some(m) = data.analysis.symbol_metrics.get(&sid) {
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        ui.label(RichText::new("Metrics").strong());
        ui.add_space(2.0);
        egui::Grid::new("sym_metrics")
            .num_columns(2)
            .spacing([24.0, 4.0])
            .show(ui, |ui| {
                row(ui, "Complexity", &m.cyclomatic_complexity.to_string());
                row(ui, "Lines", &m.line_count.to_string());
                row(ui, "Parameters", &m.parameter_count.to_string());
                row(ui, "Fan-in", &m.fan_in.to_string());
                row(ui, "Fan-out", &m.fan_out.to_string());
            });
    }

    // Complexity flags
    let flags: Vec<_> = data
        .analysis
        .complexity_flags
        .iter()
        .filter(|f| {
            matches!(&f.target, spectron_analysis::FlagTarget::Symbol(s) if *s == sid)
        })
        .collect();
    if !flags.is_empty() {
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        ui.label(RichText::new("Flags").strong().color(RED));
        for flag in &flags {
            ui.label(
                RichText::new(format!("{:?}: {} (threshold {})", flag.kind, flag.value, flag.threshold))
                    .small()
                    .color(RED),
            );
        }
    }

    // Security attributes
    let attrs = &sym.attributes;
    if attrs.is_unsafe || attrs.is_extern || attrs.has_unsafe_block {
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        ui.label(RichText::new("Security").strong().color(RED));
        if attrs.is_unsafe {
            ui.label(RichText::new("unsafe fn").color(RED));
        }
        if attrs.is_extern {
            ui.label(RichText::new("extern").color(RED));
        }
        if attrs.has_unsafe_block {
            ui.label(RichText::new("contains unsafe block").color(RED));
        }
    }

    // Callers / Callees
    if let Some(callees) = data.graph_set.call_graph_data.callees.get(&sid) {
        if !callees.is_empty() {
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);
            ui.label(RichText::new("Callees").strong());
            for &cid in callees {
                if let Some(s) = data.symbols.get(&cid) {
                    let c = symbol_kind_color(&s.kind);
                    if ui.link(RichText::new(&s.name).color(c)).clicked() {
                        *callers_clicked = Some(cid);
                    }
                }
            }
        }
    }

    if let Some(callers) = data.graph_set.call_graph_data.callers.get(&sid) {
        if !callers.is_empty() {
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);
            ui.label(RichText::new("Callers").strong());
            for &cid in callers {
                if let Some(s) = data.symbols.get(&cid) {
                    let c = symbol_kind_color(&s.kind);
                    if ui.link(RichText::new(&s.name).color(c)).clicked() {
                        *callers_clicked = Some(cid);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Module inspector
// ---------------------------------------------------------------------------

fn show_module(ui: &mut Ui, mid: ModuleId, data: &ProjectData) {
    let Some(m) = data.modules.get(&mid) else {
        ui.label("Module not found.");
        return;
    };

    ui.label(RichText::new(&m.name).heading().strong().color(GREEN));
    ui.label(RichText::new(m.path.as_str()).monospace().small());
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);

    egui::Grid::new("mod_info")
        .num_columns(2)
        .spacing([24.0, 4.0])
        .show(ui, |ui| {
            if let Some(ref fp) = m.file_path {
                row(ui, "File", &fp.display().to_string());
            }
            if let Some(pid) = m.parent {
                let pname = data.modules.get(&pid).map_or("?", |p| &p.name);
                row(ui, "Parent", pname);
            }
            row(ui, "Children", &m.children.len().to_string());
            row(ui, "Symbols", &m.symbol_ids.len().to_string());
        });

    if let Some(mm) = data.analysis.module_metrics.get(&mid) {
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        ui.label(RichText::new("Metrics").strong());
        egui::Grid::new("mod_metrics")
            .num_columns(2)
            .spacing([24.0, 4.0])
            .show(ui, |ui| {
                row(ui, "Symbol count", &mm.symbol_count.to_string());
                row(ui, "Line count", &mm.line_count.to_string());
                row(ui, "Fan-in", &mm.fan_in.to_string());
                row(ui, "Fan-out", &mm.fan_out.to_string());
            });
    }
}

// ---------------------------------------------------------------------------
// Crate inspector
// ---------------------------------------------------------------------------

fn show_crate(ui: &mut Ui, cid: CrateId, data: &ProjectData) {
    let Some(c) = data.crate_index.get(&cid).and_then(|&i| data.crates.get(i)) else {
        ui.label("Crate not found.");
        return;
    };

    ui.label(RichText::new(&c.name).heading().strong().color(BLUE));
    let type_label = match c.crate_type {
        CrateType::Library => "Library",
        CrateType::Binary => "Binary",
    };
    ui.label(RichText::new(type_label).color(BLUE).small());
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);

    egui::Grid::new("crate_info")
        .num_columns(2)
        .spacing([24.0, 4.0])
        .show(ui, |ui| {
            row(ui, "Path", &c.path.display().to_string());
            row(ui, "Modules", &c.module_ids.len().to_string());
            row(ui, "Dependencies", &c.dependencies.len().to_string());
        });

    if !c.dependencies.is_empty() {
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        ui.label(RichText::new("Dependencies").strong());
        for dep in &c.dependencies {
            ui.label(RichText::new(dep).monospace().small());
        }
    }
}

// ---------------------------------------------------------------------------
// Edge inspector
// ---------------------------------------------------------------------------

fn show_edge(ui: &mut Ui, eidx: EdgeIndex, data: &ProjectData) {
    let graph = &data.graph_set.structure_graph;
    let Some(edge) = graph.edge_weight(eidx) else {
        ui.label("Edge not found.");
        return;
    };
    let Some((src, tgt)) = graph.edge_endpoints(eidx) else {
        ui.label("Edge endpoints not found.");
        return;
    };

    let kind_label = format!("{}", edge.kind);
    ui.label(RichText::new(&kind_label).heading().strong());
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);

    egui::Grid::new("edge_info")
        .num_columns(2)
        .spacing([24.0, 4.0])
        .show(ui, |ui| {
            row(ui, "Kind", &kind_label);
            row(ui, "Weight", &format!("{:.2}", edge.weight));
            row(ui, "Source", &format!("{}", graph[src]));
            row(ui, "Target", &format!("{}", graph[tgt]));
        });

    // For Calls edges: list call site info if available
    if edge.kind == RelationshipKind::Calls {
        if let (GraphNode::Symbol(src_sid), GraphNode::Symbol(tgt_sid)) =
            (&graph[src], &graph[tgt])
        {
            if let Some(callees) = data.graph_set.call_graph_data.callees.get(src_sid) {
                if callees.contains(tgt_sid) {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(4.0);
                    let src_name = data.symbols.get(src_sid).map_or("?", |s| &s.name);
                    let tgt_name = data.symbols.get(tgt_sid).map_or("?", |s| &s.name);
                    ui.label(
                        RichText::new(format!("{} calls {}", src_name, tgt_name))
                            .small(),
                    );
                }
            }
        }
    }

    // Count parallel edges between same endpoints
    let parallel = graph
        .edges_connecting(src, tgt)
        .chain(graph.edges_connecting(tgt, src))
        .count();
    if parallel > 1 {
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!("{} edges between these nodes", parallel))
                .small()
                .color(DIM),
        );
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn row(ui: &mut Ui, label: &str, value: &str) {
    ui.label(RichText::new(label).color(DIM).small());
    ui.label(RichText::new(value).small());
    ui.end_row();
}

fn visibility_label(v: &Visibility) -> &'static str {
    match v {
        Visibility::Public => "pub",
        Visibility::Crate => "pub(crate)",
        Visibility::Restricted => "pub(restricted)",
        Visibility::Private => "private",
    }
}

fn symbol_kind_label(k: &SymbolKind) -> &'static str {
    match k {
        SymbolKind::Function => "Function",
        SymbolKind::Method => "Method",
        SymbolKind::Struct => "Struct",
        SymbolKind::Enum => "Enum",
        SymbolKind::Trait => "Trait",
        SymbolKind::ImplBlock => "Impl Block",
        SymbolKind::Constant => "Constant",
        SymbolKind::Static => "Static",
        SymbolKind::TypeAlias => "Type Alias",
    }
}

pub fn symbol_kind_color(k: &SymbolKind) -> Color32 {
    match k {
        SymbolKind::Function | SymbolKind::Method => RED,
        SymbolKind::Struct | SymbolKind::Enum => PURPLE,
        SymbolKind::Trait => YELLOW_GREEN,
        SymbolKind::ImplBlock => DIM,
        _ => DIM,
    }
}

pub fn symbol_kind_prefix(k: &SymbolKind) -> &'static str {
    match k {
        SymbolKind::Function => "fn",
        SymbolKind::Method => "fn",
        SymbolKind::Struct => "S ",
        SymbolKind::Enum => "E ",
        SymbolKind::Trait => "T ",
        SymbolKind::ImplBlock => "im",
        SymbolKind::Constant => "co",
        SymbolKind::Static => "st",
        SymbolKind::TypeAlias => "ty",
    }
}
