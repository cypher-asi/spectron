//! Interactive graph canvas with pan, zoom, node selection, and edge filtering.

use std::collections::{HashMap, HashSet};

use egui::{Color32, FontId, Pos2, RichText, Sense, Shape, Stroke, Ui, Vec2};
use petgraph::graph::NodeIndex;
use spectron_core::{
    ArchGraph, GraphNode, RelationshipKind, SymbolId, SymbolKind,
};

use crate::ProjectData;

// ---------------------------------------------------------------------------
// Colors
// ---------------------------------------------------------------------------

const CRATE_COLOR: Color32 = Color32::from_rgb(110, 180, 255);
const MODULE_COLOR: Color32 = Color32::from_rgb(160, 215, 140);
const FUNCTION_COLOR: Color32 = Color32::from_rgb(255, 170, 100);
const STRUCT_COLOR: Color32 = Color32::from_rgb(200, 165, 255);
const TRAIT_COLOR: Color32 = Color32::from_rgb(100, 210, 210);
const FILE_COLOR: Color32 = Color32::from_rgb(140, 140, 150);
const DEFAULT_NODE_COLOR: Color32 = Color32::from_rgb(170, 170, 170);

const CONTAINS_EDGE: Color32 = Color32::from_rgb(80, 80, 80);
const CALLS_EDGE: Color32 = Color32::from_rgb(255, 170, 100);
const IMPORTS_EDGE: Color32 = Color32::from_rgb(110, 180, 255);
const IMPLEMENTS_EDGE: Color32 = Color32::from_rgb(160, 215, 140);
const DEPENDS_ON_EDGE: Color32 = Color32::from_rgb(255, 100, 100);
const REFERENCES_EDGE: Color32 = Color32::from_rgb(150, 150, 150);

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Persistent state for a single graph view (structure or call).
pub struct GraphViewState {
    pub positions: HashMap<NodeIndex, Vec2>,
    pub pan: Vec2,
    pub zoom: f32,
    pub selected: Option<NodeIndex>,
    hovered: Option<NodeIndex>,
    dragging: Option<NodeIndex>,
    pub initialized: bool,
    pub edge_filters: HashMap<RelationshipKind, bool>,
}

impl GraphViewState {
    pub fn new_structure() -> Self {
        let mut ef = HashMap::new();
        ef.insert(RelationshipKind::Contains, false);
        ef.insert(RelationshipKind::Calls, true);
        ef.insert(RelationshipKind::Imports, true);
        ef.insert(RelationshipKind::Implements, true);
        ef.insert(RelationshipKind::DependsOn, true);
        ef.insert(RelationshipKind::References, false);
        Self {
            positions: HashMap::new(),
            pan: Vec2::ZERO,
            zoom: 1.0,
            selected: None,
            hovered: None,
            dragging: None,
            initialized: false,
            edge_filters: ef,
        }
    }

    pub fn new_call() -> Self {
        let mut ef = HashMap::new();
        ef.insert(RelationshipKind::Contains, true);
        ef.insert(RelationshipKind::Calls, true);
        ef.insert(RelationshipKind::Imports, true);
        ef.insert(RelationshipKind::Implements, true);
        ef.insert(RelationshipKind::DependsOn, true);
        ef.insert(RelationshipKind::References, true);
        Self {
            positions: HashMap::new(),
            pan: Vec2::ZERO,
            zoom: 1.0,
            selected: None,
            hovered: None,
            dragging: None,
            initialized: false,
            edge_filters: ef,
        }
    }
}

// ---------------------------------------------------------------------------
// Click result
// ---------------------------------------------------------------------------

pub enum ClickResult {
    Nothing,
    NodeClicked(GraphNode),
    BackgroundClicked,
}

// ---------------------------------------------------------------------------
// Toolbar
// ---------------------------------------------------------------------------

pub fn show_toolbar(ui: &mut Ui, state: &mut GraphViewState) {
    ui.horizontal(|ui| {
        ui.label("Edges:");
        for (kind, label, color) in [
            (RelationshipKind::Contains, "Contains", CONTAINS_EDGE),
            (RelationshipKind::Calls, "Calls", CALLS_EDGE),
            (RelationshipKind::Imports, "Imports", IMPORTS_EDGE),
            (RelationshipKind::Implements, "Impl", IMPLEMENTS_EDGE),
            (RelationshipKind::DependsOn, "Deps", DEPENDS_ON_EDGE),
            (RelationshipKind::References, "Refs", REFERENCES_EDGE),
        ] {
            let checked = state.edge_filters.entry(kind).or_insert(true);
            ui.checkbox(checked, RichText::new(label).color(color).small());
        }
        ui.separator();
        if ui.small_button("Reset View").clicked() {
            state.pan = Vec2::ZERO;
            state.zoom = 1.0;
        }
        if ui.small_button("Re-layout").clicked() {
            state.initialized = false;
        }
    });
}

// ---------------------------------------------------------------------------
// Canvas
// ---------------------------------------------------------------------------

