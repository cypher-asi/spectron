//! Inspector panel: detailed view for a selected graph node.

use egui::{Color32, RichText, Ui};
use spectron_core::{CrateId, CrateType, GraphNode, ModuleId, SymbolId, SymbolKind, Visibility};

use crate::ProjectData;

const DIM: Color32 = Color32::from_rgb(150, 150, 150);
const ORANGE: Color32 = Color32::from_rgb(255, 170, 100);
const BLUE: Color32 = Color32::from_rgb(110, 180, 255);
const GREEN: Color32 = Color32::from_rgb(160, 215, 140);
const PURPLE: Color32 = Color32::from_rgb(200, 165, 255);
const TEAL: Color32 = Color32::from_rgb(100, 210, 210);
const RED: Color32 = Color32::from_rgb(255, 110, 110);

// ---------------------------------------------------------------------------
// Inspector target
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub enum InspectorTarget {
    Symbol(SymbolId),
    Module(ModuleId),
    Crate(CrateId),
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

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn show_inspector(
    ui: &mut Ui,
    target: &InspectorTarget,
    data: &ProjectData,
    callers_clicked: &mut Option<SymbolId>,
) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match target {
            InspectorTarget::Symbol(id) => show_symbol(ui, *id, data, callers_clicked),
            InspectorTarget::Module(id) => show_module(ui, *id, data),
            InspectorTarget::Crate(id) => show_crate(ui, *id, data),
        });
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
    let Some(c) = data.crates.iter().find(|c| c.id == cid) else {
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
        SymbolKind::Function | SymbolKind::Method => ORANGE,
        SymbolKind::Struct | SymbolKind::Enum => PURPLE,
        SymbolKind::Trait => TEAL,
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
