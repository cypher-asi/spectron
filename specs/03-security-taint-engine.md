# 03 -- Module 1: Security Taint Engine

## PRD Reference

### Goal

Detect real-world exploitable vulnerabilities using source-to-sink tracking.

### Capabilities

- Track untrusted input through system
- Identify dangerous sinks
- Validate sanitization presence

### Sources

HTTP input, CLI input, file reads, external APIs.

### Sinks

SQL queries, shell execution, file system writes, template rendering,
HTTP requests (SSRF), logging (secrets).

### MVP Scope

- Function-level taint
- Basic interprocedural summaries
- Known sink library
- Simple sanitizer rules

---

## Current Implementation

There is **no taint engine**. However, several building blocks exist:

### Relevant Existing Infrastructure

1. **Call graph** (`spectron-graph/src/builder.rs`):
   - `GraphSet.call_graph` -- ArchGraph with only Function/Method nodes and Calls edges
   - `CallGraphData.callers` / `CallGraphData.callees` -- O(1) lookup of callers/callees per SymbolId
   - This is the backbone for interprocedural taint propagation.

2. **Entrypoint detection** (`spectron-analysis/src/entrypoints.rs`):
   - Detects `main`, `#[tokio::main]`, HTTP handlers (`#[get]`, `#[post]`, etc.), CLI commands, tests, zero-caller functions
   - Entrypoints are natural taint sources (parameters of HTTP handlers receive untrusted input).

3. **Security indicators** (`spectron-analysis/src/security.rs`):
   - Already detects filesystem access, network access, subprocess execution via call-name matching
   - These are potential taint **sinks** -- currently detected but not connected to taint sources.

4. **Sensitive API patterns** (already defined in `security.rs`):
   ```
   FILESYSTEM_PREFIXES: std::fs::, tokio::fs::
   NETWORK_PREFIXES: std::net::, tokio::net::, reqwest::, hyper::
   SUBPROCESS_PREFIXES: std::process::Command, tokio::process::Command
   ```

5. **Symbol attributes** (`spectron-core/src/symbol.rs`):
   - `SymbolAttributes.attribute_paths` -- used for matching handler attributes
   - `signature: Option<String>` -- can be parsed for parameter types

6. **Graph algorithms** (`spectron-graph/src/algorithms.rs`):
   - `find_paths(graph, from, to, max_depth)` -- directly usable for source-to-sink path finding

---

## Gap Analysis

| Required Component | Current Status | Action |
|---|---|---|
| Source catalog | Entrypoints detected; no taint labeling | Define source rules from entrypoint types |
| Sink catalog | SecurityIndicator detects some sinks | Extend with SQL, template, logging sinks |
| Taint propagation | find_paths exists but no taint semantics | Build taint propagation over call graph |
| Sanitizer recognition | Not implemented | Define sanitizer pattern library |
| Confidence scoring | Not implemented | Score based on path length, sanitizer gaps |
| Path reconstruction | find_paths returns NodeIndex paths | Map back to SymbolId paths with call sites |
| Finding output | SecurityIndicator (no path, no confidence) | Emit unified Finding (spec 02) |

---

## Design

### Architecture

```
spectron-analysis/src/taint/
  mod.rs          -- public API: run_taint_analysis()
  sources.rs      -- source identification rules
  sinks.rs        -- sink catalog
  sanitizers.rs   -- sanitizer pattern matching
  propagation.rs  -- taint propagation algorithm
```

### Source Identification

A **taint source** is a parameter or return value that receives untrusted input.

| Source Type | Detection Rule |
|---|---|
| HTTP handler params | Function is an entrypoint detected via handler attributes (`#[get]`, `#[post]`, etc.) |
| CLI input | Function is `main` or `#[command]`; track args/stdin reads |
| File reads | Call to `std::fs::read*`, `tokio::fs::read*` |
| External API responses | Call to `reqwest::*`, `hyper::*` return values |
| Environment variables | Call to `std::env::var*` |

For MVP, taint sources are **function-level**: the entire function is marked as
producing tainted data, not individual parameters.

### Sink Catalog

A **taint sink** is a function that is dangerous when receiving unsanitized input.