pub fn show_canvas(
    ui: &mut Ui,
    graph: &ArchGraph,
    state: &mut GraphViewState,
    data: &ProjectData,
    entrypoints: &HashSet<SymbolId>,
) -> ClickResult {
    if graph.node_count() == 0 {
        ui.centered_and_justified(|ui| {
            ui.label("No nodes to display.");
        });
        return ClickResult::Nothing;
    }

    if !state.initialized {
        let size = ui.available_size();
        let w = size.x.max(800.0);
        let h = size.y.max(600.0);
        state.positions = crate::layout::compute_layout(graph, w, h);
        state.initialized = true;
    }

    let (response, painter) =
        ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let rect = response.rect;

    // --- Zoom (scroll wheel) ---
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            let zoom_factor = 1.0 + scroll * 0.002;
            let old_zoom = state.zoom;
            state.zoom = (state.zoom * zoom_factor).clamp(0.1, 5.0);
            if let Some(pointer) = response.hover_pos() {
                let cursor = pointer.to_vec2() - rect.min.to_vec2();
                let ratio = state.zoom / old_zoom;
                state.pan = cursor * (1.0 - ratio) + state.pan * ratio;
            }
        }
    }

    // Coordinate transforms.
    let world_to_screen = |world: Vec2| -> Pos2 {
        let s = world * state.zoom + state.pan + rect.min.to_vec2();
        Pos2::new(s.x, s.y)
    };

    // --- Draw edges ---
    let expanded = rect.expand(60.0);
    for edge_idx in graph.edge_indices() {
        let edge = &graph[edge_idx];
        if !state.edge_filters.get(&edge.kind).copied().unwrap_or(true) {
            continue;
        }
        let Some((src, tgt)) = graph.edge_endpoints(edge_idx) else {
            continue;
        };
        let (Some(&sp), Some(&tp)) = (state.positions.get(&src), state.positions.get(&tgt))
        else {
            continue;
        };

        let from = world_to_screen(sp);
        let to = world_to_screen(tp);

        if !expanded.contains(from) && !expanded.contains(to) {
            continue;
        }

        let color = edge_color(&edge.kind);
        let width = if edge.kind == RelationshipKind::Contains {
            0.5
        } else {
            1.2
        } * state.zoom.sqrt();

        let dir = to - from;
        let dist = dir.length();
        if dist < 2.0 {
            continue;
        }
        let dir_n = dir / dist;

        let src_r = node_radius(&graph[src], data) * state.zoom;
        let tgt_r = node_radius(&graph[tgt], data) * state.zoom;
        let line_start = from + dir_n * src_r;
        let tip = to - dir_n * tgt_r;

        painter.line_segment([line_start, tip], Stroke::new(width, color));

        if dist > 40.0 * state.zoom {
            let perp = egui::vec2(-dir_n.y, dir_n.x);
            let arrow = 7.0 * state.zoom.sqrt();
            let p1 = tip;
            let p2 = tip - dir_n * arrow + perp * arrow * 0.4;
            let p3 = tip - dir_n * arrow - perp * arrow * 0.4;
            painter.add(Shape::convex_polygon(
                vec![p1, p2, p3],
                color,
                Stroke::NONE,
            ));
        }
    }

    // --- Draw nodes and detect hover ---
    state.hovered = None;
    let pointer = response.hover_pos();

    for node_idx in graph.node_indices() {
        let Some(&world_pos) = state.positions.get(&node_idx) else {
            continue;
        };
        let screen_pos = world_to_screen(world_pos);
        if !expanded.contains(screen_pos) {
            continue;
        }

        let node = &graph[node_idx];
        let radius = node_radius(node, data) * state.zoom;
        let color = node_color(node, data);
        let is_hovered =
            pointer.map_or(false, |p| p.distance(screen_pos) < radius + 3.0);
        if is_hovered {
            state.hovered = Some(node_idx);
        }
        let is_selected = state.selected == Some(node_idx);
        let is_entry =
            matches!(node, GraphNode::Symbol(sid) if entrypoints.contains(sid));

        // Entrypoint glow.
        if is_entry {
            painter.circle_filled(
                screen_pos,
                radius + 4.0 * state.zoom,
                Color32::from_rgba_unmultiplied(255, 200, 50, 50),
            );
        }
        // Selection ring.
        if is_selected {
            painter.circle_stroke(
                screen_pos,
                radius + 2.5,
                Stroke::new(2.0, Color32::WHITE),
            );
        }
        // Body.
        let fill = if is_hovered { lighten(color, 35) } else { color };
        painter.circle_filled(screen_pos, radius, fill);

        // Label (only when zoomed in enough).
        if state.zoom > 0.35 {
            let font = FontId::proportional((10.0 * state.zoom.sqrt()).clamp(7.0, 16.0));
            let text_pos = Pos2::new(screen_pos.x, screen_pos.y + radius + 3.0);
            painter.text(
                text_pos,
                egui::Align2::CENTER_TOP,
                node_label(node, data),
                font,
                Color32::from_rgb(220, 220, 220),
            );
        }
    }

    // --- Tooltip for hovered node ---
    if let Some(hovered_idx) = state.hovered {
        let node = &graph[hovered_idx];
        egui::show_tooltip_at_pointer(
            ui.ctx(),
            ui.layer_id(),
            egui::Id::new("graph_tip"),
            |ui| {
                show_node_tooltip(ui, node, data);
            },
        );
    }

    // --- Interaction: drag start ---
    if response.drag_started() {
        state.dragging = state.hovered;
    }

    // --- Interaction: ongoing drag ---
    if response.dragged() {
        if let Some(dragging) = state.dragging {
            if let Some(pos) = state.positions.get_mut(&dragging) {
                *pos += response.drag_delta() / state.zoom;
            }
        } else {
            state.pan += response.drag_delta();
        }
    }

    if response.drag_stopped() {
        state.dragging = None;
    }

    // --- Interaction: click ---
    if response.clicked() {
        if let Some(hovered) = state.hovered {
            state.selected = Some(hovered);
            return ClickResult::NodeClicked(graph[hovered].clone());
        } else {
            state.selected = None;
            return ClickResult::BackgroundClicked;
        }
    }

    ClickResult::Nothing
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_color(node: &GraphNode, data: &ProjectData) -> Color32 {
    match node {
        GraphNode::Crate(_) => CRATE_COLOR,
        GraphNode::Module(_) => MODULE_COLOR,
        GraphNode::File(_) => FILE_COLOR,
        GraphNode::Symbol(sid) => match data.symbols.get(sid) {
            Some(s) => match s.kind {
                SymbolKind::Function | SymbolKind::Method => FUNCTION_COLOR,
                SymbolKind::Struct | SymbolKind::Enum => STRUCT_COLOR,
                SymbolKind::Trait => TRAIT_COLOR,
                _ => DEFAULT_NODE_COLOR,
            },
            None => DEFAULT_NODE_COLOR,
        },
    }
}

