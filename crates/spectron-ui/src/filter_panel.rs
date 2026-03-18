//! Right-side filter panel for controlling graph node/edge visibility.

use std::collections::HashSet;

use egui::{Color32, RichText, Ui};
use spectron_core::{RelationshipKind, SymbolId, SymbolKind, Visibility};

use crate::graph_view::{
    GraphViewState, NodeTypeFilter, CALLS_EDGE, CONTAINS_EDGE, CRATE_COLOR, DEFAULT_NODE_COLOR,
    DEPENDS_ON_EDGE, FILE_COLOR, FUNCTION_COLOR, IMPLEMENTS_EDGE, IMPORTS_EDGE, MODULE_COLOR,
    REFERENCES_EDGE, STRUCT_COLOR, TRAIT_COLOR,
};
use crate::ProjectData;

const DIM: Color32 = Color32::from_rgb(150, 150, 150);

/// Show the filter panel. Returns `true` if any filter changed (caller should
/// trigger a re-layout).
pub fn show_filter_panel(
    ui: &mut Ui,
    state: &mut GraphViewState,
    data: &ProjectData,
    entrypoints: &HashSet<SymbolId>,
) -> bool {
    let mut changed = false;

    ui.label(RichText::new("Filters").strong());
    ui.add_space(4.0);

    changed |= show_presets(ui, state);
    ui.add_space(4.0);
    ui.separator();
    changed |= show_node_types(ui, state);
    ui.add_space(2.0);
    ui.separator();
    changed |= show_symbol_kinds(ui, state);
    ui.add_space(2.0);
    ui.separator();
    changed |= show_edge_types(ui, state);
    ui.add_space(2.0);
    ui.separator();
    changed |= show_visibility(ui, state);
    ui.add_space(2.0);
    ui.separator();
    changed |= show_crates(ui, state, data);
    ui.add_space(2.0);
    ui.separator();
    changed |= show_highlights(ui, state, data, entrypoints);
    ui.add_space(2.0);
    ui.separator();
    changed |= show_focus_controls(ui, state);
    ui.add_space(2.0);
    ui.separator();
    changed |= show_degree_filter(ui, state);

    changed
}

// ---------------------------------------------------------------------------
// Presets
// ---------------------------------------------------------------------------

fn show_presets(ui: &mut Ui, state: &mut GraphViewState) -> bool {
    let mut changed = false;

    egui::CollapsingHeader::new(RichText::new("Presets").small().strong())
        .default_open(true)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.small_button("Dependencies").clicked() {
                    apply_preset_dependencies(state);
                    changed = true;
                }
                if ui.small_button("Modules").clicked() {
                    apply_preset_module_structure(state);
                    changed = true;
                }
                if ui.small_button("Call Flow").clicked() {
                    apply_preset_call_flow(state);
                    changed = true;
                }
            });
            ui.horizontal_wrapped(|ui| {
                if ui.small_button("Type Graph").clicked() {
                    apply_preset_type_graph(state);
                    changed = true;
                }
                if ui.small_button("Imports").clicked() {
                    apply_preset_imports(state);
                    changed = true;
                }
                if ui.small_button("Everything").clicked() {
                    apply_preset_everything(state);
                    changed = true;
                }
            });
        });

    changed
}

fn set_all_node_types(state: &mut GraphViewState, val: bool) {
    for v in state.node_type_filters.values_mut() {
        *v = val;
    }
}

fn set_all_symbol_kinds(state: &mut GraphViewState, val: bool) {
    for v in state.symbol_kind_filters.values_mut() {
        *v = val;
    }
}

fn set_all_edges(state: &mut GraphViewState, val: bool) {
    for v in state.edge_filters.values_mut() {
        *v = val;
    }
}

fn set_all_visibility(state: &mut GraphViewState, val: bool) {
    for v in state.visibility_filters.values_mut() {
        *v = val;
    }
}

