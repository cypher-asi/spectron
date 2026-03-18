# 07 -- Module 5: Resource & Lifetime Analyzer

## PRD Reference

### Goal

Detect correctness and performance issues in resource handling.

### Capabilities

- Track resource lifecycle
- Detect leaks/misuse

### Resources

Files, DB connections, locks, memory, transactions.

### Key Detections

- Open without close
- Lock without unlock
- Transaction without commit/rollback
- Use-after-release patterns

### MVP Scope

- Simple typestate model
- Pattern-based lifecycle tracking

---

## Current Implementation

There is **no resource lifecycle tracking**. However, some building blocks exist:

### Security Indicators (PARTIAL)

`SecurityIndicator` already detects:
- `FilesystemAccess { span, function_name }` -- filesystem API calls
- `SubprocessExecution { span, function_name }` -- subprocess API calls
- `UnsafeBlock` / `UnsafeFunction` -- unsafe code regions

These detect resource _usage_ but not resource _lifecycle_ (open/close pairing).

### CFG (EXISTS)

`ControlFlowGraph` with `CfgNode::Loop`, `CfgNode::Branch`, `CfgNode::Return`
provides intra-procedural control flow. This is needed to determine if a
resource opened on one path is closed on all paths.

### Call Graph (EXISTS)

`CallGraphData` provides callers/callees for tracking resource-related calls
interprocedurally.

### Symbol Attributes (EXISTS)

`SymbolAttributes.is_async` -- relevant for detecting async resource patterns
(e.g., holding a lock across `.await`).

### What Is Missing

- **Resource type tracking**: No concept of what constitutes a "resource."
- **Typestate model**: No state machine for resource lifecycle.
- **Open/close pairing**: No analysis to match resource acquisition with release.
- **Lock-across-await detection**: No check for locks held over await points.
- **Transaction analysis**: No concept of transaction begin/commit/rollback.

---

## Gap Analysis

| PRD Requirement | Current Status | Action |
|---|---|---|
| Track resource lifecycle | Not implemented | Build typestate model |
| Detect leaks (open without close) | Not implemented | Pattern-match open/close pairs |
| Lock without unlock | Not implemented | Detect Mutex::lock without drop/scope exit |
| Transaction without commit/rollback | Not implemented | Detect transaction begin without commit |
| Use-after-release | Not implemented | Phase 4+ (requires data-flow analysis) |

---

## Design

### Architecture

```
spectron-analysis/src/resource/
  mod.rs          -- public API: run_resource_analysis()
  types.rs        -- resource type definitions and state model
  patterns.rs     -- resource API pattern matching
  lifecycle.rs    -- lifecycle analysis (open/close pairing)
  locks.rs        -- lock-specific analysis
```

### Resource Type Model

```rust
pub enum ResourceKind {
    File,
    DbConnection,
    Lock,
    Transaction,
    TcpStream,
    Process,
}

pub enum ResourceState {
    Unopened,
    Acquired,
    Released,
    Error,
}

pub struct ResourceEvent {
    pub kind: ResourceKind,
    pub transition: ResourceTransition,
    pub span: SourceSpan,
    pub symbol_id: SymbolId,
}

pub enum ResourceTransition {
    Acquire,  // File::open, Mutex::lock, db.begin()
    Release,  // drop, close, unlock, commit, rollback
    Use,      // read, write, query
}
```

### Resource Pattern Catalog

| ResourceKind | Acquire Pattern | Release Pattern |
|---|---|---|
| File | `File::open`, `File::create`, `OpenOptions::open` | `drop`, scope exit, `.close()` |
| DbConnection | `connect`, `Pool::get`, `Connection::open` | `drop`, scope exit, `.close()` |
| Lock | `Mutex::lock`, `RwLock::read`, `RwLock::write` | `drop` (MutexGuard), scope exit |
| Transaction | `begin`, `transaction` | `commit`, `rollback` |
| TcpStream | `TcpStream::connect`, `TcpListener::bind` | `drop`, `.shutdown()` |
| Process | `Command::spawn` | `.wait()`, `.kill()` |

