# 08 -- Interprocedural Function Summaries

## PRD Reference

### Cross-Module Shared Engine

All analysis modules rely on interprocedural summaries. Each function stores:

- Side effects
- Cost profile
- Taint in/out
- Required preconditions
- Resource usage

This is the **Phase 3** deliverable that scales accuracy and reduces noise
across all analysis modules.

---

## Current Implementation

There are **no interprocedural summaries**. Each analysis module operates
independently using raw graph traversal:

### What Exists

1. **Call graph** (`GraphSet.call_graph`, `CallGraphData`):
   - Direct callers/callees per function
   - Used for fan-in/fan-out, entrypoint detection, and security indicator propagation

2. **Per-function CFG** (`GraphSet.control_flow_graphs`):
   - Intra-procedural control flow with branch/loop/await/return nodes
   - Used for cyclomatic complexity computation

3. **Per-symbol metrics** (`SymbolMetrics`):
   - Cyclomatic complexity, line count, parameter count, fan-in, fan-out
   - These are single-function metrics, not summaries that account for callees

4. **Security indicators** (`SecurityReport`):
   - Detected per-symbol (unsafe, FFI, sensitive APIs)
   - Not propagated through call chains

### What Is Missing

- **Function summary type**: No struct that captures a function's interprocedural
  behavior (side effects, cost, taint, resources).
- **Bottom-up summary computation**: No algorithm to compute summaries from
  leaves to roots of the call graph.
- **Summary-based analysis**: All analysis modules re-traverse the call graph
  instead of querying precomputed summaries.

---

## Gap Analysis

| PRD Component | Current Status | Action |
|---|---|---|
| Side effects | Not tracked | Derive from callee patterns |
| Cost profile | Not implemented (spec 05 adds direct cost) | Propagate costs into summaries |
| Taint in/out | Not implemented (spec 03 is path-based) | Track taint parameters per function |
| Required preconditions | Not implemented | Track auth guard requirements |
| Resource usage | Not implemented (spec 07 is intra-procedural) | Track resource acquire/release in summaries |

---

## Design

### Architecture

```
spectron-analysis/src/summary/
  mod.rs          -- public API: compute_summaries()
  types.rs        -- FunctionSummary and related types
  compute.rs      -- bottom-up summary computation
```

### Function Summary Type

```rust
pub struct FunctionSummary {
    pub symbol_id: SymbolId,

    // Side effects
    pub side_effects: Vec<SideEffect>,

    // Cost (from spec 05)
    pub direct_cost: f32,
    pub total_cost: f32,
    pub cost_breakdown: Vec<(CostType, f32)>,

    // Taint (from spec 03)
    pub taint_sources: bool,      // does this function introduce taint?
    pub taint_propagates: bool,   // does taint in -> taint out?
    pub taint_sanitizes: bool,    // does this function sanitize taint?
    pub taint_sinks: Vec<String>, // sink types this function reaches

    // Auth (from spec 04)
    pub requires_auth: bool,      // does any path through this function check auth?
    pub provides_auth: bool,      // is this function itself an auth guard?

    // Resources (from spec 07)
    pub resources_acquired: Vec<ResourceKind>,
    pub resources_released: Vec<ResourceKind>,
    pub holds_lock: bool,
    pub has_await: bool,

    // Structural
    pub is_pure: bool,            // no side effects
    pub max_call_depth: u32,
}

pub enum SideEffect {
    FileRead,
    FileWrite,
    NetworkCall,
    DbQuery,
    DbWrite,
    SubprocessExec,
    Logging,
    Allocation,
    LockAcquire,
    Panic,
}
```

### Computation Algorithm

**Bottom-up propagation on the call graph:**

1. **Topological sort** the call graph (from T-01.7). Handle cycles by
   identifying SCCs and processing them as groups.

2. **Leaf functions first**: For functions with no callees:
   - Compute direct properties from the function's own code:
     - Side effects from callee name patterns (same as spec 03/05/07 pattern catalogs)
     - Direct cost from cost weight table (spec 05)
     - Taint source/sink/sanitizer from pattern matching (spec 03)
     - Resource acquire/release from resource patterns (spec 07)
     - `is_pure = side_effects.is_empty()`

