# 09 -- Finding Ranking & Prioritization

## PRD Reference

### Rank by

1. Reachability (public > internal)
2. Exploitability
3. Runtime cost impact
4. Dependency centrality
5. Code churn
6. Confidence

---

## Current Implementation

There is **no finding ranking**. The existing outputs are unordered lists:

- `SecurityReport.indicators: Vec<SecurityIndicator>` -- insertion order
- `AnalysisOutput.complexity_flags: Vec<ComplexityFlag>` -- insertion order
- `AnalysisOutput.entrypoints: Vec<SymbolId>` -- sorted by SymbolId value

The UI does not sort or prioritize findings. Users see raw lists.

### Building Blocks That Exist

1. **Entrypoints** (`AnalysisOutput.entrypoints`): Can determine reachability
   from public entrypoints via the call graph.

2. **Fan-in / fan-out** (`SymbolMetrics`, `ModuleMetrics`): Fan-in is a proxy
   for dependency centrality (more callers = more central).

3. **Cyclomatic complexity** (`SymbolMetrics.cyclomatic_complexity`): Higher
   complexity correlates with harder-to-fix issues.

4. **File hashes** (`FileInfo.hash`): SHA-256 content hashes exist for change
   detection. Code churn requires historical data (not yet available).

---

## Gap Analysis

| PRD Ranking Factor | Current Status | Action |
|---|---|---|
| Reachability | Entrypoints exist; no reachability scoring per finding | Compute reachability from entrypoints |
| Exploitability | Not implemented | Derive from finding category + sink type |
| Runtime cost impact | Not implemented (spec 05 adds cost) | Use function cost from summary |
| Dependency centrality | fan_in exists | Use fan_in as centrality proxy |
| Code churn | File hashes exist; no history | Phase 6 (diff/baseline) |
| Confidence | Not implemented (spec 02 adds confidence to Finding) | Use Finding.confidence directly |

---

## Design

### Architecture

```
spectron-analysis/src/ranking/
  mod.rs          -- public API: rank_findings()
  score.rs        -- scoring algorithm
  factors.rs      -- individual factor computation
```

### Scoring Model

Each finding receives a **priority score** (0-100) computed from weighted factors:

```rust
pub struct FindingScore {
    pub finding_id: FindingId,
    pub total_score: f32,
    pub factors: RankingFactors,
}

pub struct RankingFactors {
    pub reachability_score: f32,    // 0-1
    pub exploitability_score: f32,  // 0-1
    pub cost_impact_score: f32,     // 0-1
    pub centrality_score: f32,      // 0-1
    pub confidence_score: f32,      // 0-1
    pub severity_score: f32,        // 0-1
}
```

### Factor Computation

**Reachability (weight: 0.25)**

```
if finding.reachability == Public:    1.0
if finding.reachability == Internal:  0.5
if finding.reachability == Unknown:   0.3
```

More precisely: if the finding's symbol is reachable from any external
entrypoint (HTTP handler, main, CLI command) via the call graph, score = 1.0.
If reachable only from internal entrypoints (zero-caller functions), score = 0.5.

**Exploitability (weight: 0.20)**

Derived from finding category and rule:

| Category | Rule Pattern | Score |
|---|---|---|
| Security | `taint/sql-injection` | 1.0 |
| Security | `taint/command-injection` | 1.0 |
| Security | `auth/missing-guard` | 0.9 |
| Security | `taint/ssrf` | 0.8 |
| Performance | `cost/db-in-loop` | 0.5 |
| Architecture | `structure/cyclic-dependency` | 0.3 |
| Quality | `complexity/*` | 0.2 |

**Cost Impact (weight: 0.15)**

If function cost data is available (spec 05/08):

```
cost_impact = min(1.0, function_total_cost / COST_THRESHOLD)
```

Where `COST_THRESHOLD` is a configurable value (default: 50.0).

If not available: 0.0 (neutral).

**Centrality (weight: 0.15)**

```
centrality = min(1.0, fan_in / CENTRALITY_THRESHOLD)
```

Where `CENTRALITY_THRESHOLD` = 20. Functions called by many others have
higher centrality, meaning bugs in them affect more of the codebase.

**Confidence (weight: 0.15)**

```
High:   1.0
Medium: 0.6
Low:    0.3
```

**Severity (weight: 0.10)**

```
Critical: 1.0
High:     0.8
Medium:   0.5
Low:      0.3
Info:     0.1
```

### Total Score

```
total = reachability * 0.25
      + exploitability * 0.20
      + cost_impact * 0.15
      + centrality * 0.15
      + confidence * 0.15
      + severity * 0.10
```

Scaled to 0-100 for display.

### Reachability Computation

A pre-pass before ranking:

1. Collect all external entrypoints (HTTP handlers, main, CLI commands).
2. For each entrypoint, compute `descendants()` on the call graph.
3. Build a set of all reachable SymbolIds.
4. For each finding, check if its symbol_id is in the reachable set.
5. Set `finding.reachability = Public` if reachable, else `Internal`.

This also populates `Finding.reachability` (from spec 02) which was initially
set to `Unknown`.

---

## Design Decisions

1. **Weighted linear score**: A simple weighted sum is transparent and
   debuggable. Users can see which factors contributed to each finding's
   priority. More sophisticated models (ML-based) can come later.

2. **Weights are configurable**: The default weights (0.25, 0.20, 0.15, etc.)
   can be overridden via configuration for organizations that prioritize
   different factors.

3. **Code churn deferred to Phase 6**: Churn requires historical file hashes
   or git integration. This is a Phase 6 feature when baseline/diff support
   lands (spec 12).

4. **Reachability is computed once for all findings**: Rather than computing
   reachability per-finding, do a single BFS from all external entrypoints
   and cache the result set.

5. **Score breakdown is exposed**: The UI can display the factor breakdown
   to help users understand why a finding is ranked high/low.

---

## Tasks

- [ ] **T-09.1** Create `spectron-analysis/src/ranking/` module directory
- [ ] **T-09.2** Define `FindingScore` and `RankingFactors` types
- [ ] **T-09.3** Implement reachability pre-pass: BFS from external entrypoints, tag all findings with Public/Internal
- [ ] **T-09.4** Implement exploitability scoring based on finding rule_id
- [ ] **T-09.5** Implement centrality scoring from fan_in metrics
- [ ] **T-09.6** Implement confidence and severity score mapping
- [ ] **T-09.7** Implement cost impact scoring (uses function cost from spec 05, or 0.0 if unavailable)
- [ ] **T-09.8** Implement total score computation with configurable weights
- [ ] **T-09.9** Sort findings by total_score descending in `AnalysisOutput`
- [ ] **T-09.10** Wire `rank_findings()` into `spectron-analysis::analyze()` as a post-processing step
- [ ] **T-09.11** Add unit tests: findings with known factors produce expected scores and ordering
- [ ] **T-09.12** (Phase 6) Add code churn factor from git history integration