### MVP Analysis Strategy: Pattern-Based

For MVP, use a simplified approach that does NOT require full data-flow
analysis:

**Function-level resource lifecycle check:**

1. For each function, scan its callees for resource acquire patterns.
2. If an acquire is found, check if a corresponding release pattern exists
   among the same function's callees.
3. If no release is found within the same function scope:
   a. Check if the function returns a type that wraps the resource
      (e.g., returns a File handle -- not a leak, just a transfer).
   b. If no return-transfer detected, emit a finding.

This is conservative (may miss leaks across function boundaries) but avoids
false positives from Rust's ownership model (RAII handles drop automatically).

**Special case: Rust RAII**

In Rust, most resources are cleaned up automatically via `Drop`. This means
"open without close" is less common than in C/Java. The main risks are:

1. **Long-lived resources**: Opening a file/connection early and holding it
   much longer than needed (not a leak, but a performance issue).
2. **Lock across await**: Holding a `MutexGuard` across an `.await` point
   in async code, which blocks the runtime.
3. **Transaction without commit**: Opening a DB transaction and returning
   without explicit commit/rollback (relies on drop behavior which may
   vary by driver).

### Lock-Across-Await Detection

This is the highest-value detection for Rust codebases.

**Algorithm:**

1. For each async function (where `symbol.attributes.is_async == true`):
2. Get its CFG.
3. Find all lock acquire nodes (callees matching lock patterns).
4. Find all `CfgNode::Await` nodes.
5. If a lock acquire is reachable to an await node WITHOUT an intervening
   drop/unlock: emit finding.
   - `rule_id: resource/lock/held-across-await`
   - `severity: High`
   - `explanation`: "MutexGuard is potentially held across an await point in async function <name>"

### Transaction Lifecycle

**Algorithm:**

1. For each function that calls a transaction begin pattern:
2. Check if the same function (or a callee reachable from it) calls
   commit or rollback.
3. If neither commit nor rollback is reachable: emit finding.
   - `rule_id: resource/transaction/uncommitted`
   - `severity: Medium`

---

## Design Decisions

1. **RAII-aware analysis**: Unlike C or Java resource analyzers, Spectron must
   account for Rust's Drop semantics. Most "open without close" cases are handled
   by RAII, so raw open/close pairing has high false-positive risk. Focus on
   the exceptions: lock-across-await, uncommitted transactions.

2. **Lock-across-await is the highest priority**: This is the most impactful
   Rust-specific resource bug. It should be implemented first.

3. **CFG-based for intra-procedural**: Use the existing CFG to determine if
   a lock guard can reach an await point. This is intra-procedural only for MVP.

4. **Skip use-after-release for now**: Use-after-release requires tracking
   value lifetimes through the data-flow graph, which depends on variable-level
   symbol extraction (Phase 3+). Defer to Phase 7.

5. **Pattern matching is conservative**: Rather than trying to model all
   resource APIs, start with a small catalog of well-known patterns and expand
   incrementally.

---

## Tasks

- [ ] **T-07.1** Create `spectron-analysis/src/resource/` module directory
- [ ] **T-07.2** Define `ResourceKind`, `ResourceState`, `ResourceEvent`, `ResourceTransition` types
- [ ] **T-07.3** Implement resource pattern catalog: map API call names to acquire/release transitions
- [ ] **T-07.4** Implement lock-across-await detection: find lock acquire -> await paths in CFGs of async functions
- [ ] **T-07.5** Implement transaction lifecycle check: begin without commit/rollback detection
- [ ] **T-07.6** Implement basic function-level resource lifecycle check (acquire without release in same scope)
- [ ] **T-07.7** Emit findings for detected resource issues (depends on T-02.1)
- [ ] **T-07.8** Wire `run_resource_analysis()` into `spectron-analysis::analyze()`
- [ ] **T-07.9** Add unit tests: async function with lock-across-await, transaction without commit
- [ ] **T-07.10** (Phase 7) Implement interprocedural resource tracking using function summaries
- [ ] **T-07.11** (Phase 7) Implement use-after-release detection using data-flow analysis