fn reset_highlights(state: &mut GraphViewState) {
    state.highlight_entrypoints_only = false;
    state.highlight_unsafe_only = false;
    state.highlight_flagged_only = false;
}

fn apply_preset_dependencies(state: &mut GraphViewState) {
    set_all_node_types(state, false);
    state.node_type_filters.insert(NodeTypeFilter::Crate, true);
    set_all_symbol_kinds(state, false);
    set_all_edges(state, false);
    state.edge_filters.insert(RelationshipKind::DependsOn, true);
    set_all_visibility(state, true);
    reset_highlights(state);
}

fn apply_preset_module_structure(state: &mut GraphViewState) {
    set_all_node_types(state, false);
    state.node_type_filters.insert(NodeTypeFilter::Crate, true);
    state.node_type_filters.insert(NodeTypeFilter::Module, true);
    set_all_symbol_kinds(state, false);
    set_all_edges(state, false);
    state.edge_filters.insert(RelationshipKind::Contains, true);
    set_all_visibility(state, true);
    reset_highlights(state);
}

fn apply_preset_call_flow(state: &mut GraphViewState) {
    set_all_node_types(state, false);
    state.node_type_filters.insert(NodeTypeFilter::Symbol, true);
    set_all_symbol_kinds(state, false);
    state.symbol_kind_filters.insert(SymbolKind::Function, true);
    state.symbol_kind_filters.insert(SymbolKind::Method, true);
    set_all_edges(state, false);
    state.edge_filters.insert(RelationshipKind::Calls, true);
    set_all_visibility(state, true);
    reset_highlights(state);
}

fn apply_preset_type_graph(state: &mut GraphViewState) {
    set_all_node_types(state, false);
    state.node_type_filters.insert(NodeTypeFilter::Symbol, true);
    set_all_symbol_kinds(state, false);
    state.symbol_kind_filters.insert(SymbolKind::Struct, true);
    state.symbol_kind_filters.insert(SymbolKind::Enum, true);
    state.symbol_kind_filters.insert(SymbolKind::Trait, true);
    set_all_edges(state, false);
    state.edge_filters.insert(RelationshipKind::Implements, true);
    set_all_visibility(state, true);
    reset_highlights(state);
}

fn apply_preset_imports(state: &mut GraphViewState) {
    set_all_node_types(state, true);
    set_all_symbol_kinds(state, true);
    set_all_edges(state, false);
    state.edge_filters.insert(RelationshipKind::Imports, true);
    set_all_visibility(state, true);
    reset_highlights(state);
}

fn apply_preset_everything(state: &mut GraphViewState) {
    set_all_node_types(state, true);
    set_all_symbol_kinds(state, true);
    set_all_edges(state, true);
    set_all_visibility(state, true);
    reset_highlights(state);
}

// ---------------------------------------------------------------------------
// Node types
// ---------------------------------------------------------------------------

fn show_node_types(ui: &mut Ui, state: &mut GraphViewState) -> bool {
    let mut changed = false;

    egui::CollapsingHeader::new(RichText::new("Node Types").small().strong())
        .default_open(true)
        .show(ui, |ui| {
            for (filter, label, color) in [
                (NodeTypeFilter::Crate, "Crates", CRATE_COLOR),
                (NodeTypeFilter::Module, "Modules", MODULE_COLOR),
                (NodeTypeFilter::File, "Files", FILE_COLOR),
                (NodeTypeFilter::Symbol, "Symbols", FUNCTION_COLOR),
            ] {
                let checked = state.node_type_filters.entry(filter).or_insert(true);
                if ui
                    .checkbox(checked, RichText::new(label).color(color).small())
                    .changed()
                {
                    changed = true;
                }
            }
            ui.add_space(4.0);
            if ui
                .checkbox(
                    &mut state.hide_test_code,
                    RichText::new("Hide test code").small().color(DIM),
                )
                .changed()
            {
                changed = true;
            }
        });

    changed
}

// ---------------------------------------------------------------------------
// Symbol kinds
// ---------------------------------------------------------------------------

