# 11 -- UI Views & User Experience

## PRD Reference

### Views

**Security View**: Source-to-sink paths, trust boundaries, auth coverage.

**Performance View**: Hot paths, cost heatmap, loop issues.

**Architecture View**: Module graph, cycles, layer violations.

### User Experience

The PRD calls for views that surface analysis results organized by concern,
not just raw graph visualizations.

---

## Current Implementation

### Existing Views (`spectron-ui/src/lib.rs`)

The UI has four view modes defined in `ViewMode`:

```rust
pub enum ViewMode {
    Overview,                    // Stats dashboard
    StructureGraph,              // Full architecture graph canvas
    CallGraph,                   // Call-only graph canvas
    ModuleDetail(ModuleId),      // Single module detail panel
}
```

### Overview (`show_overview()`)

Displays stats: crate count, module count, file count, symbol count,
function count, struct count, trait count, total lines, entrypoint count,
complexity flag count.

### Graph Views (`graph_view.rs`)

Interactive canvas with:
- Pan/zoom (mouse scroll + drag)
- Node selection and dragging
- Color-coded nodes by type (Crate, Module, File, Symbol)
- Color-coded edges by relationship kind
- Entrypoint halos, selection rings, hover highlights
- Tooltips for nodes

### Filter Panel (`filter_panel.rs`)

Presets: Dependencies, Modules, Call Flow, Type Graph, Imports, Everything.

Filters by: Node type, Symbol kind, Edge type, Visibility, Crate, Isolate
(entrypoints only, unsafe only, flagged only).

### Inspector (`inspector.rs`)

Three inspector targets: `Symbol(SymbolId)`, `Module(ModuleId)`, `Crate(CrateId)`.

- Symbol inspector: name, kind, signature, visibility, module, file, span,
  metrics, complexity flags, security attributes, callers/callees.
- Module inspector: name, path, file, parent, children, symbols, module metrics.
- Crate inspector: name, type, path, modules, dependencies.

### Layout (`layout.rs`)

Fruchterman-Reingold force-directed layout with:
- Repulsion between all visible nodes
- Attraction along edges (weighted by `GraphEdge.weight`)
- Simulated annealing (200 iterations, decreasing temperature)
- Edge attraction weighted by edge weight

### What Is Missing

- **Findings list view**: No view that shows findings sorted by priority.
- **Security view**: No source-to-sink path visualization, no trust boundary view.
- **Performance view**: No cost heatmap, no hotspot highlighting.
- **Architecture view**: No cycle highlighting, no layer violation overlay.
- **Finding detail panel**: Inspector shows symbol/module info but not finding details.

---

## Gap Analysis

| PRD View | Current Status | Action |
|---|---|---|
| Security View (source-sink paths) | Not implemented | Add taint path visualization on call graph |
| Security View (trust boundaries) | Not implemented | Color entrypoints by trust level |
| Security View (auth coverage) | Not implemented | Highlight guarded vs unguarded endpoints |
| Performance View (hot paths) | Not implemented | Color nodes by cost on call graph |
| Performance View (cost heatmap) | Not implemented | Heat color scale on graph nodes |
| Performance View (loop issues) | Not implemented | Highlight N+1 and loop-cost findings |
| Architecture View (module graph) | Structure graph exists | Filter to module-only with cycle overlay |
| Architecture View (cycles) | Not implemented | Highlight SCC cycles in module graph |
| Architecture View (layer violations) | Not implemented | Color nodes by layer, highlight violations |
| Findings list | Not implemented | Sortable table of all findings |
| Finding detail | Inspector exists but not findings-aware | Extend inspector for Finding details |

---

## Design

### New View Modes

Extend the `ViewMode` enum:

```rust
pub enum ViewMode {
    // Existing
    Overview,
    StructureGraph,
    CallGraph,
    ModuleDetail(ModuleId),

    // New
    FindingsList,           // Sortable table of all findings
    SecurityView,           // Call graph colored by taint flow
    PerformanceView,        // Call graph colored by cost
    ArchitectureView,       // Module graph with cycles and layers
}
```

### Findings List View

A table view showing all findings sorted by priority score (spec 09):

| Column | Source |
|---|---|
| Priority | FindingScore.total_score |
| Severity | Finding.severity (color-coded) |
| Category | Finding.category (icon) |
| Rule | Finding.rule_id |
| Title | Finding.title |
| Location | Finding.location (file:line) |
| Confidence | Finding.confidence |

**Interactions**:
- Click a row to open finding detail in inspector.
- Filter by category, severity, confidence.
- Sort by any column.

