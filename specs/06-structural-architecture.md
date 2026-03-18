# 06 -- Module 4: Structural / Architecture Analyzer

## PRD Reference

### Goal

Expose codebase health and architectural decay.

### Capabilities

- Dependency graph
- Cycle detection
- Module cohesion analysis
- API surface analysis

### Key Detections

- Cyclic dependencies
- God modules
- Excessive coupling
- Layer violations

### Example Rule

```
UI layer cannot access DB directly
```

### MVP Scope

- Module graph
- Cycle detection
- Rule-based architecture constraints

---

## Current Implementation

### Structure Graph (EXISTS)

The structure graph in `GraphSet.structure_graph` is an `ArchGraph` containing:

- **Nodes**: Crate, Module, File, Symbol
- **Edges**: Contains (hierarchy), Imports (use statements), Implements,
  References, DependsOn (crate-level dependencies)

This is already the dependency graph the PRD calls for.

### Graph Algorithms (EXISTS -- `spectron-graph/src/algorithms.rs`)

- `find_paths(graph, from, to, max_depth)` -- path finding between nodes
- `descendants(graph, node)` -- all reachable nodes from a root
- `reachable_subgraph(graph, roots)` -- induced subgraph
- `components(graph)` -- connected components (undirected)

### Module Metrics (EXISTS)

```rust
pub struct ModuleMetrics {
    pub module_id: ModuleId,
    pub symbol_count: u32,
    pub line_count: u32,
    pub fan_in: u32,
    pub fan_out: u32,
}
```

Thresholds: `LargeModule >= 50 symbols`, `HighFanIn >= 20`, `HighFanOut >= 15`.

### What Is Missing

- **Cycle detection**: `components()` finds connected components but does NOT
  detect directed cycles (SCCs). Tarjan's or Kosaraju's algorithm is needed.
- **God module detection**: `LargeModule` flag exists but only checks symbol count.
  "God module" also needs coupling analysis (fan-in + fan-out combined, or
  ratio of internal vs external dependencies).
- **Coupling metrics**: Fan-in/fan-out are computed but not combined into
  coupling/cohesion scores.
- **Layer violation rules**: No concept of architectural layers or rules.
- **API surface analysis**: No measurement of public vs private symbol ratio.

---

## Gap Analysis

| PRD Requirement | Current Status | Action |
|---|---|---|
| Dependency graph | Structure graph exists | Done (use as-is) |
| Cycle detection | Connected components only (undirected) | Add SCC-based directed cycle detection |
| Module cohesion analysis | Not implemented | Compute cohesion from internal vs external refs |
| API surface analysis | Visibility tracked per symbol | Aggregate public/private ratio per module |
| Cyclic dependency detection | Not implemented | SCC on module/crate dependency subgraph |
| God module detection | LargeModule flag (symbol count only) | Add coupling threshold |
| Excessive coupling | fan_in/fan_out exist | Combine into instability/coupling score |
| Layer violations | Not implemented | Define layer rules + check import edges |

---

## Design

### Architecture

```
spectron-analysis/src/structural/
  mod.rs          -- public API: run_structural_analysis()
  cycles.rs       -- SCC-based cycle detection
  coupling.rs     -- coupling/cohesion metrics
  layers.rs       -- layer violation detection
  api_surface.rs  -- public API surface analysis
```

### Cycle Detection

**Algorithm**: Tarjan's SCC on the module-level subgraph.

1. Extract a module-only subgraph from the structure graph: nodes where
   `GraphNode::Module(_)`, edges where kind is `Imports` or `DependsOn`.

2. Run Tarjan's SCC (petgraph provides `tarjan_scc()`).

3. Any SCC with more than one node represents a cyclic dependency.

4. Emit findings:
   - `rule_id: arch/structure/cyclic-dependency`
   - `severity: Medium` (2 modules in cycle) to `High` (3+ modules)
   - `explanation`: "Cyclic dependency detected: module_a -> module_b -> module_a"
   - `call_path`: module IDs forming the cycle (mapped to symbol IDs of
     the import statements that create the cycle)

Also run at **crate level**: extract crate-only subgraph with DependsOn edges.
Crate-level cycles are `Critical` severity.

### Coupling / Cohesion Metrics

For each module, compute:

```
instability = fan_out / (fan_in + fan_out)
```