fn show_symbol_kinds(ui: &mut Ui, state: &mut GraphViewState) -> bool {
    let symbols_on = state
        .node_type_filters
        .get(&NodeTypeFilter::Symbol)
        .copied()
        .unwrap_or(true);

    let mut changed = false;

    egui::CollapsingHeader::new(RichText::new("Symbol Kinds").small().strong())
        .default_open(false)
        .show(ui, |ui| {
            ui.add_enabled_ui(symbols_on, |ui| {
                for (kind, label, color) in [
                    (SymbolKind::Function, "Function", FUNCTION_COLOR),
                    (SymbolKind::Method, "Method", FUNCTION_COLOR),
                    (SymbolKind::Struct, "Struct", STRUCT_COLOR),
                    (SymbolKind::Enum, "Enum", STRUCT_COLOR),
                    (SymbolKind::Trait, "Trait", TRAIT_COLOR),
                    (SymbolKind::ImplBlock, "Impl Block", DEFAULT_NODE_COLOR),
                    (SymbolKind::Constant, "Constant", DEFAULT_NODE_COLOR),
                    (SymbolKind::Static, "Static", DEFAULT_NODE_COLOR),
                    (SymbolKind::TypeAlias, "Type Alias", DEFAULT_NODE_COLOR),
                ] {
                    let checked = state.symbol_kind_filters.entry(kind).or_insert(true);
                    if ui
                        .checkbox(checked, RichText::new(label).color(color).small())
                        .changed()
                    {
                        changed = true;
                    }
                }
            });
        });

    changed
}

// ---------------------------------------------------------------------------
// Edge types
// ---------------------------------------------------------------------------

fn show_edge_types(ui: &mut Ui, state: &mut GraphViewState) -> bool {
    let mut changed = false;

    egui::CollapsingHeader::new(RichText::new("Edge Types").small().strong())
        .default_open(true)
        .show(ui, |ui| {
            for (kind, label, color) in [
                (RelationshipKind::Contains, "Contains", CONTAINS_EDGE),
                (RelationshipKind::Calls, "Calls", CALLS_EDGE),
                (RelationshipKind::Imports, "Imports", IMPORTS_EDGE),
                (RelationshipKind::Implements, "Implements", IMPLEMENTS_EDGE),
                (RelationshipKind::DependsOn, "DependsOn", DEPENDS_ON_EDGE),
                (RelationshipKind::References, "References", REFERENCES_EDGE),
            ] {
                let checked = state.edge_filters.entry(kind).or_insert(true);
                if ui
                    .checkbox(checked, RichText::new(label).color(color).small())
                    .changed()
                {
                    changed = true;
                }
            }
        });

    changed
}

// ---------------------------------------------------------------------------
// Visibility
// ---------------------------------------------------------------------------

fn show_visibility(ui: &mut Ui, state: &mut GraphViewState) -> bool {
    let mut changed = false;

    egui::CollapsingHeader::new(RichText::new("Visibility").small().strong())
        .default_open(false)
        .show(ui, |ui| {
            for (vis, label) in [
                (Visibility::Public, "pub"),
                (Visibility::Crate, "pub(crate)"),
                (Visibility::Restricted, "pub(restricted)"),
                (Visibility::Private, "private"),
            ] {
                let checked = state.visibility_filters.entry(vis).or_insert(true);
                if ui
                    .checkbox(checked, RichText::new(label).small())
                    .changed()
                {
                    changed = true;
                }
            }
        });

    changed
}

// ---------------------------------------------------------------------------
// Crates
// ---------------------------------------------------------------------------