### Security View

The call graph overlaid with taint analysis results:

1. **Base**: Render the call graph (existing `CallGraph` view mode).
2. **Taint overlay**:
   - Color taint source functions in **red**.
   - Color taint sink functions in **orange**.
   - Color sanitizer functions in **green**.
   - Highlight taint paths (source -> sink) with **red edges**.
3. **Trust boundary overlay**:
   - External entrypoints: **red** halo.
   - Internal entrypoints: **yellow** halo.
   - Auth-guarded functions: **green** border.
4. **Sidebar**: List of taint findings with source-sink path detail.

### Performance View

The call graph overlaid with cost analysis results:

1. **Base**: Render the call graph.
2. **Cost heatmap**:
   - Color nodes by `total_cost` using a gradient (cool blue -> hot red).
   - Node size proportional to cost.
3. **Loop issue markers**:
   - Functions containing N+1 patterns: **warning icon overlay**.
4. **Sidebar**: Top-N hotspots ranked by cost.

### Architecture View

The module-level subgraph with structural analysis results:

1. **Base**: Render only Module and Crate nodes from the structure graph
   (use existing filter presets as a starting point).
2. **Cycle overlay**:
   - Highlight SCC cycles with **red** edges.
   - Cycle member nodes get a **red** border.
3. **Layer overlay** (if layer rules configured):
   - Color nodes by layer (e.g., presentation=blue, domain=green, infra=orange).
   - Violation edges highlighted in **red**.
4. **Sidebar**: List of cycle findings, layer violations, god modules.

### Inspector Extension

When a `Finding` is selected (from findings list or graph overlay), the
inspector shows:

- Finding title and explanation
- Severity and confidence badges
- Rule ID (with link to rule description)
- Location (file, line, module)
- Call path (if applicable, as clickable chain)
- Suggested fix (if available)
- Reachability
- Priority score breakdown (from spec 09)

### Implementation Approach

The existing `GraphViewState` already supports:
- Per-node color overrides (via filters/highlights)
- Selected node tracking
- Filter presets

New views reuse the existing graph canvas but apply different:
- Filter presets (e.g., architecture view filters to Module+Crate only)
- Color functions (e.g., security view colors by taint role)
- Overlay rendering (e.g., cycle edges drawn on top)

This means new views are primarily **new render modes** on the existing
`graph_view.rs` canvas, not entirely separate UI components.

---

## Design Decisions

1. **Extend existing graph canvas, not replace**: The existing graph view with
   force-directed layout, pan/zoom, and filtering is solid infrastructure.
   New views are render overlays on the same canvas, not separate implementations.

2. **Findings list as primary entry point**: Users should start at the findings
   list (sorted by priority), then drill into specific views for context.

3. **Sidebar context panels**: Each specialized view has a sidebar showing
   relevant findings for the current view mode, not all findings.

4. **Color schemes avoid conflicts**: Each view mode uses a distinct color
   palette so users can visually distinguish which view they're in.

5. **Existing filter panel adapts to view mode**: The filter panel shows
   relevant filters for the active view mode. In security view, taint-related
   filters. In architecture view, layer filters.

6. **Finding detail reuses inspector panel**: The right-side inspector panel
   already shows symbol/module details. Extend it with a `Finding` variant
   in `InspectorTarget`.

---

## Tasks

- [ ] **T-11.1** Add `FindingsList`, `SecurityView`, `PerformanceView`, `ArchitectureView` variants to `ViewMode`
- [ ] **T-11.2** Implement findings list view: sortable table with filter/sort controls
- [ ] **T-11.3** Add `InspectorTarget::Finding(FindingId)` to inspector panel
- [ ] **T-11.4** Implement finding detail display in inspector (title, explanation, severity, confidence, call path, suggested fix)
- [ ] **T-11.5** Implement security view: taint overlay on call graph (source=red, sink=orange, sanitizer=green, taint edges=red)
- [ ] **T-11.6** Implement trust boundary overlay: color entrypoints by trust level
- [ ] **T-11.7** Implement performance view: cost heatmap coloring on call graph nodes
- [ ] **T-11.8** Implement performance view sidebar: top-N hotspots list
- [ ] **T-11.9** Implement architecture view: module-only subgraph with cycle highlighting
- [ ] **T-11.10** Implement layer violation overlay on architecture view
- [ ] **T-11.11** Adapt filter panel to show view-mode-specific filters
- [ ] **T-11.12** Add view mode tabs/buttons to the top bar for switching between views
- [ ] **T-11.13** Add `--findings` output to CLI for text/JSON finding list (non-GUI)
