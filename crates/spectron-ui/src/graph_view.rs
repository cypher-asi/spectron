//! Interactive graph canvas with pan, zoom, node selection, and edge filtering.

use std::collections::HashMap;
use std::collections::HashSet;

use egui::{Color32, FontId, Pos2, RichText, Sense, Shape, Stroke, Ui, Vec2};
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use spectron_core::{
    ArchGraph, CrateId, GraphNode, RelationshipKind, SymbolId, SymbolKind, Visibility,
};

use crate::ProjectData;

// ---------------------------------------------------------------------------
// Colors (pub for filter_panel)
// ---------------------------------------------------------------------------

pub const CRATE_COLOR: Color32 = Color32::from_rgb(77, 84, 245); // #4D54F5
pub const MODULE_COLOR: Color32 = Color32::from_rgb(82, 242, 132); // #52F284
pub const FUNCTION_COLOR: Color32 = Color32::from_rgb(254, 75, 66); // #FE4B42
pub const STRUCT_COLOR: Color32 = Color32::from_rgb(182, 83, 249); // #B653F9
pub const TRAIT_COLOR: Color32 = Color32::from_rgb(171, 240, 18); // #ABF012
pub const FILE_COLOR: Color32 = Color32::from_rgb(140, 140, 150);
pub const DEFAULT_NODE_COLOR: Color32 = Color32::from_rgb(170, 170, 170);

pub const CONTAINS_EDGE: Color32 = Color32::from_rgb(80, 80, 80);
pub const CALLS_EDGE: Color32 = Color32::from_rgb(254, 75, 66); // #FE4B42
pub const IMPORTS_EDGE: Color32 = Color32::from_rgb(77, 84, 245); // #4D54F5
pub const IMPLEMENTS_EDGE: Color32 = Color32::from_rgb(82, 242, 132); // #52F284
pub const DEPENDS_ON_EDGE: Color32 = Color32::from_rgb(254, 75, 66); // #FE4B42
pub const REFERENCES_EDGE: Color32 = Color32::from_rgb(150, 150, 150);

// ---------------------------------------------------------------------------
// Node type filter
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeTypeFilter {
    Crate,
    Module,
    File,
    Symbol,
}

const GRID_CELL: f32 = 50.0;

// ---------------------------------------------------------------------------
// Spatial grid for O(visible) viewport culling
// ---------------------------------------------------------------------------

struct SpatialGrid {
    cells: HashMap<(i32, i32), Vec<NodeIndex>>,
}

impl SpatialGrid {
    fn new() -> Self {
        Self {
            cells: HashMap::new(),
        }
    }

    fn rebuild(&mut self, positions: &HashMap<NodeIndex, Vec2>) {
        self.cells.clear();
        for (&ni, &pos) in positions {
            let cx = (pos.x / GRID_CELL).floor() as i32;
            let cy = (pos.y / GRID_CELL).floor() as i32;
            self.cells.entry((cx, cy)).or_default().push(ni);
        }
    }
}

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
    pub node_type_filters: HashMap<NodeTypeFilter, bool>,
    pub symbol_kind_filters: HashMap<SymbolKind, bool>,
    pub visibility_filters: HashMap<Visibility, bool>,
    pub crate_filters: HashMap<CrateId, bool>,
    pub highlight_entrypoints_only: bool,
    pub highlight_unsafe_only: bool,
    pub highlight_flagged_only: bool,
    pub hide_test_code: bool,
    // Phase 3B: Focus / ego view
    pub focus_node: Option<NodeIndex>,
    pub focus_depth: usize,
    // Phase 3D: Pin selection
    pub pinned_nodes: HashSet<NodeIndex>,
    // Phase 3A: Layout algorithm selector
    pub layout_algorithm: LayoutAlgorithm,
    pub active_preset: Option<Preset>,
    pub request_fit: bool,
    layout: Option<crate::layout::LayoutState>,
    grid: SpatialGrid,
    label_cache: HashMap<NodeIndex, String>,
    /// Cluster rectangles produced by the Grouped layout (empty for other layouts).
    pub cluster_rects: Vec<crate::layout::ClusterRect>,
    /// Set of NodeIndex values that participate in dependency cycles.
    /// Populated externally; used to highlight cycle edges and node borders.
    pub cycle_nodes: HashSet<NodeIndex>,
    /// Per-node coupling score for heatmap colouring (0.0 = cool, higher = hot).
    pub coupling_heatmap: HashMap<NodeIndex, f32>,
}