3. **Propagate upward**: For each function in reverse topological order:
   - Merge callee summaries:
     - `side_effects = own_side_effects UNION callee.side_effects`
     - `total_cost = direct_cost + sum(callee.total_cost)`
     - `taint_propagates = any callee taint_sources OR taint_propagates`
     - `taint_sinks = own_sinks UNION callee.taint_sinks`
     - `requires_auth = any callee provides_auth`
     - `resources_acquired = own UNION callee.resources_acquired`
     - `is_pure = own is_pure AND all callees are_pure`
     - `max_call_depth = 1 + max(callee.max_call_depth)`

4. **Handle cycles (SCCs)**:
   - For functions in the same SCC, compute a fixed-point:
     - Initialize each function's summary from direct properties.
     - Iterate: merge callee summaries within the SCC until convergence
       (no summary changes).
     - Cap iterations (e.g., 10) to prevent infinite loops.

### Integration with Analysis Modules

Once summaries are computed, each analysis module queries summaries instead
of re-traversing the call graph:

| Module | Uses Summary Fields |
|---|---|
| Taint (spec 03) | `taint_sources`, `taint_propagates`, `taint_sanitizes`, `taint_sinks` |
| Auth (spec 04) | `requires_auth`, `provides_auth` |
| Cost (spec 05) | `total_cost`, `cost_breakdown` |
| Structural (spec 06) | `side_effects`, `is_pure` |
| Resource (spec 07) | `resources_acquired`, `resources_released`, `holds_lock`, `has_await` |

### Incremental Summary Updates (Phase 6)

When diff-aware analysis is implemented (spec 12), summaries can be recomputed
incrementally:

1. Identify changed functions.
2. Recompute their summaries.
3. Propagate changes upward through callers only.

This avoids full recomputation for small diffs.

---

## Design Decisions

1. **Summaries are computed AFTER individual analyses in Phase 2-4, then
   become the BACKBONE in Phase 3+**: In the initial build, each analysis
   module operates independently. Once summaries are implemented, modules
   switch to querying summaries rather than doing their own traversals.

2. **Fixed-point for cycles**: Recursive functions create cycles in the call
   graph. The standard approach is iterative fixed-point computation with a
   convergence check.

3. **Summary is append-only**: New analysis modules add fields to
   `FunctionSummary` as they are built. The summary struct grows over time.
   Using `Option<T>` for fields that aren't yet computed avoids breaking
   changes.

4. **Summaries stored in AnalysisOutput**: The summary map is a primary
   analysis artifact, stored as
   `HashMap<SymbolId, FunctionSummary>` in `AnalysisOutput`.

5. **Pattern catalogs are shared**: The taint engine, cost analyzer, and
   resource analyzer all define pattern catalogs. With summaries, these
   catalogs are applied once during summary computation, not repeatedly
   during each analysis pass.

---

## Tasks

- [ ] **T-08.1** Create `spectron-analysis/src/summary/` module directory with `mod.rs`, `types.rs`, `compute.rs`
- [ ] **T-08.2** Define `FunctionSummary` struct with all fields (use `Option<T>` for initially-uncomputed fields)
- [ ] **T-08.3** Define `SideEffect` enum
- [ ] **T-08.4** Implement topological sort on call graph (shared with T-01.7)
- [ ] **T-08.5** Implement leaf-function summary computation from direct analysis
- [ ] **T-08.6** Implement bottom-up summary propagation (merge callee summaries)
- [ ] **T-08.7** Implement SCC fixed-point computation for recursive call cycles
- [ ] **T-08.8** Add `summaries: HashMap<SymbolId, FunctionSummary>` to `AnalysisOutput`
- [ ] **T-08.9** Refactor taint engine (spec 03) to query summaries instead of re-traversing call graph
- [ ] **T-08.10** Refactor cost analyzer (spec 05) to use summary total_cost instead of re-propagating
- [ ] **T-08.11** Refactor resource analyzer (spec 07) to use summary resource fields
- [ ] **T-08.12** Add unit tests: summary computation on known call graphs, cycle handling, propagation correctness
