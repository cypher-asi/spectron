//! Layout algorithms: force-directed (Fruchterman-Reingold), layered (Sugiyama),
//! and grouped (cluster-by-parent).

use std::collections::{HashMap, HashSet};

use egui::Vec2;
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use spectron_core::{ArchGraph, GraphNode, RelationshipKind};

const THETA: f32 = 0.8;
const MAX_ITERATIONS: usize = 300;
const ITERATIONS_PER_STEP: usize = 10;

/// Extra multiplier on ideal distance `k` so nodes spread further apart.
const SPREAD_FACTOR: f32 = 2.2;

/// Boundary margin as a fraction of the layout dimension.
const MARGIN_FRAC: f32 = 0.03;

// ---------------------------------------------------------------------------
// Quadtree for Barnes-Hut repulsion approximation
// ---------------------------------------------------------------------------

struct QuadTree {
    cx: f32,
    cy: f32,
    half: f32,
    com_x: f32,
    com_y: f32,
    count: u32,
    children: Option<Box<[QuadTree; 4]>>,
    body: Option<usize>,
}

impl QuadTree {
    fn empty(cx: f32, cy: f32, half: f32) -> Self {
        Self {
            cx,
            cy,
            half,
            com_x: 0.0,
            com_y: 0.0,
            count: 0,
            children: None,
            body: None,
        }
    }

    fn build(positions: &[Vec2]) -> Self {
        if positions.is_empty() {
            return Self::empty(0.0, 0.0, 1.0);
        }

        let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
        let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
        for p in positions {
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        }

        let half = ((max_x - min_x).max(max_y - min_y) / 2.0).max(1.0) + 1.0;
        let cx = (min_x + max_x) / 2.0;
        let cy = (min_y + max_y) / 2.0;

        let mut tree = Self::empty(cx, cy, half);
        for (i, p) in positions.iter().enumerate() {
            tree.insert(i, p.x, p.y, positions);
        }
        tree
    }

    fn quadrant(&self, x: f32, y: f32) -> usize {
        let ew = usize::from(x >= self.cx);
        let ns = usize::from(y >= self.cy) << 1;
        ew | ns
    }

    fn child_params(&self, q: usize) -> (f32, f32, f32) {
        let h = self.half * 0.5;
        let dx = if q & 1 != 0 { h } else { -h };
        let dy = if q & 2 != 0 { h } else { -h };
        (self.cx + dx, self.cy + dy, h)
    }

    fn insert(&mut self, idx: usize, x: f32, y: f32, positions: &[Vec2]) {
        let total = self.count as f32 + 1.0;
        self.com_x = (self.com_x * self.count as f32 + x) / total;
        self.com_y = (self.com_y * self.count as f32 + y) / total;
        self.count += 1;

        if self.count == 1 {
            self.body = Some(idx);
            return;
        }

        if self.half < 0.5 {
            return;
        }

        if self.children.is_none() {
            let (c0x, c0y, h) = self.child_params(0);
            let (c1x, c1y, _) = self.child_params(1);
            let (c2x, c2y, _) = self.child_params(2);
            let (c3x, c3y, _) = self.child_params(3);
            self.children = Some(Box::new([
                Self::empty(c0x, c0y, h),
                Self::empty(c1x, c1y, h),
                Self::empty(c2x, c2y, h),
                Self::empty(c3x, c3y, h),
            ]));

            if let Some(old) = self.body.take() {
                let op = positions[old];
                let q = self.quadrant(op.x, op.y);
                self.children.as_mut().unwrap()[q].insert(old, op.x, op.y, positions);
            }
        }

        let q = self.quadrant(x, y);
        self.children.as_mut().unwrap()[q].insert(idx, x, y, positions);
    }