/// Layout algorithm selector.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LayoutAlgorithm {
    ForceDirected,
    Layered,
    Grouped,
}

/// Active filter preset (used for visual feedback in the filter panel).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Dependencies,
    Modules,
    CallFlow,
    TypeGraph,
    Imports,
    Everything,
}

fn default_filters() -> (
    HashMap<NodeTypeFilter, bool>,
    HashMap<SymbolKind, bool>,
    HashMap<Visibility, bool>,
) {
    let mut nt = HashMap::new();
    nt.insert(NodeTypeFilter::Crate, true);
    nt.insert(NodeTypeFilter::Module, true);
    nt.insert(NodeTypeFilter::File, true);
    nt.insert(NodeTypeFilter::Symbol, true);

    let mut sk = HashMap::new();
    sk.insert(SymbolKind::Function, true);
    sk.insert(SymbolKind::Method, true);
    sk.insert(SymbolKind::Struct, true);
    sk.insert(SymbolKind::Enum, true);
    sk.insert(SymbolKind::Trait, true);
    sk.insert(SymbolKind::ImplBlock, true);
    sk.insert(SymbolKind::Constant, true);
    sk.insert(SymbolKind::Static, true);
    sk.insert(SymbolKind::TypeAlias, true);

    let mut vis = HashMap::new();
    vis.insert(Visibility::Public, true);
    vis.insert(Visibility::Crate, true);
    vis.insert(Visibility::Restricted, true);
    vis.insert(Visibility::Private, true);

    (nt, sk, vis)
}

fn default_state(edge_filters: HashMap<RelationshipKind, bool>) -> GraphViewState {
    let (node_type_filters, symbol_kind_filters, visibility_filters) = default_filters();
    GraphViewState {
        positions: HashMap::new(),
        pan: Vec2::ZERO,
        zoom: 1.0,
        selected: None,
        hovered: None,
        dragging: None,
        initialized: false,
        edge_filters,
        node_type_filters,
        symbol_kind_filters,
        visibility_filters,
        crate_filters: HashMap::new(),
        highlight_entrypoints_only: false,
        highlight_unsafe_only: false,
        highlight_flagged_only: false,
        hide_test_code: true,
        focus_node: None,
        focus_depth: 1,
        pinned_nodes: HashSet::new(),
        layout_algorithm: LayoutAlgorithm::Layered,
        active_preset: None,
        request_fit: false,
        layout: None,
        grid: SpatialGrid::new(),
        label_cache: HashMap::new(),
        cluster_rects: Vec::new(),
        cycle_nodes: HashSet::new(),
        coupling_heatmap: HashMap::new(),
    }
}

impl GraphViewState {
    /// Smart default: Crate nodes + DependsOn edges only (architecture view).
    /// Uses the Grouped layout by default for clear cluster-based visualisation.
    pub fn new_structure() -> Self {
        let mut ef = HashMap::new();
        ef.insert(RelationshipKind::Contains, false);
        ef.insert(RelationshipKind::Calls, false);
        ef.insert(RelationshipKind::Imports, false);
        ef.insert(RelationshipKind::Implements, false);
        ef.insert(RelationshipKind::DependsOn, true);
        ef.insert(RelationshipKind::References, false);

        let mut nt = HashMap::new();
        nt.insert(NodeTypeFilter::Crate, true);
        nt.insert(NodeTypeFilter::Module, false);
        nt.insert(NodeTypeFilter::File, false);
        nt.insert(NodeTypeFilter::Symbol, false);

        let (_, symbol_kind_filters, visibility_filters) = default_filters();
        GraphViewState {
            node_type_filters: nt,
            symbol_kind_filters,
            visibility_filters,
            layout_algorithm: LayoutAlgorithm::Layered,
            ..default_state(ef)
        }
    }