| Sink Type | Detection Pattern | Risk |
|---|---|---|
| SQL execution | Function name contains `query`, `execute`, `prepare` on DB types | SQL injection |
| Shell execution | `std::process::Command`, `tokio::process::Command` | Command injection |
| File writes | `std::fs::write*`, `File::create`, `tokio::fs::write*` | Path traversal |
| Template rendering | `render`, `format!` with user input in template context | XSS |
| HTTP requests (SSRF) | `reqwest::get`, `Client::get/post` with dynamic URL | SSRF |
| Logging | `tracing::info!`, `log::info!`, `println!` with secrets | Information leak |

MVP starts with SQL execution, shell execution, and file writes.

### Sanitizer Recognition

A **sanitizer** is a function that neutralizes taint.

| Sanitizer Pattern | What It Neutralizes |
|---|---|
| `escape`, `sanitize`, `encode` in function name | General sanitization |
| `validate`, `verify` in function name | Input validation |
| Parameterized query usage (e.g., `sqlx::query!`) | SQL injection |

For MVP, name-based heuristics. Phase 3 upgrades to interprocedural summary-based
recognition.

### Taint Propagation Algorithm (MVP)

1. **Mark sources**: Walk all symbols. Mark entrypoints with handler attributes
   as taint sources. Mark functions calling known source APIs as taint sources.

2. **Mark sinks**: Walk all symbols and their callees. Mark functions calling
   known sink APIs.

3. **Find tainted paths**: For each (source, sink) pair, use `find_paths()` on
   the call graph with `max_depth = 10`.

4. **Check for sanitizers**: For each found path, check if any intermediate
   function matches a sanitizer pattern. If yes, reduce confidence.

5. **Emit findings**: For each unsanitized source-to-sink path, emit a `Finding`
   with:
   - `rule_id`: `security/taint/<sink-type>` (e.g., `security/taint/sql-injection`)
   - `severity`: High (no sanitizer) or Medium (weak sanitizer)
   - `confidence`: High (short path, no sanitizer), Medium (long path), Low (heuristic-only)
   - `call_path`: Vec<SymbolId> of the tainted path
   - `explanation`: "User input from <source> flows through <path> to <sink> without sanitization"

### Confidence Scoring

```
base_confidence = High

if path_length > 5:
    base_confidence -= 1 level

if any_intermediate_function_name_contains("validate", "check"):
    base_confidence -= 1 level

if sink_is_parameterized_query:
    skip finding (safe pattern)
```

---

## Design Decisions

1. **Function-level taint for MVP**: Tracking taint at the variable/expression level
   requires a much more complex data-flow engine (Phase 3). Function-level taint
   is a pragmatic starting point that catches real patterns like
   `handler() -> process() -> execute_sql()`.

2. **Reuse existing call graph**: The call graph in `GraphSet` already connects
   callers to callees. Taint propagation is essentially "can untrusted data reach
   this sink via calls?" which maps directly to `find_paths()`.

3. **Name-based sink/sanitizer matching for MVP**: Using function/method name
   patterns is imprecise but fast. Phase 3 can upgrade to type-aware matching
   using interprocedural summaries.

4. **Emit findings, not SecurityIndicators**: The taint engine is new code and
   should emit `Finding` directly (spec 02), not add more `SecurityIndicator`
   variants.

5. **Existing SecurityIndicator sink detections become building blocks**: The
   filesystem/network/subprocess detections in `security.rs` are reused as
   sink identification, not duplicated.

---

## Tasks

- [ ] **T-03.1** Create `spectron-analysis/src/taint/` module directory with `mod.rs`, `sources.rs`, `sinks.rs`, `sanitizers.rs`, `propagation.rs`
- [ ] **T-03.2** Implement source catalog: identify taint source functions from entrypoints and source API calls
- [ ] **T-03.3** Implement sink catalog: identify taint sink functions from name patterns and existing SecurityIndicator detections
- [ ] **T-03.4** Implement sanitizer pattern matching: name-based heuristics for sanitizer/validator functions
- [ ] **T-03.5** Implement taint propagation: for each (source, sink) pair, find call paths using `find_paths()` on call graph
- [ ] **T-03.6** Implement confidence scoring based on path length, sanitizer presence, sink type
- [ ] **T-03.7** Emit `Finding` for each unsanitized tainted path (depends on T-02.1)
- [ ] **T-03.8** Wire `run_taint_analysis()` into `spectron-analysis::analyze()`
- [ ] **T-03.9** Add unit tests with fixture call graphs containing source -> sink paths with/without sanitizers
- [ ] **T-03.10** Add integration test: analyze a fixture project with a known taint vulnerability (e.g., HTTP param -> SQL query)