    fn apply_repulsion(&self, idx: usize, positions: &[Vec2], k_sq: f32, disp: &mut Vec2) {
        if self.count == 0 {
            return;
        }

        let pos = positions[idx];

        if self.count == 1 {
            if self.body == Some(idx) {
                return;
            }
            let dx = pos.x - self.com_x;
            let dy = pos.y - self.com_y;
            let dist_sq = (dx * dx + dy * dy).max(1.0);
            let f = k_sq / dist_sq;
            disp.x += dx / dist_sq.sqrt() * f;
            disp.y += dy / dist_sq.sqrt() * f;
            return;
        }

        let dx = pos.x - self.com_x;
        let dy = pos.y - self.com_y;
        let dist_sq = (dx * dx + dy * dy).max(1.0);
        let width = self.half * 2.0;

        if width * width < THETA * THETA * dist_sq {
            let dist = dist_sq.sqrt();
            let f = k_sq * self.count as f32 / dist_sq;
            disp.x += dx / dist * f;
            disp.y += dy / dist * f;
            return;
        }

        if let Some(ref children) = self.children {
            for child in children.iter() {
                child.apply_repulsion(idx, positions, k_sq, disp);
            }
        } else {
            let dist = dist_sq.sqrt();
            let f = k_sq * self.count as f32 / dist_sq;
            disp.x += dx / dist * f;
            disp.y += dy / dist * f;
        }
    }
}

// ---------------------------------------------------------------------------
// Incremental layout state
// ---------------------------------------------------------------------------

/// Persistent layout state that can be advanced a few iterations per frame.
pub struct LayoutState {
    nodes: Vec<NodeIndex>,
    positions: Vec<Vec2>,
    disp: Vec<Vec2>,
    node_to_idx: HashMap<NodeIndex, usize>,
    k: f32,
    temperature: f32,
    cooling: f32,
    iteration: usize,
    width: f32,
    height: f32,
    pub done: bool,
}

impl LayoutState {
    pub fn new(graph: &ArchGraph, width: f32, height: f32) -> Self {
        let mut nodes: Vec<NodeIndex> = graph.node_indices().collect();
        nodes.sort(); // deterministic ordering
        Self::init(nodes, width, height)
    }

    pub fn new_filtered(
        graph: &ArchGraph,
        width: f32,
        height: f32,
        visible: &HashSet<NodeIndex>,
    ) -> Self {
        let mut nodes: Vec<NodeIndex> = graph
            .node_indices()
            .filter(|ni| visible.contains(ni))
            .collect();
        nodes.sort(); // deterministic ordering
        Self::init(nodes, width, height)
    }

    fn init(nodes: Vec<NodeIndex>, width: f32, height: f32) -> Self {
        let n = nodes.len();
        let node_to_idx: HashMap<NodeIndex, usize> = nodes
            .iter()
            .enumerate()
            .map(|(i, &ni)| (ni, i))
            .collect();

        let area = width * height;
        let k = if n > 0 {
            (area / n as f32).sqrt() * SPREAD_FACTOR
        } else {
            1.0
        };

        let n_f = n.max(1) as f32;
        let mut positions = Vec::with_capacity(n);
        for (i, _) in nodes.iter().enumerate() {
            let angle = 2.0 * std::f32::consts::PI * i as f32 / n_f;
            let r = k * 1.5 * (i as f32 + 1.0).sqrt();
            positions.push(Vec2::new(
                width / 2.0 + r * angle.cos(),
                height / 2.0 + r * angle.sin(),
            ));
        }

        let temperature = width.min(height) / 2.0;
        let cooling = temperature / MAX_ITERATIONS as f32;

        Self {
            nodes,
            disp: vec![Vec2::ZERO; n],
            positions,
            node_to_idx,
            k,
            temperature,
            cooling,
            iteration: 0,
            width,
            height,
            done: n == 0,
        }
    }