    pub fn new_call() -> Self {
        let mut ef = HashMap::new();
        ef.insert(RelationshipKind::Contains, false);
        ef.insert(RelationshipKind::Calls, true);
        ef.insert(RelationshipKind::Imports, false);
        ef.insert(RelationshipKind::Implements, false);
        ef.insert(RelationshipKind::DependsOn, false);
        ef.insert(RelationshipKind::References, false);
        default_state(ef)
    }

    pub fn init_crate_filters(&mut self, crate_ids: &[CrateId]) {
        for &id in crate_ids {
            self.crate_filters.entry(id).or_insert(true);
        }
    }
}

// ---------------------------------------------------------------------------
// Click result
// ---------------------------------------------------------------------------

pub enum ClickResult {
    Nothing,
    NodeClicked(NodeIndex),
    BackgroundClicked,
}

// ---------------------------------------------------------------------------
// Node visibility
// ---------------------------------------------------------------------------

/// Structural visibility: node type, crate, symbol kind, visibility, test code filters.
/// Highlight modes (unsafe/flagged/entrypoints) are handled separately as dimming.
fn is_node_visible(
    node: &GraphNode,
    state: &GraphViewState,
    data: &ProjectData,
) -> bool {
    let type_key = match node {
        GraphNode::Crate(_) => NodeTypeFilter::Crate,
        GraphNode::Module(_) => NodeTypeFilter::Module,
        GraphNode::File(_) => NodeTypeFilter::File,
        GraphNode::Symbol(_) => NodeTypeFilter::Symbol,
    };
    if !state
        .node_type_filters
        .get(&type_key)
        .copied()
        .unwrap_or(true)
    {
        return false;
    }

    match node {
        GraphNode::Crate(cid) => {
            if !state.crate_filters.get(cid).copied().unwrap_or(true) {
                return false;
            }
        }
        GraphNode::Module(mid) => {
            if let Some(cid) = data.module_to_crate.get(mid) {
                if !state.crate_filters.get(cid).copied().unwrap_or(true) {
                    return false;
                }
            }
        }
        GraphNode::Symbol(sid) => {
            if let Some(sym) = data.symbols.get(sid) {
                if let Some(cid) = data.module_to_crate.get(&sym.module_id) {
                    if !state.crate_filters.get(cid).copied().unwrap_or(true) {
                        return false;
                    }
                }
                if !state
                    .symbol_kind_filters
                    .get(&sym.kind)
                    .copied()
                    .unwrap_or(true)
                {
                    return false;
                }
                if !state
                    .visibility_filters
                    .get(&sym.visibility)
                    .copied()
                    .unwrap_or(true)
                {
                    return false;
                }
                if state.hide_test_code && sym.attributes.is_test {
                    return false;
                }
            }
        }
        GraphNode::File(_) => {}
    }

    true
}

/// Returns true if the node is "highlighted" (should be bright) when a highlight
/// isolation mode is active. Nodes that return false are dimmed, not hidden.
fn is_node_highlighted(
    node: &GraphNode,
    state: &GraphViewState,
    data: &ProjectData,
    entrypoints: &HashSet<SymbolId>,
) -> bool {
    let any_highlight =
        state.highlight_entrypoints_only || state.highlight_unsafe_only || state.highlight_flagged_only;
    if !any_highlight {
        return true;
    }

    let GraphNode::Symbol(sid) = node else {
        return false;
    };

    if state.highlight_entrypoints_only && !entrypoints.contains(sid) {
        return false;
    }
    if state.highlight_unsafe_only {
        let is_unsafe = data
            .symbols
            .get(sid)
            .map_or(false, |s| s.attributes.is_unsafe || s.attributes.has_unsafe_block);
        if !is_unsafe {
            return false;
        }
    }
    if state.highlight_flagged_only {
        use spectron_analysis::FlagTarget;
        let is_flagged = data
            .analysis
            .complexity_flags
            .iter()
            .any(|f| matches!(&f.target, FlagTarget::Symbol(s) if s == sid));
        if !is_flagged {
            return false;
        }
    }

    true
}

