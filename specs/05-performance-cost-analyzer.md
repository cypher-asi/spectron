# 05 -- Module 3: Performance Cost Analyzer

## PRD Reference

### Goal

Identify likely performance bottlenecks statically.

### Core Idea

Assign cost weights and propagate through call graph.

### Cost Types

Allocation, lock, I/O, network, DB, serialization, cloning/copy.

### Key Detections

- DB calls inside loops
- Repeated allocations
- Nested expensive calls
- High fan-out functions

### MVP Scope

- Loop + call graph analysis
- Simple cost weights
- Hotspot ranking

---

## Current Implementation

### Metrics (EXISTS -- `spectron-analysis/src/metrics.rs`)

Currently computed per-symbol:

```rust
pub struct SymbolMetrics {
    pub symbol_id: SymbolId,
    pub cyclomatic_complexity: u32,
    pub line_count: u32,
    pub parameter_count: u32,
    pub fan_in: u32,
    pub fan_out: u32,
}
```

Per-module:

```rust
pub struct ModuleMetrics {
    pub module_id: ModuleId,
    pub symbol_count: u32,
    pub line_count: u32,
    pub fan_in: u32,
    pub fan_out: u32,
}
```

Thresholds and flags already defined:

| Flag | Threshold |
|---|---|
| `HighCyclomaticComplexity` | >= 15 |
| `LargeFunction` | >= 100 lines |
| `LargeModule` | >= 50 symbols |
| `HighFanIn` | >= 20 (module) |
| `HighFanOut` | >= 15 (module) |

### CFG with Loop Detection (EXISTS -- `spectron-graph/src/cfg.rs`)

CFG nodes include `CfgNode::Loop { span }` with `CfgEdge::LoopBack` and
`CfgEdge::LoopExit` edges. This means loops are already identified
structurally in the CFG.

### Call Graph (EXISTS)

`CallGraphData.callees` gives the functions each function calls.
Combined with CFG loop detection, this is sufficient to detect
"expensive call inside loop" patterns.

### What Is Missing

- **Cost model**: No concept of operation cost weights. A function call
  to `Vec::push()` and a call to `db.query()` are treated identically.
- **Cost propagation**: No bottom-up accumulation of cost through the
  call graph.
- **N+1 detection**: Loop + DB-call detection not wired together.
- **Allocation detection**: No tracking of allocation patterns.
- **Hotspot ranking**: No ranked list of most expensive functions.

---

## Gap Analysis

| PRD Requirement | Current Status | Action |
|---|---|---|
| Cost weights | Not implemented | Define cost type enum and weight table |
| Cost propagation | Not implemented | Bottom-up propagation through call graph |
| Loop analysis | CFG has Loop nodes | Wire loop detection to cost analysis |
| DB-in-loop detection | Not implemented | Cross-reference loop body callees with DB patterns |
| Allocation detection | Not implemented | Pattern match allocation APIs in callees |
| Hotspot ranking | Not implemented | Sort functions by accumulated cost |
| High fan-out detection | fan_out metric exists | Already flagged via ComplexityFlag |

---

## Design

### Architecture

```
spectron-analysis/src/cost/
  mod.rs          -- public API: run_cost_analysis()
  weights.rs      -- cost type enum and weight table
  patterns.rs     -- expensive operation pattern detection
  propagation.rs  -- bottom-up cost propagation
  hotspots.rs     -- hotspot ranking
  loops.rs        -- loop-aware cost analysis (N+1, etc.)
```

### Cost Type Model

```rust
pub enum CostType {
    Allocation,
    Lock,
    IoRead,
    IoWrite,
    NetworkCall,
    DbQuery,
    Serialization,
    Clone,
}

pub struct CostWeight {
    pub cost_type: CostType,
    pub weight: f32,
}
```

Default weight table:

| CostType | Weight | Rationale |
|---|---|---|
| Allocation | 1.0 | Baseline: heap allocation |
| Lock | 2.0 | Contention risk |
| IoRead | 5.0 | Disk latency |
| IoWrite | 5.0 | Disk latency |
| NetworkCall | 10.0 | Network round-trip |
| DbQuery | 10.0 | DB round-trip |
| Serialization | 2.0 | CPU cost |
| Clone | 1.0 | Memory + CPU |

### Expensive Operation Detection

Detect cost-bearing operations by matching callee names/paths:

| CostType | Detection Pattern |
|---|---|
| DbQuery | `query`, `execute`, `fetch`, `prepare` on db-like paths (`sqlx::`, `diesel::`, `rusqlite::`) |
| NetworkCall | `reqwest::`, `hyper::`, `TcpStream::connect` |
| Allocation | `Vec::new`, `Box::new`, `String::from`, `HashMap::new`, `to_vec`, `to_string`, `to_owned` |
| Clone | `.clone()`, `Clone::clone` |
| IoRead | `std::fs::read`, `File::open`, `BufReader::new` |
| IoWrite | `std::fs::write`, `File::create`, `BufWriter::new` |
| Lock | `Mutex::lock`, `RwLock::read`, `RwLock::write` |
| Serialization | `serde_json::to_string`, `serde_json::from_str`, `bincode::serialize` |

### Cost Propagation Algorithm

**Bottom-up propagation on the call graph:**

1. Topologically sort the call graph (using T-01.7).
2. For each function in reverse topological order (leaves first):
   a. Compute **direct cost** = sum of weights of directly detected expensive operations.
   b. Compute **propagated cost** = sum of callee costs (from their total cost).
   c. **total_cost** = direct_cost + propagated_cost.
3. Store `FunctionCost { direct, propagated, total, breakdown: Vec<(CostType, f32)> }` per symbol.

Handle cycles (recursive functions) by capping propagation depth or assigning
a fixed recursive penalty.

### Loop-Aware Cost Analysis (N+1 Detection)

For each function with a CFG containing a `CfgNode::Loop`:

1. Identify the loop body: all CFG nodes reachable from the loop node
   via non-LoopExit edges until LoopBack.

2. Collect call expressions within the loop body (by matching CFG Statement
   spans to call relationship spans).

3. For each call in the loop body:
   - If the callee has `CostType::DbQuery` cost: emit finding
     `perf/cost/db-in-loop` (N+1 query pattern).
   - If the callee has `CostType::NetworkCall` cost: emit finding
     `perf/cost/network-in-loop`.
   - If the callee has `CostType::Allocation` and is `Vec::new` / `HashMap::new`:
     emit finding `perf/cost/allocation-in-loop`.

### Hotspot Ranking

Sort all functions by `total_cost` descending. Top-N (configurable, default 20)
are reported as hotspots:

- `rule_id: perf/cost/hotspot`
- `severity`: based on cost threshold (Critical > 100, High > 50, Medium > 20)
- `explanation`: "Function <name> has estimated cost <total> (<breakdown>)"

---

## Design Decisions

1. **Static cost estimation, not profiling**: These are heuristic weights, not
   runtime measurements. The goal is to flag likely bottlenecks for human review,
   not to replace profiling.

2. **Callee name matching for MVP**: Like the taint engine, cost detection
   starts with name-based pattern matching. Type-aware cost assignment comes
   with interprocedural summaries (Phase 3).

3. **CFG loop body analysis**: The existing CFG already marks loop nodes.
   Extracting loop body callees requires mapping CFG statement spans back to
   call relationships -- a span intersection check.

4. **Propagation uses existing call graph**: No new graph is needed. Walk the
   `CallGraphData.callees` in reverse topological order.

5. **Existing fan-out flags are complementary**: The `HighFanOut` complexity
   flag already detects functions that call too many things. The cost analyzer
   adds _what kind_ of calls they make and how expensive they are.

---

## Tasks

- [ ] **T-05.1** Create `spectron-analysis/src/cost/` module directory with `mod.rs`, `weights.rs`, `patterns.rs`, `propagation.rs`, `hotspots.rs`, `loops.rs`
- [ ] **T-05.2** Define `CostType` enum and default weight table
- [ ] **T-05.3** Implement expensive operation detection: match callee names to cost patterns
- [ ] **T-05.4** Implement bottom-up cost propagation on call graph (requires topological sort from T-01.7)
- [ ] **T-05.5** Implement `FunctionCost` data structure with direct/propagated/total/breakdown
- [ ] **T-05.6** Implement loop body callee extraction: map CFG loop nodes to call relationships via span matching
- [ ] **T-05.7** Implement N+1 detection: DB/network calls inside loops
- [ ] **T-05.8** Implement hotspot ranking: sort by total_cost, emit top-N as findings
- [ ] **T-05.9** Add `CostType` detection patterns for allocation-in-loop
- [ ] **T-05.10** Wire `run_cost_analysis()` into `spectron-analysis::analyze()`, add `function_costs` to `AnalysisOutput`
- [ ] **T-05.11** Add unit tests: known expensive call in loop produces finding, propagation sums correctly
- [ ] **T-05.12** Add integration test: fixture project with N+1 query pattern
