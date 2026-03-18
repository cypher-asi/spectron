# 04 -- Module 2: Auth & Trust Boundary Analyzer

## PRD Reference

### Goal

Detect missing or inconsistent authorization and trust violations.

### Capabilities

- Identify public entrypoints
- Track auth checks across call paths
- Detect missing guard conditions

### Features

- "Must-call" rule enforcement
- Path-sensitive checks
- Trust boundary graph

### Example Rule

```
All admin endpoints must call require_admin()
```

### MVP Scope

- Entrypoint detection (HTTP, CLI)
- Required function presence checks
- Simple path analysis

---

## Current Implementation

### Entrypoint Detection (EXISTS)

`spectron-analysis/src/entrypoints.rs` already provides robust entrypoint detection
with 6 heuristic rules:

1. `main` function in root module
2. `#[tokio::main]`, `#[async_std::main]`, `#[actix_web::main]`
3. HTTP handler attributes: `#[get(...)]`, `#[post(...)]`, `#[put(...)]`, `#[delete(...)]`, `#[handler]`
4. Test functions (`is_test = true`)
5. CLI command attributes (`#[command]`)
6. Functions with zero callers

Output: `Vec<SymbolId>` of all detected entrypoints, stored in `AnalysisOutput.entrypoints`.

### Call Graph (EXISTS)

`CallGraphData` provides:
- `callers: HashMap<SymbolId, Vec<SymbolId>>` -- who calls this function
- `callees: HashMap<SymbolId, Vec<SymbolId>>` -- what this function calls

`find_paths(graph, from, to, max_depth)` can trace call chains from entrypoints
to any target function.

### What Is Missing

- **Auth guard function detection**: No mechanism to identify which functions
  serve as authorization guards (e.g., `require_admin()`, `check_auth()`).
- **Must-call enforcement**: No rule system that says "all HTTP handlers must
  call function X before proceeding."
- **Trust boundary classification**: Entrypoints are detected but not classified
  by trust level (public internet, internal API, admin-only).
- **Path analysis for auth presence**: No analysis checks whether an auth
  function appears on every path from entrypoint to sensitive operation.

---

## Gap Analysis

| PRD Requirement | Current Status | Action |
|---|---|---|
| Identify public entrypoints | Done (6 heuristic rules) | Classify by trust level |
| Track auth checks across call paths | Not done | Build must-call path analysis |
| Detect missing guard conditions | Not done | Implement auth guard detection |
| Must-call rule enforcement | Not done | Define rule format and checker |
| Trust boundary graph | Not done | Derive from entrypoint classification |

---

## Design

### Architecture

```
spectron-analysis/src/auth/
  mod.rs          -- public API: run_auth_analysis()
  guards.rs       -- auth guard function detection
  trust.rs        -- trust boundary classification
  must_call.rs    -- must-call rule enforcement
```

### Auth Guard Detection

Auth guards are functions that enforce authorization. Detected via:

1. **Name patterns**: Functions containing `auth`, `require`, `check_permission`,
   `verify_token`, `guard`, `authorize`, `is_admin`, `has_role`.

2. **Attribute patterns**: Functions with `#[guard]`, `#[middleware]`,
   `#[require_role]` or similar attributes.

3. **User-defined rules** (Phase 5): Config file listing specific function names
   as auth guards.

For MVP, name-based heuristic detection.

### Trust Boundary Classification

Entrypoints are classified into trust levels based on their detection rule:

| Trust Level | Entrypoint Type | Risk |
|---|---|---|
| External | HTTP handlers (`#[get]`, `#[post]`, etc.) | Highest -- internet-facing |
| External | CLI commands (`main`, `#[command]`) | High -- user input |
| Internal | Functions with zero callers (library APIs) | Medium -- internal exposure |
| Test | Test functions (`is_test = true`) | Low -- test-only |

### Must-Call Rule Enforcement

A **must-call rule** specifies that all call paths from a given entrypoint class
to a sensitive operation must pass through a guard function.

```
Rule:
  scope: HTTP handlers (entrypoints with handler attributes)
  must_call: any function matching guard patterns
  before: any function (reachability from entrypoint is sufficient)
```

**Algorithm**:

1. For each HTTP handler entrypoint, collect all functions reachable via
   `descendants()` on the call graph.

2. Check if any reachable function matches an auth guard pattern.

3. If no auth guard is found in the reachable set, emit a finding:
   - `rule_id: security/auth/missing-guard`
   - `severity: High` (for external entrypoints) / `Medium` (for internal)
   - `explanation: "HTTP handler <name> does not call any authorization function"`

**Phase 3 upgrade**: Path-sensitive analysis that checks _every_ path from
entrypoint to sensitive operation (not just reachability), catching cases where
some paths are guarded but others bypass the guard.

### Sensitive Operations

Operations that should require authorization:

| Operation Type | Detection |
|---|---|
| Database writes | Callees matching `insert`, `update`, `delete`, `execute` |
| User management | Callees matching `create_user`, `delete_user`, `update_role` |
| File writes | Callees matching filesystem write patterns (from security.rs) |
| Admin functions | Functions in modules matching `admin` in path |

---

## Design Decisions

1. **Reuse entrypoint detection as-is**: The existing 6-rule entrypoint detection
   in `entrypoints.rs` is the starting point. The auth module adds classification
   on top, not a replacement.

2. **Reachability-based for MVP**: Full path-sensitive analysis (checking every
   path independently) is Phase 3. MVP uses reachability: "is any auth guard
   callable from this entrypoint?" This is simpler and catches the most obvious
   missing-auth cases.

3. **Name-based guard detection**: Like the taint engine's sanitizer detection,
   auth guard detection starts with name heuristics. This is imprecise but
   practical. Custom guard names come via the rule engine (Phase 5).

4. **Findings over custom types**: Auth analysis emits `Finding` (spec 02)
   directly, not a custom `AuthViolation` type.

5. **Trust boundaries are metadata on entrypoints**: Rather than building a
   separate trust boundary graph, trust level is an attribute of each entrypoint.
   A dedicated graph visualization comes in Phase 5 (spec 11, UI views).

---

## Tasks

- [ ] **T-04.1** Create `spectron-analysis/src/auth/` module directory with `mod.rs`, `guards.rs`, `trust.rs`, `must_call.rs`
- [ ] **T-04.2** Implement auth guard detection: name-based pattern matching to identify guard functions
- [ ] **T-04.3** Implement trust boundary classification: categorize entrypoints by trust level (External/Internal/Test)
- [ ] **T-04.4** Implement must-call analysis: for each external entrypoint, check if any auth guard is reachable via call graph
- [ ] **T-04.5** Emit `Finding` for entrypoints missing auth guards (depends on T-02.1)
- [ ] **T-04.6** Wire `run_auth_analysis()` into `spectron-analysis::analyze()`
- [ ] **T-04.7** Add unit tests: entrypoint with guard (no finding), entrypoint without guard (finding emitted)
- [ ] **T-04.8** Add integration test: fixture project with HTTP handler missing auth check
- [ ] **T-04.9** (Phase 3) Upgrade to path-sensitive must-call: check every path, not just reachability
- [ ] **T-04.10** (Phase 5) Support user-defined guard function names via config/rule engine