fn compute_visible_nodes(
    graph: &ArchGraph,
    state: &GraphViewState,
    data: &ProjectData,
) -> HashSet<NodeIndex> {
    graph
        .node_indices()
        .filter(|&ni| is_node_visible(&graph[ni], state, data))
        .collect()
}

/// Compute the set of nodes within `depth` hops of `center` (both directions).
fn compute_ego_set(
    graph: &ArchGraph,
    center: NodeIndex,
    depth: usize,
    visible: &HashSet<NodeIndex>,
) -> HashSet<NodeIndex> {
    let mut result = HashSet::new();
    result.insert(center);
    let mut frontier = vec![center];
    for _ in 0..depth {
        let mut next_frontier = Vec::new();
        for &n in &frontier {
            for neighbor in graph.neighbors_directed(n, Direction::Outgoing) {
                if visible.contains(&neighbor) && result.insert(neighbor) {
                    next_frontier.push(neighbor);
                }
            }
            for neighbor in graph.neighbors_directed(n, Direction::Incoming) {
                if visible.contains(&neighbor) && result.insert(neighbor) {
                    next_frontier.push(neighbor);
                }
            }
        }
        frontier = next_frontier;
    }
    result
}

/// Collect the 1-hop neighbor set of `center` (both directions).
fn collect_neighbors(graph: &ArchGraph, center: NodeIndex) -> HashSet<NodeIndex> {
    let mut set = HashSet::new();
    set.insert(center);
    for neighbor in graph.neighbors_directed(center, Direction::Outgoing) {
        set.insert(neighbor);
    }
    for neighbor in graph.neighbors_directed(center, Direction::Incoming) {
        set.insert(neighbor);
    }
    set
}

/// Compute per-node alpha multiplier based on highlight, focus, hover, and pin state.
fn compute_node_alpha(
    node_idx: NodeIndex,
    graph: &ArchGraph,
    state: &GraphViewState,
    data: &ProjectData,
    entrypoints: &HashSet<SymbolId>,
    hover_neighbors: &Option<HashSet<NodeIndex>>,
    focus_set: &Option<HashSet<NodeIndex>>,
) -> f32 {
    let mut alpha = 1.0_f32;

    if !is_node_highlighted(&graph[node_idx], state, data, entrypoints) {
        alpha = alpha.min(0.15);
    }

    if let Some(ref fset) = focus_set {
        if !fset.contains(&node_idx) {
            alpha = alpha.min(0.12);
        }
    }

    if !state.pinned_nodes.is_empty() && !state.pinned_nodes.contains(&node_idx) {
        alpha = alpha.min(0.15);
    }

    if let Some(ref hn) = hover_neighbors {
        if !hn.contains(&node_idx) {
            alpha = alpha.min(0.20);
        }
    }

    alpha
}

