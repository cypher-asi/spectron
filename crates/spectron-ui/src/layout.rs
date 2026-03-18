//! Fruchterman-Reingold force-directed graph layout.

use std::collections::HashMap;

use egui::Vec2;
use petgraph::graph::NodeIndex;
use spectron_core::ArchGraph;

/// Compute a force-directed layout for the given graph.
///
/// Returns a mapping from each node index to a 2D position within the
/// `[0, width] x [0, height]` bounding box. Runs ~200 iterations of the
/// Fruchterman-Reingold algorithm with temperature cooling.
pub fn compute_layout(graph: &ArchGraph, width: f32, height: f32) -> HashMap<NodeIndex, Vec2> {
    let n = graph.node_count();
    if n == 0 {
        return HashMap::new();
    }

    let nodes: Vec<NodeIndex> = graph.node_indices().collect();
    let node_to_idx: HashMap<NodeIndex, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, &n)| (n, i))
        .collect();

    let area = width * height;
    let k = (area / n as f32).sqrt();

    let mut pos: Vec<Vec2> = Vec::with_capacity(n);
    for (i, _) in nodes.iter().enumerate() {
        let angle = 2.0 * std::f32::consts::PI * i as f32 / n as f32;
        let r = k * (i as f32 + 1.0).sqrt();
        pos.push(Vec2::new(
            width / 2.0 + r * angle.cos(),
            height / 2.0 + r * angle.sin(),
        ));
    }

    let max_iterations = 200;
    let mut temperature = width.min(height) / 4.0;
    let cooling = temperature / max_iterations as f32;

    for _ in 0..max_iterations {
        let mut disp = vec![Vec2::ZERO; n];

        // Repulsive forces between all node pairs.
        for i in 0..n {
            for j in (i + 1)..n {
                let delta = pos[i] - pos[j];
                let dist = delta.length().max(1.0);
                let repulsion = k * k / dist;
                let force = delta / dist * repulsion;
                disp[i] += force;
                disp[j] -= force;
            }
        }

        // Attractive forces along edges.
        for edge_idx in graph.edge_indices() {
            if let Some((a, b)) = graph.edge_endpoints(edge_idx) {
                let ai = node_to_idx[&a];
                let bi = node_to_idx[&b];
                let delta = pos[ai] - pos[bi];
                let dist = delta.length().max(1.0);
                let weight = graph[edge_idx].weight;
                let attraction = dist * dist / k * weight;
                let force = delta / dist * attraction;
                disp[ai] -= force;
                disp[bi] += force;
            }
        }

        // Apply displacements capped by temperature.
        for i in 0..n {
            let len = disp[i].length().max(0.001);
            let bounded = disp[i] / len * len.min(temperature);
            pos[i] += bounded;
            pos[i].x = pos[i].x.clamp(30.0, width - 30.0);
            pos[i].y = pos[i].y.clamp(30.0, height - 30.0);
        }

        temperature -= cooling;
        if temperature < 0.1 {
            break;
        }
    }

    nodes
        .into_iter()
        .enumerate()
        .map(|(i, node_idx)| (node_idx, pos[i]))
        .collect()
}