    /// Run a fixed budget of layout iterations. Returns `true` when complete.
    pub fn step(&mut self, graph: &ArchGraph) -> bool {
        if self.done {
            return true;
        }

        let n = self.positions.len();
        let budget = ITERATIONS_PER_STEP.min(MAX_ITERATIONS.saturating_sub(self.iteration));
        let k = self.k;
        let k_sq = k * k;

        for _ in 0..budget {
            for d in self.disp.iter_mut() {
                *d = Vec2::ZERO;
            }

            let tree = QuadTree::build(&self.positions);
            for i in 0..n {
                tree.apply_repulsion(i, &self.positions, k_sq, &mut self.disp[i]);
            }

            for edge_idx in graph.edge_indices() {
                if let Some((a, b)) = graph.edge_endpoints(edge_idx) {
                    let (Some(&ai), Some(&bi)) =
                        (self.node_to_idx.get(&a), self.node_to_idx.get(&b))
                    else {
                        continue;
                    };
                    let delta = self.positions[ai] - self.positions[bi];
                    let dist = delta.length().max(1.0);
                    let weight = graph[edge_idx].weight;
                    let attraction = dist * dist / k * weight;
                    let force = delta / dist * attraction;
                    self.disp[ai] -= force;
                    self.disp[bi] += force;
                }
            }

            let mx = self.width * MARGIN_FRAC;
            let my = self.height * MARGIN_FRAC;
            for i in 0..n {
                let len = self.disp[i].length().max(0.001);
                let bounded = self.disp[i] / len * len.min(self.temperature);
                self.positions[i] += bounded;
                self.positions[i].x = self.positions[i].x.clamp(mx, self.width - mx);
                self.positions[i].y = self.positions[i].y.clamp(my, self.height - my);
            }

            self.temperature -= self.cooling;
            self.iteration += 1;

            if self.temperature < 0.1 || self.iteration >= MAX_ITERATIONS {
                self.done = true;
                break;
            }
        }

        self.done
    }

    /// Build a `HashMap` snapshot of current positions.
    pub fn to_position_map(&self) -> HashMap<NodeIndex, Vec2> {
        self.nodes
            .iter()
            .enumerate()
            .map(|(i, &ni)| (ni, self.positions[i]))
            .collect()
    }

    /// Update a single node's position (e.g. after user drag).
    pub fn update_position(&mut self, node: NodeIndex, pos: Vec2) {
        if let Some(&idx) = self.node_to_idx.get(&node) {
            self.positions[idx] = pos;
        }
    }
}

/// Compute a complete layout in one blocking call.
pub fn compute_layout(graph: &ArchGraph, width: f32, height: f32) -> HashMap<NodeIndex, Vec2> {
    let mut state = LayoutState::new(graph, width, height);
    while !state.step(graph) {}
    state.to_position_map()
}

// ---------------------------------------------------------------------------
// Sugiyama (layered) layout — improved
// ---------------------------------------------------------------------------

const BARYCENTER_SWEEPS: usize = 10;