fn edge_color(kind: &RelationshipKind) -> Color32 {
    match kind {
        RelationshipKind::Contains => CONTAINS_EDGE,
        RelationshipKind::Calls => CALLS_EDGE,
        RelationshipKind::Imports => IMPORTS_EDGE,
        RelationshipKind::Implements => IMPLEMENTS_EDGE,
        RelationshipKind::DependsOn => DEPENDS_ON_EDGE,
        RelationshipKind::References => REFERENCES_EDGE,
    }
}

fn node_label(node: &GraphNode, data: &ProjectData) -> String {
    match node {
        GraphNode::Crate(id) => data
            .crates
            .iter()
            .find(|c| c.id == *id)
            .map_or("?".into(), |c| c.name.clone()),
        GraphNode::Module(id) => data
            .modules
            .get(id)
            .map_or("?".into(), |m| m.name.clone()),
        GraphNode::File(id) => data
            .files
            .iter()
            .find(|f| f.id == *id)
            .map_or("?".into(), |f| {
                f.path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            }),
        GraphNode::Symbol(id) => data
            .symbols
            .get(id)
            .map_or("?".into(), |s| s.name.clone()),
    }
}

fn node_radius(node: &GraphNode, data: &ProjectData) -> f32 {
    match node {
        GraphNode::Crate(_) => 16.0,
        GraphNode::Module(_) => 12.0,
        GraphNode::File(_) => 7.0,
        GraphNode::Symbol(id) => {
            if let Some(m) = data.analysis.symbol_metrics.get(id) {
                (6.0 + (m.line_count as f32).ln().max(0.0) * 1.8).min(20.0)
            } else {
                8.0
            }
        }
    }
}

fn lighten(c: Color32, amt: u8) -> Color32 {
    Color32::from_rgb(
        c.r().saturating_add(amt),
        c.g().saturating_add(amt),
        c.b().saturating_add(amt),
    )
}

fn show_node_tooltip(ui: &mut Ui, node: &GraphNode, data: &ProjectData) {
    match node {
        GraphNode::Crate(id) => {
            if let Some(c) = data.crates.iter().find(|c| c.id == *id) {
                ui.label(RichText::new(&c.name).strong());
                ui.label(format!(
                    "Crate ({}) \u{2022} {} modules",
                    match c.crate_type {
                        spectron_core::CrateType::Library => "lib",
                        spectron_core::CrateType::Binary => "bin",
                    },
                    c.module_ids.len()
                ));
            }
        }
        GraphNode::Module(id) => {
            if let Some(m) = data.modules.get(id) {
                ui.label(RichText::new(&m.name).strong());
                ui.label(RichText::new(m.path.as_str()).monospace().small());
            }
        }
        GraphNode::File(id) => {
            if let Some(f) = data.files.iter().find(|f| f.id == *id) {
                ui.label(RichText::new(f.path.display().to_string()).monospace());
                ui.label(format!("{} lines", f.line_count));
            }
        }
        GraphNode::Symbol(id) => {
            if let Some(s) = data.symbols.get(id) {
                ui.label(RichText::new(&s.name).strong());
                ui.label(format!("{:?} \u{2022} {:?}", s.kind, s.visibility));
                if let Some(ref sig) = s.signature {
                    ui.label(RichText::new(sig).monospace().small());
                }
            }
        }
    }
}