Where:
- `fan_in` = number of external modules that import symbols from this module
- `fan_out` = number of external modules this module imports from

Additionally compute:

```
coupling_score = fan_in + fan_out
cohesion = internal_references / total_references
```

Where `internal_references` = calls/references between symbols within the same
module, `total_references` = all calls/references from symbols in this module.

**God module detection**: A module is a "god module" if:
- `symbol_count >= LARGE_MODULE_THRESHOLD` (existing: 50), AND
- `coupling_score >= 30` (high external coupling)

Or alternatively:
- `fan_out >= 20` (depends on many other modules)
- `cohesion < 0.3` (low internal cohesion)

### Layer Violation Detection

**Layer model**: Users define architectural layers as ordered sets of module
path prefixes.

Example configuration:

```toml
[[layers]]
name = "presentation"
modules = ["ui", "views", "controllers"]

[[layers]]
name = "domain"
modules = ["models", "services", "logic"]

[[layers]]
name = "infrastructure"
modules = ["db", "storage", "network", "fs"]
```

**Rule**: A module in layer N may only import from layer N or layer N+1
(one level down). Importing from a lower layer (skipping) or upward is a violation.

**Detection**:

1. For each import edge in the structure graph:
   a. Determine the source module's layer (by matching module path to layer prefixes).
   b. Determine the target module's layer.
   c. If target layer index < source layer index (importing upward): violation.

2. Emit findings:
   - `rule_id: arch/structure/layer-violation`
   - `severity: High`
   - `explanation`: "Module <source> (layer: infrastructure) imports from <target> (layer: presentation)"

For MVP, layer rules are hardcoded or loaded from a `.spectron.toml` config file.
Phase 5 moves this into the rule engine DSL.

### API Surface Analysis

For each module, compute:

```
api_surface_ratio = public_symbols / total_symbols
```

Flag modules with `api_surface_ratio > 0.8` (most symbols are public, possible
over-exposure):
- `rule_id: arch/structure/excessive-api-surface`
- `severity: Low`

---

## Design Decisions

1. **Use petgraph's `tarjan_scc()`**: petgraph already provides SCC. No need
   to implement Tarjan's from scratch.

2. **Module-level analysis for MVP**: Cycle detection and coupling operate on
   modules, not individual symbols. This keeps the analysis fast and the findings
   actionable ("module A depends on module B" is more useful than "function X
   calls function Y").

3. **Layer rules via config for MVP**: Rather than building a full DSL (Phase 5),
   start with a simple TOML config file for layer definitions. If no config
   exists, skip layer violation analysis.

4. **Coupling metrics extend existing ModuleMetrics**: Rather than creating
   a separate type, add `instability`, `cohesion`, and `coupling_score` fields
   to `ModuleMetrics` (or a new `ArchMetrics` struct alongside it).

5. **God module = existing flag + coupling**: The existing `LargeModule`
   complexity flag already catches big modules. The structural analyzer adds
   coupling-aware "god module" detection that may flag smaller but highly
   coupled modules.

---

## Tasks

- [ ] **T-06.1** Create `spectron-analysis/src/structural/` module directory
- [ ] **T-06.2** Implement module-level SCC cycle detection using petgraph's `tarjan_scc()` on the Imports/DependsOn subgraph
- [ ] **T-06.3** Implement crate-level cycle detection on the DependsOn subgraph
- [ ] **T-06.4** Emit cycle findings with severity based on cycle size (depends on T-02.1)
- [ ] **T-06.5** Implement coupling metrics: instability, cohesion, coupling_score per module
- [ ] **T-06.6** Implement god module detection: LargeModule AND high coupling
- [ ] **T-06.7** Implement API surface ratio computation per module
- [ ] **T-06.8** Define layer configuration format (TOML) and parser
- [ ] **T-06.9** Implement layer violation detection: check import edges against layer ordering
- [ ] **T-06.10** Wire `run_structural_analysis()` into `spectron-analysis::analyze()`
- [ ] **T-06.11** Add topological sort algorithm to `spectron-graph/src/algorithms.rs` (shared with T-01.7)
- [ ] **T-06.12** Add unit tests: SCC on fixture graph with known cycles, coupling computation, layer violation detection
- [ ] **T-06.13** Add integration test: fixture workspace with cyclic module imports