/// Compute a layered (Sugiyama-style) layout for the visible subset of the
/// graph.
///
/// Improvements over the original implementation:
/// 1. **Cycle-breaking** — DFS identifies back-edges and temporarily reverses
///    them so Kahn's algorithm can produce a proper topological layer
///    assignment even when cycles exist.
/// 2. **Deterministic ordering** — nodes are sorted by `NodeIndex` before
///    processing so the same graph always yields the same layout.
/// 3. **More barycenter sweeps** (10 instead of 3) for better crossing
///    minimisation.
/// 4. **Stable tie-breaking** — when two nodes have the same barycenter, they
///    keep their current relative order (preserving determinism across sweeps).
pub fn compute_layered_layout(
    graph: &ArchGraph,
    width: f32,
    height: f32,
    visible: &HashSet<NodeIndex>,
) -> HashMap<NodeIndex, Vec2> {
    use petgraph::Direction;

    // Collect visible nodes in deterministic (ascending NodeIndex) order.
    let mut nodes: Vec<NodeIndex> = graph
        .node_indices()
        .filter(|n| visible.contains(n))
        .collect();
    nodes.sort();

    if nodes.is_empty() {
        return HashMap::new();
    }

    let node_set: HashSet<NodeIndex> = nodes.iter().copied().collect();

    // ------------------------------------------------------------------
    // Phase 1: cycle breaking via DFS back-edge detection
    // ------------------------------------------------------------------
    // We build an adjacency list restricted to visible nodes, then perform
    // DFS. Back-edges are "reversed" in a separate set so that the layer
    // assignment sees a DAG.

    let mut adj: HashMap<NodeIndex, Vec<NodeIndex>> = HashMap::new();
    for &n in &nodes {
        let mut neighbors: Vec<NodeIndex> = graph
            .neighbors_directed(n, Direction::Outgoing)
            .filter(|nb| node_set.contains(nb))
            .collect();
        neighbors.sort(); // deterministic traversal
        adj.insert(n, neighbors);
    }

    let mut reversed_edges: HashSet<(NodeIndex, NodeIndex)> = HashSet::new();
    {
        let mut color: HashMap<NodeIndex, u8> = HashMap::new(); // 0=white, 1=gray, 2=black
        for &n in &nodes {
            color.insert(n, 0);
        }
        let mut stack: Vec<(NodeIndex, usize)> = Vec::new();
        for &root in &nodes {
            if color[&root] != 0 {
                continue;
            }
            stack.push((root, 0));
            color.insert(root, 1);
            while let Some((node, idx)) = stack.last_mut() {
                let neighbors = &adj[node];
                if *idx < neighbors.len() {
                    let nb = neighbors[*idx];
                    *idx += 1;
                    match color[&nb] {
                        0 => {
                            color.insert(nb, 1);
                            stack.push((nb, 0));
                        }
                        1 => {
                            // Back-edge — mark for reversal.
                            reversed_edges.insert((*node, nb));
                        }
                        _ => {}
                    }
                } else {
                    color.insert(*node, 2);
                    stack.pop();
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Phase 2: topological layer assignment (Kahn's on the virtual DAG)
    // ------------------------------------------------------------------

    let mut in_degree: HashMap<NodeIndex, usize> = HashMap::new();
    for &n in &nodes {
        in_degree.insert(n, 0);
    }
    for &n in &nodes {
        for &nb in &adj[&n] {
            if reversed_edges.contains(&(n, nb)) {
                // Reversed edge: treat as nb -> n for layering.
                *in_degree.entry(n).or_default() += 1;
            } else {
                *in_degree.entry(nb).or_default() += 1;
            }
        }
    }

    // Priority queue ensures deterministic ordering among equal-degree nodes.
    let mut queue: std::collections::BinaryHeap<std::cmp::Reverse<NodeIndex>> =
        std::collections::BinaryHeap::new();
    for &n in &nodes {
        if in_degree[&n] == 0 {
            queue.push(std::cmp::Reverse(n));
        }
    }

    let mut layer_of: HashMap<NodeIndex, usize> = HashMap::new();
    while let Some(std::cmp::Reverse(current)) = queue.pop() {
        let current_layer = layer_of.get(&current).copied().unwrap_or(0);
        for &nb in &adj[&current] {
            let (src, tgt) = if reversed_edges.contains(&(current, nb)) {
                (nb, current) // virtual direction
            } else {
                (current, nb)
            };
            if src == current {
                let new_layer = current_layer + 1;
                let entry = layer_of.entry(tgt).or_insert(0);
                if new_layer > *entry {
                    *entry = new_layer;
                }
                let deg = in_degree.get_mut(&tgt).unwrap();
                *deg = deg.saturating_sub(1);
                if *deg == 0 {
                    queue.push(std::cmp::Reverse(tgt));
                }
            }
        }
        layer_of.entry(current).or_insert(current_layer);
    }

    // Safety net: assign any remaining unvisited nodes (shouldn't happen
    // with correct cycle-breaking, but be defensive).
    for &n in &nodes {
        layer_of.entry(n).or_insert(0);
    }

    // ------------------------------------------------------------------
    // Phase 3: build layer vectors
    // ------------------------------------------------------------------

    let max_layer = layer_of.values().copied().max().unwrap_or(0);
    let mut layers: Vec<Vec<NodeIndex>> = vec![Vec::new(); max_layer + 1];
    for &n in &nodes {
        layers[layer_of[&n]].push(n);
    }
    // Deterministic initial order within each layer.
    for layer in &mut layers {
        layer.sort();
    }

    // ------------------------------------------------------------------
    // Phase 4: barycenter ordering (10 forward+backward sweeps)
    // ------------------------------------------------------------------

    for _sweep in 0..BARYCENTER_SWEEPS {
        // Forward sweep (top → bottom)
        for l in 1..layers.len() {
            let prev_positions: HashMap<NodeIndex, f32> = layers[l - 1]
                .iter()
                .enumerate()
                .map(|(i, &n)| (n, i as f32))
                .collect();

            let mut indexed: Vec<(usize, NodeIndex, f32)> = layers[l]
                .iter()
                .enumerate()
                .map(|(orig_idx, &n)| {
                    let mut sum = 0.0_f32;
                    let mut count = 0_u32;
                    for neighbor in graph.neighbors_directed(n, Direction::Incoming) {
                        if let Some(&pos) = prev_positions.get(&neighbor) {
                            sum += pos;
                            count += 1;
                        }
                    }
                    let bc = if count > 0 { sum / count as f32 } else { orig_idx as f32 };
                    (orig_idx, n, bc)
                })
                .collect();
            indexed.sort_by(|a, b| {
                a.2.partial_cmp(&b.2)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.0.cmp(&b.0)) // stable tie-break
            });
            layers[l] = indexed.into_iter().map(|(_, n, _)| n).collect();
        }

        // Backward sweep (bottom → top)
        for l in (0..layers.len().saturating_sub(1)).rev() {
            let next_positions: HashMap<NodeIndex, f32> = layers[l + 1]
                .iter()
                .enumerate()
                .map(|(i, &n)| (n, i as f32))
                .collect();

            let mut indexed: Vec<(usize, NodeIndex, f32)> = layers[l]
                .iter()
                .enumerate()
                .map(|(orig_idx, &n)| {
                    let mut sum = 0.0_f32;
                    let mut count = 0_u32;
                    for neighbor in graph.neighbors_directed(n, Direction::Outgoing) {
                        if let Some(&pos) = next_positions.get(&neighbor) {
                            sum += pos;
                            count += 1;
                        }
                    }
                    let bc = if count > 0 { sum / count as f32 } else { orig_idx as f32 };
                    (orig_idx, n, bc)
                })
                .collect();
            indexed.sort_by(|a, b| {
                a.2.partial_cmp(&b.2)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.0.cmp(&b.0))
            });
            layers[l] = indexed.into_iter().map(|(_, n, _)| n).collect();
        }
    }

    // ------------------------------------------------------------------
    // Phase 5: position assignment
    // ------------------------------------------------------------------

    let num_layers = layers.len().max(1);
    let margin_x = width * 0.06;
    let margin_y = height * 0.06;
    let usable_w = width - 2.0 * margin_x;
    let usable_h = height - 2.0 * margin_y;
    let layer_spacing = usable_h / num_layers.max(2) as f32;

    let mut positions = HashMap::new();
    for (layer_idx, layer) in layers.iter().enumerate() {
        let count = layer.len().max(1);
        let node_spacing = usable_w / count as f32;
        for (pos_idx, &n) in layer.iter().enumerate() {
            let x = margin_x + (pos_idx as f32 + 0.5) * node_spacing;
            let y = margin_y + (layer_idx as f32 + 0.5) * layer_spacing;
            positions.insert(n, Vec2::new(x, y));
        }
    }

    positions
}

// ---------------------------------------------------------------------------
// Grouped (cluster-by-parent) layout
// ---------------------------------------------------------------------------

/// Cluster padding within each group.
const CLUSTER_INNER_PAD: f32 = 30.0;
/// Padding between clusters.
const CLUSTER_GAP: f32 = 80.0;

/// Compute a grouped layout that clusters nodes by their parent
/// (crate or module) based on `Contains` edges.
///
/// Nodes without a parent in the visible set are placed in a special
/// "ungrouped" cluster. Within each cluster, nodes are arranged in a
/// grid. Clusters themselves are arranged in a row-wrapped layout.
///
/// This layout is deterministic: same graph always produces the same
/// positions.
pub fn compute_grouped_layout(
    graph: &ArchGraph,
    width: f32,
    _height: f32,
    visible: &HashSet<NodeIndex>,
) -> (HashMap<NodeIndex, Vec2>, Vec<ClusterRect>) {
    let mut nodes: Vec<NodeIndex> = graph
        .node_indices()
        .filter(|n| visible.contains(n))
        .collect();
    nodes.sort();

    if nodes.is_empty() {
        return (HashMap::new(), Vec::new());
    }

    let node_set: HashSet<NodeIndex> = nodes.iter().copied().collect();

    // Build parent mapping: child -> parent via incoming Contains edge.
    let mut parent_of: HashMap<NodeIndex, NodeIndex> = HashMap::new();
    for &n in &nodes {
        for edge in graph.edges_directed(n, petgraph::Direction::Incoming) {
            if edge.weight().kind == RelationshipKind::Contains && node_set.contains(&edge.source()) {
                parent_of.insert(n, edge.source());
                break;
            }
        }
    }

    // Group nodes by their parent. Roots (no parent) are cluster heads.
    // Nodes whose parent is also visible are grouped under that parent.
    let mut clusters: HashMap<Option<NodeIndex>, Vec<NodeIndex>> = HashMap::new();
    for &n in &nodes {
        let group_key = parent_of.get(&n).copied();
        clusters.entry(group_key).or_default().push(n);
    }

    // Sort cluster keys deterministically (None first, then by NodeIndex).
    let mut cluster_keys: Vec<Option<NodeIndex>> = clusters.keys().copied().collect();
    cluster_keys.sort_by_key(|k| k.map(|ni| ni.index()).unwrap_or(0));

    // Layout each cluster, then arrange clusters in a row-wrapped grid.
    let margin = width * 0.04;
    let max_row_width = width - 2.0 * margin;

    let node_size = 24.0_f32;
    let mut positions = HashMap::new();
    let mut cluster_rects: Vec<ClusterRect> = Vec::new();
    let mut cursor_x = margin;
    let mut cursor_y = margin;
    let mut row_max_height = 0.0_f32;

    for key in &cluster_keys {
        let members = &clusters[key];
        let count = members.len();
        let cols = (count as f32).sqrt().ceil().max(1.0) as usize;
        let rows = (count + cols - 1) / cols;

        let cluster_w = cols as f32 * (node_size + CLUSTER_INNER_PAD) + CLUSTER_INNER_PAD;
        let cluster_h = rows as f32 * (node_size + CLUSTER_INNER_PAD) + CLUSTER_INNER_PAD;

        // Wrap to next row if this cluster doesn't fit.
        if cursor_x + cluster_w > max_row_width + margin && cursor_x > margin + 1.0 {
            cursor_x = margin;
            cursor_y += row_max_height + CLUSTER_GAP;
            row_max_height = 0.0;
        }

        // Record the cluster rectangle for rendering.
        let label = key
            .and_then(|ni| {
                match &graph[ni] {
                    GraphNode::Crate(id) => Some(format!("Crate({})", id)),
                    GraphNode::Module(id) => Some(format!("Module({})", id)),
                    _ => None,
                }
            })
            .unwrap_or_else(|| "ungrouped".to_string());

        cluster_rects.push(ClusterRect {
            x: cursor_x,
            y: cursor_y,
            w: cluster_w,
            h: cluster_h,
            label,
        });

        // Place members in a grid within the cluster.
        for (i, &n) in members.iter().enumerate() {
            let col = i % cols;
            let row = i / cols;
            let x = cursor_x + CLUSTER_INNER_PAD + col as f32 * (node_size + CLUSTER_INNER_PAD);
            let y = cursor_y + CLUSTER_INNER_PAD + row as f32 * (node_size + CLUSTER_INNER_PAD);
            positions.insert(n, Vec2::new(x, y));
        }

        cursor_x += cluster_w + CLUSTER_GAP;
        row_max_height = row_max_height.max(cluster_h);
    }

    (positions, cluster_rects)
}

/// Rectangle describing a cluster boundary for rendering.
#[derive(Clone, Debug)]
pub struct ClusterRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub label: String,
}
