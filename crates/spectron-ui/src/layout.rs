//! Fruchterman-Reingold force-directed layout with Barnes-Hut O(n log n) repulsion.

use std::collections::{HashMap, HashSet};

use egui::Vec2;
use petgraph::graph::NodeIndex;
use spectron_core::ArchGraph;

const THETA: f32 = 0.8;
const MAX_ITERATIONS: usize = 200;
const ITERATIONS_PER_STEP: usize = 8;

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
        let nodes: Vec<NodeIndex> = graph.node_indices().collect();
        Self::init(nodes, width, height)
    }

    pub fn new_filtered(
        graph: &ArchGraph,
        width: f32,
        height: f32,
        visible: &HashSet<NodeIndex>,
    ) -> Self {
        let nodes: Vec<NodeIndex> = graph
            .node_indices()
            .filter(|ni| visible.contains(ni))
            .collect();
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
            (area / n as f32).sqrt()
        } else {
            1.0
        };

        let n_f = n.max(1) as f32;
        let mut positions = Vec::with_capacity(n);
        for (i, _) in nodes.iter().enumerate() {
            let angle = 2.0 * std::f32::consts::PI * i as f32 / n_f;
            let r = k * (i as f32 + 1.0).sqrt();
            positions.push(Vec2::new(
                width / 2.0 + r * angle.cos(),
                height / 2.0 + r * angle.sin(),
            ));
        }

        let temperature = width.min(height) / 4.0;
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

            for i in 0..n {
                let len = self.disp[i].length().max(0.001);
                let bounded = self.disp[i] / len * len.min(self.temperature);
                self.positions[i] += bounded;
                self.positions[i].x = self.positions[i].x.clamp(30.0, self.width - 30.0);
                self.positions[i].y = self.positions[i].y.clamp(30.0, self.height - 30.0);
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
// Sugiyama (layered) layout
// ---------------------------------------------------------------------------

/// Compute a layered (Sugiyama) layout for the visible subset of the graph.
///
/// 1. Layer assignment via longest-path from roots.
/// 2. Ordering within layers via barycenter heuristic (3 sweeps).
/// 3. Position assignment with even spacing.
pub fn compute_layered_layout(
    graph: &ArchGraph,
    width: f32,
    height: f32,
    visible: &HashSet<NodeIndex>,
) -> HashMap<NodeIndex, Vec2> {
    use petgraph::Direction;

    let nodes: Vec<NodeIndex> = graph
        .node_indices()
        .filter(|n| visible.contains(n))
        .collect();
    if nodes.is_empty() {
        return HashMap::new();
    }

    let node_set: HashSet<NodeIndex> = nodes.iter().copied().collect();

    // Layer assignment: topological longest-path from roots.
    // Roots = nodes with no incoming edges from visible set.
    let mut in_degree: HashMap<NodeIndex, usize> = HashMap::new();
    for &n in &nodes {
        in_degree.insert(n, 0);
    }
    for &n in &nodes {
        for neighbor in graph.neighbors_directed(n, Direction::Outgoing) {
            if node_set.contains(&neighbor) {
                *in_degree.entry(neighbor).or_default() += 1;
            }
        }
    }

    let mut layer_of: HashMap<NodeIndex, usize> = HashMap::new();
    let mut queue: std::collections::VecDeque<NodeIndex> = std::collections::VecDeque::new();

    for &n in &nodes {
        if in_degree[&n] == 0 {
            layer_of.insert(n, 0);
            queue.push_back(n);
        }
    }

    // Handle cycles: if no roots found, pick the first node as root.
    if queue.is_empty() {
        let root = nodes[0];
        layer_of.insert(root, 0);
        queue.push_back(root);
    }

    while let Some(current) = queue.pop_front() {
        let current_layer = layer_of[&current];
        for neighbor in graph.neighbors_directed(current, Direction::Outgoing) {
            if !node_set.contains(&neighbor) {
                continue;
            }
            let new_layer = current_layer + 1;
            let existing = layer_of.get(&neighbor).copied().unwrap_or(0);
            if new_layer > existing || !layer_of.contains_key(&neighbor) {
                let was_new = !layer_of.contains_key(&neighbor);
                layer_of.insert(neighbor, new_layer);
                if was_new {
                    queue.push_back(neighbor);
                }
            }
        }
    }

    // Assign unvisited nodes (from cycles) to layer 0.
    for &n in &nodes {
        layer_of.entry(n).or_insert(0);
    }

    let max_layer = layer_of.values().copied().max().unwrap_or(0);

    // Build layers.
    let mut layers: Vec<Vec<NodeIndex>> = vec![Vec::new(); max_layer + 1];
    for &n in &nodes {
        layers[layer_of[&n]].push(n);
    }

    // Barycenter ordering (3 sweeps).
    for _sweep in 0..3 {
        // Forward sweep
        for l in 1..layers.len() {
            let prev_positions: HashMap<NodeIndex, f32> = layers[l - 1]
                .iter()
                .enumerate()
                .map(|(i, &n)| (n, i as f32))
                .collect();

            let mut barycenters: Vec<(NodeIndex, f32)> = layers[l]
                .iter()
                .map(|&n| {
                    let mut sum = 0.0_f32;
                    let mut count = 0_u32;
                    for neighbor in graph.neighbors_directed(n, Direction::Incoming) {
                        if let Some(&pos) = prev_positions.get(&neighbor) {
                            sum += pos;
                            count += 1;
                        }
                    }
                    let bc = if count > 0 { sum / count as f32 } else { f32::MAX };
                    (n, bc)
                })
                .collect();
            barycenters.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            layers[l] = barycenters.into_iter().map(|(n, _)| n).collect();
        }

        // Backward sweep
        for l in (0..layers.len().saturating_sub(1)).rev() {
            let next_positions: HashMap<NodeIndex, f32> = layers[l + 1]
                .iter()
                .enumerate()
                .map(|(i, &n)| (n, i as f32))
                .collect();

            let mut barycenters: Vec<(NodeIndex, f32)> = layers[l]
                .iter()
                .map(|&n| {
                    let mut sum = 0.0_f32;
                    let mut count = 0_u32;
                    for neighbor in graph.neighbors_directed(n, Direction::Outgoing) {
                        if let Some(&pos) = next_positions.get(&neighbor) {
                            sum += pos;
                            count += 1;
                        }
                    }
                    let bc = if count > 0 { sum / count as f32 } else { f32::MAX };
                    (n, bc)
                })
                .collect();
            barycenters.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            layers[l] = barycenters.into_iter().map(|(n, _)| n).collect();
        }
    }

    // Position assignment.
    let num_layers = layers.len().max(1);
    let layer_spacing = (height - 80.0) / num_layers as f32;
    let margin = 40.0;

    let mut positions = HashMap::new();
    for (layer_idx, layer) in layers.iter().enumerate() {
        let count = layer.len().max(1);
        let node_spacing = (width - 2.0 * margin) / count as f32;
        for (pos_idx, &n) in layer.iter().enumerate() {
            let x = margin + (pos_idx as f32 + 0.5) * node_spacing;
            let y = margin + layer_idx as f32 * layer_spacing;
            positions.insert(n, Vec2::new(x, y));
        }
    }

    positions
}