fn show_crates(ui: &mut Ui, state: &mut GraphViewState, data: &ProjectData) -> bool {
    let mut changed = false;

    egui::CollapsingHeader::new(RichText::new("Crates").small().strong())
        .default_open(false)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.small_button("All").clicked() {
                    for v in state.crate_filters.values_mut() {
                        *v = true;
                    }
                    changed = true;
                }
                if ui.small_button("None").clicked() {
                    for v in state.crate_filters.values_mut() {
                        *v = false;
                    }
                    changed = true;
                }
            });
            for krate in &data.crates {
                let checked = state.crate_filters.entry(krate.id).or_insert(true);
                if ui
                    .checkbox(
                        checked,
                        RichText::new(&krate.name).color(CRATE_COLOR).small(),
                    )
                    .changed()
                {
                    changed = true;
                }
            }
        });

    changed
}

// ---------------------------------------------------------------------------
// Highlights (isolation modes)
// ---------------------------------------------------------------------------

fn show_highlights(
    ui: &mut Ui,
    state: &mut GraphViewState,
    data: &ProjectData,
    entrypoints: &HashSet<SymbolId>,
) -> bool {
    let mut changed = false;

    egui::CollapsingHeader::new(RichText::new("Isolate").small().strong())
        .default_open(false)
        .show(ui, |ui| {
            let label_entry = format!("Entrypoints only ({})", entrypoints.len());
            if ui
                .checkbox(
                    &mut state.highlight_entrypoints_only,
                    RichText::new(label_entry).small(),
                )
                .changed()
            {
                changed = true;
            }

            let unsafe_count = data
                .symbols
                .values()
                .filter(|s| s.attributes.is_unsafe || s.attributes.has_unsafe_block)
                .count();
            let label_unsafe = format!("Unsafe only ({})", unsafe_count);
            if ui
                .checkbox(
                    &mut state.highlight_unsafe_only,
                    RichText::new(label_unsafe)
                        .small()
                        .color(Color32::from_rgb(255, 110, 110)),
                )
                .changed()
            {
                changed = true;
            }

            let flagged_count = data.analysis.complexity_flags.len();
            let label_flagged = format!("Flagged only ({})", flagged_count);
            if ui
                .checkbox(
                    &mut state.highlight_flagged_only,
                    RichText::new(label_flagged)
                        .small()
                        .color(Color32::from_rgb(255, 200, 50)),
                )
                .changed()
            {
                changed = true;
            }
        });

    changed
}

// ---------------------------------------------------------------------------
// Focus / ego controls
// ---------------------------------------------------------------------------

fn show_focus_controls(ui: &mut Ui, state: &mut GraphViewState) -> bool {
    let mut changed = false;

    egui::CollapsingHeader::new(RichText::new("Focus / Ego").small().strong())
        .default_open(false)
        .show(ui, |ui| {
            if state.focus_node.is_some() {
                ui.label(RichText::new("Focus node active").small().color(Color32::from_rgb(255, 200, 50)));
                ui.add_space(2.0);
                let mut depth = state.focus_depth as f64;
                if ui
                    .add(
                        egui::Slider::new(&mut depth, 1.0..=5.0)
                            .text("Depth")
                            .integer(),
                    )
                    .changed()
                {
                    state.focus_depth = depth as usize;
                    changed = true;
                }
                if ui.small_button("Clear Focus").clicked() {
                    state.focus_node = None;
                    changed = true;
                }
            } else {
                ui.label(
                    RichText::new("Right-click a node in the inspector to set focus")
                        .small()
                        .color(DIM),
                );
            }
            if !state.pinned_nodes.is_empty() {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("{} pinned nodes", state.pinned_nodes.len()))
                        .small(),
                );
                if ui.small_button("Clear Pins").clicked() {
                    state.pinned_nodes.clear();
                }
            }
        });

    changed
}

// ---------------------------------------------------------------------------
// Degree filter (low-degree node collapse)
// ---------------------------------------------------------------------------

fn show_degree_filter(ui: &mut Ui, _state: &mut GraphViewState) -> bool {
    let mut _changed = false;

    egui::CollapsingHeader::new(RichText::new("Degree Filter").small().strong())
        .default_open(false)
        .show(ui, |ui| {
            ui.label(
                RichText::new("Min degree slider (future)")
                    .small()
                    .color(DIM),
            );
        });

    _changed
}