fn with_alpha(c: Color32, alpha: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as f32 * alpha) as u8)
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

    let visible = compute_visible_nodes(graph, state, data);

    if visible.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.label("All nodes hidden by current filters.");
        });
        return ClickResult::Nothing;
    }

    // --- Initialisation / re-layout ---
    if !state.initialized {
        let size = ui.available_size();
        let vw = size.x.max(800.0);
        let vh = size.y.max(600.0);
        // Use a larger virtual canvas so the layout has room to spread nodes.
        let scale = 2.5_f32 + (visible.len() as f32 / 30.0).min(2.0);
        let w = vw * scale;
        let h = vh * scale;

        state.cluster_rects.clear();
        match state.layout_algorithm {
            LayoutAlgorithm::ForceDirected => {
                let layout = crate::layout::LayoutState::new_filtered(graph, w, h, &visible);
                state.positions = layout.to_position_map();
                state.layout = Some(layout);
            }
            LayoutAlgorithm::Layered => {
                state.positions =
                    crate::layout::compute_layered_layout(graph, w, h, &visible);
                state.layout = None;
            }
            LayoutAlgorithm::Grouped => {
                let (pos, rects) =
                    crate::layout::compute_grouped_layout(graph, w, h, &visible);
                state.positions = pos;
                state.cluster_rects = rects;
                state.layout = None;
            }
        }

        state.label_cache.clear();
        for &node_idx in &visible {
            let node = &graph[node_idx];
            state.label_cache.insert(node_idx, node_label(node, data));
        }

        // Auto-fit: compute bounding box and set zoom/pan to show all nodes.
        auto_fit_viewport(state, vw, vh);

        state.initialized = true;
    }

    // --- Incremental layout stepping (force-directed only) ---
    if let Some(ref mut layout) = state.layout {
        if !layout.done {
            let was_running = true;
            layout.step(graph);
            state.positions = layout.to_position_map();
            if was_running && layout.done {
                let size = ui.available_size();
                auto_fit_viewport(state, size.x.max(800.0), size.y.max(600.0));
            }
            ui.ctx().request_repaint();
        }
    }

    // --- Handle deferred fit-all request from the filter panel ---
    if state.request_fit {
        state.request_fit = false;
        let size = ui.available_size();
        auto_fit_viewport(state, size.x.max(800.0), size.y.max(600.0));
    }

    // --- Rebuild spatial grid from current positions ---
    state.grid.rebuild(&state.positions);

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
    let zoom = state.zoom;
    let pan = state.pan;
    let rect_min = rect.min.to_vec2();

    let world_to_screen = |world: Vec2| -> Pos2 {
        let s = world * zoom + pan + rect_min;
        Pos2::new(s.x, s.y)
    };

    let screen_to_world = |screen: Pos2| -> Vec2 {
        (screen.to_vec2() - rect_min - pan) / zoom
    };

    // --- Precompute hover detection (first pass over nodes to find hovered) ---
    state.hovered = None;
    let pointer = response.hover_pos();
    let expanded = rect.expand(60.0);

    {
        let world_min = screen_to_world(expanded.min);
        let world_max = screen_to_world(expanded.max);
        let cx_min = (world_min.x / GRID_CELL).floor() as i32;
        let cy_min = (world_min.y / GRID_CELL).floor() as i32;
        let cx_max = (world_max.x / GRID_CELL).floor() as i32;
        let cy_max = (world_max.y / GRID_CELL).floor() as i32;

        for gx in cx_min..=cx_max {
            for gy in cy_min..=cy_max {
                let Some(nodes) = state.grid.cells.get(&(gx, gy)) else {
                    continue;
                };
                for &node_idx in nodes {
                    if !visible.contains(&node_idx) {
                        continue;
                    }
                    let Some(&world_pos) = state.positions.get(&node_idx) else {
                        continue;
                    };
                    let screen_pos = world_to_screen(world_pos);
                    let radius = node_radius(&graph[node_idx], data) * zoom;
                    if pointer.map_or(false, |p| p.distance(screen_pos) < radius + 3.0) {
                        state.hovered = Some(node_idx);
                    }
                }
            }
        }
    }

    // --- Compute dimming contexts ---
    let hover_neighbors: Option<HashSet<NodeIndex>> = state
        .hovered
        .map(|h| collect_neighbors(graph, h));

    let focus_set: Option<HashSet<NodeIndex>> = state
        .focus_node
        .filter(|n| visible.contains(n))
        .map(|n| compute_ego_set(graph, n, state.focus_depth, &visible));

    // --- Draw cluster rectangles (Grouped layout only) ---
    for cr in &state.cluster_rects {
        let tl = world_to_screen(Vec2::new(cr.x, cr.y));
        let br = world_to_screen(Vec2::new(cr.x + cr.w, cr.y + cr.h));
        let cluster_rect = egui::Rect::from_min_max(tl, br);
        painter.rect(
            cluster_rect,
            6.0,
            Color32::from_rgba_premultiplied(30, 35, 50, 120),
            Stroke::new(1.0, Color32::from_rgb(70, 80, 100)),
        );
        painter.text(
            Pos2::new(tl.x + 6.0, tl.y + 4.0),
            egui::Align2::LEFT_TOP,
            &cr.label,
            FontId::proportional(11.0 * zoom.sqrt()),
            Color32::from_rgb(120, 130, 160),
        );
    }

    // --- Draw edges ---
    for edge_idx in graph.edge_indices() {
        let edge = &graph[edge_idx];
        if !state.edge_filters.get(&edge.kind).copied().unwrap_or(true) {
            continue;
        }
        let Some((src, tgt)) = graph.edge_endpoints(edge_idx) else {
            continue;
        };
        if !visible.contains(&src) || !visible.contains(&tgt) {
            continue;
        }
        let (Some(&sp), Some(&tp)) = (state.positions.get(&src), state.positions.get(&tgt))
        else {
            continue;
        };

        let from = world_to_screen(sp);
        let to = world_to_screen(tp);

        if !expanded.contains(from) && !expanded.contains(to) {
            continue;
        }

        let src_alpha = compute_node_alpha(
            src, graph, state, data, entrypoints, &hover_neighbors, &focus_set,
        );
        let tgt_alpha = compute_node_alpha(
            tgt, graph, state, data, entrypoints, &hover_neighbors, &focus_set,
        );
        let edge_alpha = src_alpha.min(tgt_alpha);

        let both_in_cycle = state.cycle_nodes.contains(&src)
            && state.cycle_nodes.contains(&tgt);
        let is_hover_connected = state.hovered
            .map_or(false, |h| h == src || h == tgt);

        let base_color = if both_in_cycle && is_hover_connected {
            Color32::from_rgb(255, 60, 60)
        } else if is_hover_connected {
            edge_color(&edge.kind)
        } else {
            Color32::from_rgb(60, 60, 60)
        };
        let weight_alpha = (60.0 + (edge.weight.min(5.0) / 5.0) * 160.0) / 255.0;
        let final_alpha = edge_alpha * weight_alpha;
        let color = with_alpha(base_color, final_alpha);

        let base_width = if both_in_cycle {
            2.5
        } else if edge.kind == RelationshipKind::Contains {
            0.5
        } else {
            1.0 + edge.weight.ln().max(0.0) * 0.5
        };
        let width = base_width.min(6.0) * zoom.sqrt();

        let dir = to - from;
        let dist = dir.length();
        if dist < 2.0 {
            continue;
        }
        let dir_n = dir / dist;

        let src_r = node_radius(&graph[src], data) * zoom;
        let tgt_r = node_radius(&graph[tgt], data) * zoom;
        let line_start = from + dir_n * src_r;
        let tip = to - dir_n * tgt_r;

        // Check for parallel edges and apply curvature
        let parallel_count = graph
            .edges_connecting(src, tgt)
            .chain(graph.edges_connecting(tgt, src))
            .count();
        if parallel_count > 1 {
            let edge_ordinal = graph
                .edges_connecting(src, tgt)
                .chain(graph.edges_connecting(tgt, src))
                .position(|e| e.id() == edge_idx)
                .unwrap_or(0);
            let offset = (edge_ordinal as f32 - (parallel_count as f32 - 1.0) / 2.0) * 12.0 * zoom;
            let perp = egui::vec2(-dir_n.y, dir_n.x);
            let mid = Pos2::new(
                (line_start.x + tip.x) / 2.0 + perp.x * offset,
                (line_start.y + tip.y) / 2.0 + perp.y * offset,
            );
            let cp1 = Pos2::new(
                line_start.x + (mid.x - line_start.x) * 0.5,
                line_start.y + (mid.y - line_start.y) * 0.5,
            );
            let cp2 = Pos2::new(
                tip.x + (mid.x - tip.x) * 0.5,
                tip.y + (mid.y - tip.y) * 0.5,
            );
            painter.add(Shape::CubicBezier(egui::epaint::CubicBezierShape::from_points_stroke(
                [line_start, cp1, cp2, tip],
                false,
                Color32::TRANSPARENT,
                Stroke::new(width, color),
            )));
        } else {
            painter.line_segment([line_start, tip], Stroke::new(width, color));
        }

        // Arrowheads only when the edge or endpoints are hovered
        let show_arrow = state.hovered == Some(src)
            || state.hovered == Some(tgt)
            || hover_neighbors.as_ref().map_or(false, |hn| hn.contains(&src) && hn.contains(&tgt));
        if show_arrow && dist > 40.0 * zoom {
            let perp = egui::vec2(-dir_n.y, dir_n.x);
            let arrow = 7.0 * zoom.sqrt();
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

    // --- Draw nodes via spatial grid ---
    let world_min = screen_to_world(expanded.min);
    let world_max = screen_to_world(expanded.max);
    let cx_min = (world_min.x / GRID_CELL).floor() as i32;
    let cy_min = (world_min.y / GRID_CELL).floor() as i32;
    let cx_max = (world_max.x / GRID_CELL).floor() as i32;
    let cy_max = (world_max.y / GRID_CELL).floor() as i32;

    for gx in cx_min..=cx_max {
        for gy in cy_min..=cy_max {
            let Some(nodes) = state.grid.cells.get(&(gx, gy)) else {
                continue;
            };
            for &node_idx in nodes {
                if !visible.contains(&node_idx) {
                    continue;
                }
                let Some(&world_pos) = state.positions.get(&node_idx) else {
                    continue;
                };
                let screen_pos = world_to_screen(world_pos);
                if !expanded.contains(screen_pos) {
                    continue;
                }

                let node = &graph[node_idx];
                let radius = node_radius(node, data) * zoom;
                let base_color = if let Some(&coupling) = state.coupling_heatmap.get(&node_idx) {
                    coupling_to_color(coupling)
                } else {
                    node_color(node, data)
                };
                let is_hovered = state.hovered == Some(node_idx);
                let is_selected = state.selected == Some(node_idx);
                let is_pinned = state.pinned_nodes.contains(&node_idx);
                let is_entry =
                    matches!(node, GraphNode::Symbol(sid) if entrypoints.contains(sid));
                let is_in_cycle = state.cycle_nodes.contains(&node_idx);

                let alpha = compute_node_alpha(
                    node_idx, graph, state, data, entrypoints, &hover_neighbors, &focus_set,
                );

                if is_in_cycle {
                    painter.circle_filled(
                        screen_pos,
                        radius + 5.0 * zoom,
                        with_alpha(Color32::from_rgb(255, 50, 50), alpha * 0.25),
                    );
                    painter.circle_stroke(
                        screen_pos,
                        radius + 3.0 * zoom,
                        Stroke::new(2.0 * zoom.sqrt(), with_alpha(Color32::from_rgb(255, 60, 60), alpha)),
                    );
                }
                if is_entry {
                    painter.circle_filled(
                        screen_pos,
                        radius + 4.0 * zoom,
                        with_alpha(Color32::from_rgb(171, 240, 18), alpha * 0.2),
                    );
                }
                if is_selected {
                    painter.circle_stroke(
                        screen_pos,
                        radius + 2.5,
                        Stroke::new(2.0, Color32::WHITE),
                    );
                }
                if is_pinned {
                    painter.circle_stroke(
                        screen_pos,
                        radius + 3.5,
                        Stroke::new(1.5, Color32::from_rgb(171, 240, 18)),
                    );
                }
                let color = with_alpha(
                    if is_hovered { lighten(base_color, 35) } else { base_color },
                    alpha,
                );
                painter.circle_filled(screen_pos, radius, color);

                if zoom > 0.35 {
                    let font =
                        FontId::proportional((10.0 * zoom.sqrt()).clamp(7.0, 16.0));
                    let text_pos = Pos2::new(screen_pos.x, screen_pos.y + radius + 3.0);
                    let label = state
                        .label_cache
                        .get(&node_idx)
                        .map_or("?", |s| s.as_str());
                    painter.text(
                        text_pos,
                        egui::Align2::CENTER_TOP,
                        label,
                        font,
                        with_alpha(Color32::from_rgb(220, 220, 220), alpha),
                    );
                }
            }
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
                *pos += response.drag_delta() / zoom;
                if let Some(ref mut layout) = state.layout {
                    layout.update_position(dragging, *pos);
                }
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
            return ClickResult::NodeClicked(hovered);
        } else {
            state.selected = None;
            return ClickResult::BackgroundClicked;
        }
    }

    // --- Interaction: secondary click (right click) for pin toggle ---
    if response.secondary_clicked() {
        if let Some(hovered) = state.hovered {
            if !state.pinned_nodes.remove(&hovered) {
                state.pinned_nodes.insert(hovered);
            }
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

/// Map a coupling score to a blue-to-red heatmap colour.
/// 0 = cool blue, 10 = neutral, 20+ = hot red.
fn coupling_to_color(score: f32) -> Color32 {
    let t = (score / 25.0).clamp(0.0, 1.0);
    let r = (t * 255.0) as u8;
    let b = ((1.0 - t) * 200.0) as u8;
    let g = ((1.0 - (2.0 * t - 1.0).abs()) * 100.0) as u8;
    Color32::from_rgb(r, g, b)
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
            .crate_index
            .get(id)
            .and_then(|&i| data.crates.get(i))
            .map_or("?".into(), |c| c.name.clone()),
        GraphNode::Module(id) => data
            .modules
            .get(id)
            .map_or("?".into(), |m| m.name.clone()),
        GraphNode::File(id) => data
            .file_index
            .get(id)
            .and_then(|&i| data.files.get(i))
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
        GraphNode::Crate(id) => {
            let module_count = data
                .crate_index
                .get(id)
                .and_then(|&i| data.crates.get(i))
                .map_or(1, |c| c.module_ids.len()) as f32;
            (12.0 + module_count.sqrt() * 3.0).min(30.0)
        }
        GraphNode::Module(id) => {
            let sym_count = data.modules.get(id).map_or(1, |m| m.symbol_ids.len()) as f32;
            (8.0 + sym_count.sqrt() * 2.0).min(22.0)
        }
        GraphNode::Symbol(id) => {
            let fan_in = data
                .analysis
                .symbol_metrics
                .get(id)
                .map_or(0, |m| m.fan_in) as f32;
            (5.0 + fan_in.sqrt() * 2.5).min(24.0)
        }
        GraphNode::File(_) => 7.0,
    }
}

/// Compute zoom and pan so that all positioned nodes fit within the viewport with padding.
fn auto_fit_viewport(state: &mut GraphViewState, viewport_w: f32, viewport_h: f32) {
    if state.positions.is_empty() {
        return;
    }
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for pos in state.positions.values() {
        min_x = min_x.min(pos.x);
        min_y = min_y.min(pos.y);
        max_x = max_x.max(pos.x);
        max_y = max_y.max(pos.y);
    }

    let content_w = (max_x - min_x).max(1.0);
    let content_h = (max_y - min_y).max(1.0);
    let padding = 60.0;

    let zoom_x = (viewport_w - padding * 2.0) / content_w;
    let zoom_y = (viewport_h - padding * 2.0) / content_h;
    state.zoom = zoom_x.min(zoom_y).clamp(0.15, 2.0);

    let center_x = (min_x + max_x) / 2.0;
    let center_y = (min_y + max_y) / 2.0;
    state.pan = Vec2::new(
        viewport_w / 2.0 - center_x * state.zoom,
        viewport_h / 2.0 - center_y * state.zoom,
    );
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
            if let Some(c) = data.crate_index.get(id).and_then(|&i| data.crates.get(i)) {
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
            if let Some(f) = data.file_index.get(id).and_then(|&i| data.files.get(i)) {
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
