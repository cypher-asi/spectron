# Spectron — Requirements Document

## 1. Product Overview

### 1.1 Purpose

Spectron is a unified, multi-module static code analysis platform that analyzes large Rust codebases for security vulnerabilities, performance inefficiencies, architectural weaknesses, and refactoring opportunities. It combines graph-based program analysis with an interactive desktop GUI to deliver explainable, actionable findings.

### 1.2 Core Principles

| # | Principle | Description |
|---|-----------|-------------|
| 1 | Multi-layer analysis | Cheap structural checks through deep interprocedural analysis, applied incrementally. |
| 2 | Graph-first architecture | All code relationships modeled as a directed graph (petgraph). Analyses are graph traversals. |
| 3 | Interprocedural summaries | Function summaries propagated bottom-up through the call graph for scalable cross-function reasoning. |
| 4 | Explainable findings | Every finding includes a human-readable explanation, call path, and suggested fix — no black boxes. |
| 5 | Diff-aware operation | Focus analysis output on new or changed code to reduce noise in CI pipelines. |
| 6 | Actionable outputs | Findings include severity, confidence, priority ranking, and remediation guidance. |

### 1.3 Target Users

- Rust developers performing code review
- Security engineers auditing Rust services
- Platform/infrastructure teams enforcing architectural standards
- CI/CD pipelines gating merges on analysis thresholds

### 1.4 Scope

**In scope (v1):** Rust codebases only, analyzed via `syn` AST parsing.

**Out of scope (v1):** Other languages, runtime profiling, dynamic analysis, IDE plugin, cloud-hosted service.

---

## 2. System Architecture

### 2.1 High-Level Pipeline

```
CLI arg (path)
  │
  ▼
spectron-loader     →  LoadResult    { project, crates, modules, files }
  │
  ▼
spectron-parser     →  ParseResult   { symbols, relationships, errors }
  │
  ▼
spectron-graph      →  GraphSet      { structure_graph, call_graph, CFGs, index }
  │
  ▼
spectron-analysis   →  AnalysisOutput { metrics, findings, security, entrypoints, summaries }
  │
  ▼
spectron-ui         →  Interactive GUI window
```

### 2.2 Crate Map

| Crate | Responsibility |
|-------|---------------|
| `spectron-core` | Shared domain types: IDs, Symbol, graph types, metrics, security indicators, Finding |
| `spectron-loader` | Project/crate/module/file discovery from Cargo manifests |
| `spectron-parser` | AST parsing via `syn`, symbol + relationship extraction, name resolution |
| `spectron-graph` | Structure graph, call graph, CFG construction, graph algorithms |
| `spectron-analysis` | All analysis modules: metrics, security, taint, auth, cost, structural, resource, ranking, rules |
| `spectron-storage` | Persistence layer for baselines and analysis artifacts |
| `spectron-render` | GPU rendering engine (wgpu) for advanced visualizations |
| `spectron-ui` | Egui-based GUI: graph visualization, filter panel, inspector, specialized views |
| `spectron-app` | CLI entry point with subcommands and output format selection |

### 2.3 Internal Dependency Graph

```
spectron-app
  ├── spectron-core
  ├── spectron-loader     → spectron-core
  ├── spectron-parser     → spectron-core, spectron-loader
  ├── spectron-graph      → spectron-core, spectron-parser, spectron-loader
  ├── spectron-analysis   → spectron-core, spectron-graph
  ├── spectron-storage    → spectron-core
  ├── spectron-render     → spectron-core
  └── spectron-ui         → spectron-core, spectron-parser, spectron-graph, spectron-analysis
```

### 2.4 Technology Stack

| Category | Technology | Version |
|----------|-----------|---------|
| Language | Rust | 2021 edition |
| AST Parsing | `syn` | 2.x (full, parsing, visit features) |
| Graph Engine | `petgraph` | 0.6 |
| GUI Framework | `egui` / `eframe` | 0.28 |
| GPU Backend | `wgpu` | 0.20 |
| Windowing | `winit` | 0.29 |
| CLI Framework | `clap` (derive) | 4.x |
| Serialization | `serde` + `serde_json` + `bincode` | 1.x |
| Persistence | `sled` | 0.34 |
| Async Runtime | `tokio` (full) | 1.x |
| Parallelism | `rayon` | 1.x |
| Logging | `tracing` + `tracing-subscriber` | 0.1 / 0.3 |
| File Walking | `walkdir`, `glob` | 2.x / 0.3 |
| Cargo Parsing | `cargo_toml` | 0.20 |
| Hashing | `sha2` | 0.10 |
| Error Handling | `thiserror`, `anyhow` | 1.x |

---

## 3. Data Model

### 3.1 Identity System

Four strongly-typed ID newtypes sharing a single atomic counter via `IdGenerator`:

- `CrateId(u64)` — uniquely identifies a crate
- `ModuleId(u64)` — uniquely identifies a module
- `FileId(u64)` — uniquely identifies a source file
- `SymbolId(u64)` — uniquely identifies a symbol (function, struct, trait, etc.)
- `FindingId(u64)` — uniquely identifies an analysis finding

All IDs are thread-safe, monotonically allocated, and serde-serializable.

### 3.2 Project Hierarchy

| Type | Fields | Purpose |
|------|--------|---------|
| `ProjectInfo` | name, root_path, is_workspace, crate_ids | Root project metadata |
| `CrateInfo` | id, name, path, crate_type (Library/Binary), module_ids, dependencies | Per-crate metadata |
| `ModuleInfo` | id, name, path, file_path, parent, children, symbol_ids | Per-module metadata |
| `FileInfo` | id, path, hash (SHA-256), line_count | Per-file metadata |
| `ModulePath` | segments | Qualified path like `my_crate::foo::bar` |

### 3.3 Symbol Model

```
Symbol
  ├── id: SymbolId
  ├── name: String
  ├── kind: SymbolKind
  │     { Function, Method, Struct, Enum, Trait, ImplBlock, Constant, Static, TypeAlias }
  ├── module_id: ModuleId
  ├── file_id: FileId
  ├── span: SourceSpan { start_line, start_col, end_line, end_col }
  ├── visibility: Visibility { Public, Crate, Restricted, Private }
  ├── signature: Option<String>
  └── attributes: SymbolAttributes
        ├── is_async: bool
        ├── is_unsafe: bool
        ├── is_extern: bool
        ├── is_test: bool
        ├── has_unsafe_block: bool
        ├── doc_comment: Option<String>
        └── attribute_paths: Vec<String>
```

### 3.4 Graph Model

**Structure Graph** — `ArchGraph = DiGraph<GraphNode, GraphEdge>`

| Node Variants | Edge Kinds (RelationshipKind) |
|--------------|-------------------------------|
| `Crate(CrateId)` | `Contains` — hierarchical parent-child |
| `Module(ModuleId)` | `Imports` — use statements |
| `File(FileId)` | `Calls` — function/method invocations |
| `Symbol(SymbolId)` | `References` — type/symbol references |
| `ExternalResource(String)` *(planned)* | `Implements` — trait implementations |
| | `DependsOn` — crate-level dependencies |
| | `TaintFlow` *(planned)* — taint propagation |
| | `DataFlow` *(planned)* — data flow edges |

**Call Graph** — Functions/methods only, `Calls` edges only. Accompanied by `CallGraphData` providing O(1) caller/callee lookup per `SymbolId`.

**Control Flow Graph (per-function):**

| CFG Nodes | CFG Edges |
|-----------|-----------|
| Entry | Sequential |
| Exit | TrueBranch |
| Statement | FalseBranch |
| Branch | LoopBack |
| Loop | LoopExit |
| Await | |
| Return | |

### 3.5 Unified Finding Model

Every analysis result is represented as a `Finding`:

```
Finding
  ├── id: FindingId
  ├── rule_id: String              — e.g., "security/taint/sql-injection"
  ├── severity: Severity           — Critical, High, Medium, Low, Info
  ├── confidence: Confidence       — High (90-100%), Medium (60-89%), Low (<60%)
  ├── category: FindingCategory    — Security, Performance, Architecture, Quality, Resource
  ├── location: FindingLocation
  │     ├── file_id, file_path, span
  │     ├── symbol_id
  │     └── module_id
  ├── call_path: Option<Vec<SymbolId>>
  ├── title: String
  ├── explanation: String          — mandatory; enforces "no black box" principle
  ├── suggested_fix: Option<String>
  ├── reachability: Reachability   — Public, Internal, Unknown
  └── metadata: HashMap<String, String>
```

**Rule ID convention:** `<category>/<module>/<check-name>` (e.g., `security/taint/sql-injection`, `perf/cost/db-in-loop`, `arch/structure/cyclic-dependency`).

### 3.6 Metrics

**Per-symbol:**

| Metric | Description |
|--------|-------------|
| Cyclomatic complexity | Number of linearly independent paths |
| Line count | Source lines |
| Parameter count | Function/method parameters |
| Fan-in | Number of callers |
| Fan-out | Number of callees |

**Per-module:**

| Metric | Description |
|--------|-------------|
| Symbol count | Total symbols in module |
| Line count | Total lines |
| Fan-in / Fan-out | Cross-module references |
| Instability | fan_out / (fan_in + fan_out) |
| Cohesion | internal_references / total_references |
| Coupling score | fan_in + fan_out |
| API surface ratio | public_symbols / total_symbols |

### 3.7 Function Summary (Interprocedural)

```
FunctionSummary
  ├── symbol_id: SymbolId
  ├── side_effects: Vec<SideEffect>
  │     { FileRead, FileWrite, NetworkCall, DbQuery, DbWrite,
  │       SubprocessExec, Logging, Allocation, LockAcquire, Panic }
  ├── direct_cost: f32
  ├── total_cost: f32
  ├── cost_breakdown: Vec<(CostType, f32)>
  ├── taint_sources: bool
  ├── taint_propagates: bool
  ├── taint_sanitizes: bool
  ├── taint_sinks: Vec<String>
  ├── requires_auth: bool
  ├── provides_auth: bool
  ├── resources_acquired: Vec<ResourceKind>
  ├── resources_released: Vec<ResourceKind>
  ├── holds_lock: bool
  ├── has_await: bool
  ├── is_pure: bool
  └── max_call_depth: u32
```

---

## 4. Functional Requirements

### 4.1 Project Loading (FR-LOAD)

| ID | Requirement | Priority |
|----|-------------|----------|
| FR-LOAD-01 | Load single Rust crate from a path containing `Cargo.toml` | Must |
| FR-LOAD-02 | Load Rust workspace (multi-crate) from a workspace root `Cargo.toml` | Must |
| FR-LOAD-03 | Discover all crate targets (lib, bin) including dual-target crates | Must |
| FR-LOAD-04 | Discover all modules via `mod` declarations (line scanning) | Must |
| FR-LOAD-05 | Discover all source files and compute SHA-256 content hashes | Must |
| FR-LOAD-06 | Record line count per file | Must |
| FR-LOAD-07 | Skip `build.rs`, `tests/`, `examples/`, `benches/` directories | Must |
| FR-LOAD-08 | Handle malformed Cargo.toml gracefully with error reporting | Must |
| FR-LOAD-09 | Handle dual-target crates with module deduplication | Must |
| FR-LOAD-10 | Produce `LoadResult` containing project, crates, modules, and files | Must |

### 4.2 Parsing & Symbol Extraction (FR-PARSE)

| ID | Requirement | Priority |
|----|-------------|----------|
| FR-PARSE-01 | Parse all Rust source files using `syn` with full AST support | Must |
| FR-PARSE-02 | Extract symbols: Function, Method, Struct, Enum, Trait, ImplBlock, Constant, Static, TypeAlias | Must |
| FR-PARSE-03 | Record symbol visibility (Public, Crate, Restricted, Private) | Must |
| FR-PARSE-04 | Record symbol attributes: async, unsafe, extern, test, unsafe_block, doc_comments, attribute_paths | Must |
| FR-PARSE-05 | Record symbol signature (function signature string) | Must |
| FR-PARSE-06 | Record source span (start_line, start_col, end_line, end_col) per symbol | Must |
| FR-PARSE-07 | Extract relationships: imports, calls, references, implements | Must |
| FR-PARSE-08 | Resolve names to SymbolIds via SymbolTable (ModuleId + name → SymbolId) | Must |
| FR-PARSE-09 | Parse files in parallel using rayon | Must |
| FR-PARSE-10 | Report parse errors with file path, message, and optional span | Must |
| FR-PARSE-11 | Assign unresolved symbols to a sentinel `UNRESOLVED_MODULE_ID` | Should |

### 4.3 Graph Construction (FR-GRAPH)

| ID | Requirement | Priority |
|----|-------------|----------|
| FR-GRAPH-01 | Build structure graph containing all crates, modules, files, and symbols as nodes | Must |
| FR-GRAPH-02 | Build structure graph edges: Contains, Imports, Implements, References, DependsOn | Must |
| FR-GRAPH-03 | Build call graph containing only Function/Method nodes with Calls edges | Must |
| FR-GRAPH-04 | Provide O(1) caller/callee lookup per SymbolId via `CallGraphData` | Must |
| FR-GRAPH-05 | Build per-function control flow graphs with Entry, Exit, Statement, Branch, Loop, Await, Return nodes | Must |
| FR-GRAPH-06 | Provide domain ID → NodeIndex mapping via `GraphIndex` | Must |
| FR-GRAPH-07 | Implement path finding: all simple paths between two nodes with max depth | Must |
| FR-GRAPH-08 | Implement BFS reachability (descendants from a root node) | Must |
| FR-GRAPH-09 | Implement induced subgraph extraction from reachable node sets | Must |
| FR-GRAPH-10 | Implement connected component detection | Must |
| FR-GRAPH-11 | Implement Tarjan's SCC algorithm for directed cycle detection | Must |
| FR-GRAPH-12 | Implement topological sort for bottom-up propagation | Must |
| FR-GRAPH-13 | Add `ExternalResource(String)` node variant for modeling sinks (DB, network, FS) | Should |
| FR-GRAPH-14 | Add `TaintFlow` and `DataFlow` edge variants | Should |

### 4.4 Metrics & Complexity Analysis (FR-METRIC)

| ID | Requirement | Priority |
|----|-------------|----------|
| FR-METRIC-01 | Compute cyclomatic complexity per function from CFG | Must |
| FR-METRIC-02 | Compute line count per function | Must |
| FR-METRIC-03 | Compute parameter count per function | Must |
| FR-METRIC-04 | Compute fan-in (caller count) and fan-out (callee count) per symbol | Must |
| FR-METRIC-05 | Compute symbol count, line count, fan-in, fan-out per module | Must |
| FR-METRIC-06 | Flag high cyclomatic complexity (threshold: configurable, default ≥ 15) | Must |
| FR-METRIC-07 | Flag large functions (threshold: configurable, default ≥ 100 lines) | Must |
| FR-METRIC-08 | Flag large modules (threshold: configurable, default ≥ 50 symbols) | Must |
| FR-METRIC-09 | Flag high fan-in modules (threshold: configurable, default ≥ 20) | Must |
| FR-METRIC-10 | Flag high fan-out modules (threshold: configurable, default ≥ 15) | Must |
| FR-METRIC-11 | Convert complexity flags to unified `Finding` instances | Must |

### 4.5 Security Indicator Detection (FR-SEC)

| ID | Requirement | Priority |
|----|-------------|----------|
| FR-SEC-01 | Detect `unsafe` blocks with source span | Must |
| FR-SEC-02 | Detect `unsafe` functions | Must |
| FR-SEC-03 | Detect FFI calls with extern name | Must |
| FR-SEC-04 | Detect filesystem access calls (`std::fs::*`, `tokio::fs::*`) | Must |
| FR-SEC-05 | Detect network access calls (`std::net::*`, `tokio::net::*`, `reqwest::*`, `hyper::*`) | Must |
| FR-SEC-06 | Detect subprocess execution (`std::process::Command`, `tokio::process::Command`) | Must |
| FR-SEC-07 | Aggregate indicators into `SecurityReport` | Must |
| FR-SEC-08 | Convert security indicators to unified `Finding` instances | Must |

### 4.6 Entrypoint Detection (FR-ENTRY)

| ID | Requirement | Priority |
|----|-------------|----------|
| FR-ENTRY-01 | Detect `main` function in root module | Must |
| FR-ENTRY-02 | Detect async main attributes (`#[tokio::main]`, `#[async_std::main]`, `#[actix_web::main]`) | Must |
| FR-ENTRY-03 | Detect HTTP handler attributes (`#[get]`, `#[post]`, `#[put]`, `#[delete]`, `#[handler]`) | Must |
| FR-ENTRY-04 | Detect test functions (`is_test = true`) | Must |
| FR-ENTRY-05 | Detect CLI command attributes (`#[command]`) | Must |
| FR-ENTRY-06 | Detect functions with zero callers (library API surfaces) | Must |

### 4.7 Security Taint Engine (FR-TAINT)

| ID | Requirement | Priority |
|----|-------------|----------|
| FR-TAINT-01 | Identify taint source functions from entrypoints with handler attributes | Must |
| FR-TAINT-02 | Identify taint source functions from calls to known source APIs (file reads, env vars, external APIs) | Must |
| FR-TAINT-03 | Identify taint sink functions: SQL execution, shell execution, file writes | Must |
| FR-TAINT-04 | Identify taint sink functions: template rendering, HTTP requests (SSRF), logging | Should |
| FR-TAINT-05 | Propagate taint: for each (source, sink) pair, find call paths on call graph (max depth 10) | Must |
| FR-TAINT-06 | Recognize sanitizer functions via name patterns (`escape`, `sanitize`, `encode`, `validate`, `verify`) | Must |
| FR-TAINT-07 | Recognize parameterized query usage as safe pattern (skip finding) | Should |
| FR-TAINT-08 | Score confidence: High (short path, no sanitizer), Medium (long path), Low (heuristic-only) | Must |
| FR-TAINT-09 | Reduce confidence when intermediate function names suggest validation | Must |
| FR-TAINT-10 | Emit `Finding` per unsanitized tainted path with rule_id, severity, call_path, explanation | Must |

### 4.8 Auth & Trust Boundary Analysis (FR-AUTH)

| ID | Requirement | Priority |
|----|-------------|----------|
| FR-AUTH-01 | Detect auth guard functions via name patterns (`auth`, `require`, `guard`, `authorize`, `has_role`, etc.) | Must |
| FR-AUTH-02 | Detect auth guard functions via attribute patterns (`#[guard]`, `#[middleware]`, `#[require_role]`) | Should |
| FR-AUTH-03 | Classify entrypoints by trust level: External (HTTP handlers, CLI), Internal (zero-caller APIs), Test | Must |
| FR-AUTH-04 | For each external entrypoint, check if any auth guard is reachable via call graph | Must |
| FR-AUTH-05 | Emit `Finding` for entrypoints missing auth guards (High severity for external, Medium for internal) | Must |
| FR-AUTH-06 | Upgrade to path-sensitive must-call analysis (every path, not just reachability) | Should |
| FR-AUTH-07 | Support user-defined guard function names via configuration | Should |

### 4.9 Performance Cost Analysis (FR-COST)

| ID | Requirement | Priority |
|----|-------------|----------|
| FR-COST-01 | Define cost types: Allocation, Lock, IoRead, IoWrite, NetworkCall, DbQuery, Serialization, Clone | Must |
| FR-COST-02 | Assign default weight table (Allocation=1.0, Lock=2.0, IO=5.0, Network/DB=10.0, Serialization=2.0, Clone=1.0) | Must |
| FR-COST-03 | Detect expensive operations by matching callee names to cost patterns | Must |
| FR-COST-04 | Propagate costs bottom-up through call graph in reverse topological order | Must |
| FR-COST-05 | Compute per-function `FunctionCost` (direct, propagated, total, breakdown) | Must |
| FR-COST-06 | Handle recursive call cycles with capped propagation depth | Must |
| FR-COST-07 | Detect N+1 query pattern: DB calls inside loop bodies | Must |
| FR-COST-08 | Detect network calls inside loop bodies | Must |
| FR-COST-09 | Detect repeated allocations inside loop bodies | Should |
| FR-COST-10 | Rank hotspots by total cost; report top-N (configurable, default 20) | Must |
| FR-COST-11 | Emit findings for hotspots with severity based on cost thresholds (Critical>100, High>50, Medium>20) | Must |

### 4.10 Structural & Architecture Analysis (FR-ARCH)

| ID | Requirement | Priority |
|----|-------------|----------|
| FR-ARCH-01 | Detect cyclic module dependencies using SCC on module-level Imports/DependsOn subgraph | Must |
| FR-ARCH-02 | Detect cyclic crate dependencies on crate-level DependsOn subgraph (Critical severity) | Must |
| FR-ARCH-03 | Emit cycle findings with severity based on cycle size (Medium for 2, High for 3+) | Must |
| FR-ARCH-04 | Compute coupling metrics per module: instability, cohesion, coupling_score | Must |
| FR-ARCH-05 | Detect god modules: large symbol count AND high coupling score | Must |
| FR-ARCH-06 | Compute API surface ratio (public/total symbols) per module | Should |
| FR-ARCH-07 | Flag modules with excessive API surface (>80% public) | Should |
| FR-ARCH-08 | Support layer definitions via TOML configuration | Should |
| FR-ARCH-09 | Detect layer violations: imports from lower to upper layer | Should |

### 4.11 Resource & Lifetime Analysis (FR-RES)

| ID | Requirement | Priority |
|----|-------------|----------|
| FR-RES-01 | Define resource kinds: File, DbConnection, Lock, Transaction, TcpStream, Process | Must |
| FR-RES-02 | Define resource transitions: Acquire, Release, Use | Must |
| FR-RES-03 | Maintain resource pattern catalog mapping API calls to acquire/release transitions | Must |
| FR-RES-04 | Detect lock held across `.await` in async functions (High severity) | Must |
| FR-RES-05 | Detect transaction begin without commit/rollback (Medium severity) | Must |
| FR-RES-06 | Detect function-level resource acquire without corresponding release in same scope | Should |
| FR-RES-07 | Account for Rust RAII/Drop semantics to reduce false positives | Must |

### 4.12 Interprocedural Function Summaries (FR-SUMM)

| ID | Requirement | Priority |
|----|-------------|----------|
| FR-SUMM-01 | Compute `FunctionSummary` for every function/method in the call graph | Must |
| FR-SUMM-02 | Compute leaf-function summaries from direct pattern matching | Must |
| FR-SUMM-03 | Propagate summaries bottom-up in reverse topological order | Must |
| FR-SUMM-04 | Handle call graph cycles (SCCs) via iterative fixed-point with capped iterations | Must |
| FR-SUMM-05 | Merge callee summaries: union side effects, sum costs, propagate taint/auth/resource info | Must |
| FR-SUMM-06 | Refactor taint, cost, and resource analyzers to query summaries instead of re-traversing | Should |
| FR-SUMM-07 | Support incremental summary updates for changed functions only | Should |

### 4.13 Finding Ranking & Prioritization (FR-RANK)

| ID | Requirement | Priority |
|----|-------------|----------|
| FR-RANK-01 | Compute priority score (0-100) per finding from weighted factors | Must |
| FR-RANK-02 | Factor: Reachability (weight 0.25) — Public=1.0, Internal=0.5, Unknown=0.3 | Must |
| FR-RANK-03 | Factor: Exploitability (weight 0.20) — derived from finding category and rule_id | Must |
| FR-RANK-04 | Factor: Cost impact (weight 0.15) — from function total_cost | Must |
| FR-RANK-05 | Factor: Centrality (weight 0.15) — from fan_in metric | Must |
| FR-RANK-06 | Factor: Confidence (weight 0.15) — High=1.0, Medium=0.6, Low=0.3 | Must |
| FR-RANK-07 | Factor: Severity (weight 0.10) — Critical=1.0 through Info=0.1 | Must |
| FR-RANK-08 | Pre-compute reachability via BFS from external entrypoints; populate Finding.reachability | Must |
| FR-RANK-09 | Sort findings by total_score descending in AnalysisOutput | Must |
| FR-RANK-10 | Expose score breakdown per finding for UI display | Must |
| FR-RANK-11 | Support configurable factor weights | Should |
| FR-RANK-12 | Add code churn factor from git history | Should |

### 4.14 Rule Engine (FR-RULE)

| ID | Requirement | Priority |
|----|-------------|----------|
| FR-RULE-01 | Define `Rule` type with id, name, description, category, default_severity, enabled flag, config | Must |
| FR-RULE-02 | Support rule config types: Threshold, Pattern, MustCall, Layer, Custom | Must |
| FR-RULE-03 | Implement `RuleRegistry` with register, get, enable/disable, filter by category | Must |
| FR-RULE-04 | Register all existing hardcoded checks as built-in Rule instances | Must |
| FR-RULE-05 | Refactor metrics, security, and entrypoint modules to read config from RuleRegistry | Must |
| FR-RULE-06 | Load rule overrides from `.spectron.toml` configuration file | Must |
| FR-RULE-07 | Support disabling rules, changing thresholds, adding custom patterns via config | Must |
| FR-RULE-08 | Provide `--list-rules` CLI flag to print all registered rules | Should |
| FR-RULE-09 | Provide `--disable-rule` and `--enable-rule` CLI flags | Should |
| FR-RULE-10 | Design and implement rule DSL for complex cross-module constraints | Could |

### 4.15 Diff-Aware Analysis & CI Integration (FR-CI)

| ID | Requirement | Priority |
|----|-------------|----------|
| FR-CI-01 | Define `Baseline` type: timestamp, project name, file hashes, findings, summary | Must |
| FR-CI-02 | Save baseline as JSON file (`.spectron-baseline.json`) | Must |
| FR-CI-03 | Load baseline and compute `ChangeSet` (added, removed, modified, unchanged files) via hash comparison | Must |
| FR-CI-04 | Filter findings to changed files only (direct diff mode) | Must |
| FR-CI-05 | Filter findings to changed files + transitive callers (transitive diff mode) | Should |
| FR-CI-06 | Compare current findings against baseline to produce `RegressionReport` (new, fixed, unchanged) | Must |
| FR-CI-07 | Match findings via rule_id + location; fallback to fuzzy match on rule_id + symbol name | Must |
| FR-CI-08 | Output findings in SARIF format | Must |
| FR-CI-09 | Output findings in GitHub Annotations format | Should |
| FR-CI-10 | Output findings in JUnit XML format | Should |
| FR-CI-11 | Support `--fail-on <severity>` for CI quality gates (exit code 2) | Must |
| FR-CI-12 | Support `--max-findings <n>` threshold (exit code 2) | Must |
| FR-CI-13 | Exit code 3 when regressions detected vs baseline | Must |
| FR-CI-14 | Reduce noise by >80% in diff mode compared to full analysis | Must |

---

## 5. User Interface Requirements

### 5.1 Application Shell (FR-UI)

| ID | Requirement | Priority |
|----|-------------|----------|
| FR-UI-01 | Dark theme with black background | Must |
| FR-UI-02 | Top header bar with view mode tabs/buttons | Must |
| FR-UI-03 | Left sidebar: crate/module tree with search | Must |
| FR-UI-04 | Central area: graph canvas or list view depending on mode | Must |
| FR-UI-05 | Right panel: filter panel and inspector | Must |

### 5.2 View Modes

| ID | View Mode | Description | Priority |
|----|-----------|-------------|----------|
| FR-VIEW-01 | Overview | Stats dashboard: crate/module/file/symbol/function/struct/trait counts, total lines, entrypoints, flag count | Must |
| FR-VIEW-02 | Structure Graph | Full architecture graph: all node types, all edge types, interactive canvas | Must |
| FR-VIEW-03 | Call Graph | Functions/methods only with call edges, interactive canvas | Must |
| FR-VIEW-04 | Module Detail | Single module detail panel (accessible from sidebar) | Must |
| FR-VIEW-05 | Findings List | Sortable table of all findings with priority, severity, category, rule, title, location, confidence columns | Must |
| FR-VIEW-06 | Security View | Call graph with taint overlay (sources=red, sinks=orange, sanitizers=green, taint edges=red), trust boundary overlay | Must |
| FR-VIEW-07 | Performance View | Call graph with cost heatmap (blue→red gradient), node size proportional to cost, N+1 markers, top-N hotspot sidebar | Must |
| FR-VIEW-08 | Architecture View | Module-only subgraph with SCC cycle highlighting (red edges/borders), optional layer coloring and violation highlighting | Must |

### 5.3 Graph Canvas (FR-CANVAS)

| ID | Requirement | Priority |
|----|-------------|----------|
| FR-CANVAS-01 | Pan via mouse drag | Must |
| FR-CANVAS-02 | Zoom via mouse scroll | Must |
| FR-CANVAS-03 | Node selection via click | Must |
| FR-CANVAS-04 | Node dragging | Must |
| FR-CANVAS-05 | Color-coded nodes by type (Crate, Module, File, Symbol subtypes) | Must |
| FR-CANVAS-06 | Color-coded edges by relationship kind | Must |
| FR-CANVAS-07 | Entrypoint halos | Must |
| FR-CANVAS-08 | Selection rings and hover highlights | Must |
| FR-CANVAS-09 | Node tooltips on hover | Must |
| FR-CANVAS-10 | Force-directed layout (Fruchterman-Reingold with simulated annealing, 200 iterations) | Must |
| FR-CANVAS-11 | Layered layout option | Should |

### 5.4 Filter Panel (FR-FILTER)

| ID | Requirement | Priority |
|----|-------------|----------|
| FR-FILTER-01 | Presets: Dependencies, Modules, Call Flow, Type Graph, Imports, Everything | Must |
| FR-FILTER-02 | Filter by node type (Crate, Module, File, Symbol) | Must |
| FR-FILTER-03 | Filter by symbol kind (Function, Method, Struct, Enum, Trait, etc.) | Must |
| FR-FILTER-04 | Filter by edge type (Contains, Imports, Calls, References, Implements, DependsOn) | Must |
| FR-FILTER-05 | Filter by visibility (Public, Crate, Restricted, Private) | Must |
| FR-FILTER-06 | Filter by crate | Must |
| FR-FILTER-07 | Isolate modes: entrypoints only, unsafe only, flagged only | Must |
| FR-FILTER-08 | Adapt filter options to current view mode (security filters in security view, etc.) | Should |

### 5.5 Inspector Panel (FR-INSPECT)

| ID | Requirement | Priority |
|----|-------------|----------|
| FR-INSPECT-01 | Symbol detail: name, kind, signature, visibility, module, file, span, metrics, flags, security attributes, callers, callees | Must |
| FR-INSPECT-02 | Module detail: name, path, file, parent, children, symbols, module metrics | Must |
| FR-INSPECT-03 | Crate detail: name, type, path, modules, dependencies | Must |
| FR-INSPECT-04 | Finding detail: title, explanation, severity/confidence badges, rule_id, location, call path (clickable), suggested fix, reachability, priority score breakdown | Must |

---

## 6. CLI Requirements

### 6.1 Commands & Flags

| Command / Flag | Description | Priority |
|----------------|-------------|----------|
| `spectron <path>` | Run full pipeline and open GUI | Must |
| `spectron <path> --json` | Output full analysis results as JSON | Must |
| `spectron <path> --cli` | Print structure tree to stdout | Must |
| `spectron <path> --findings` | Print findings as JSON | Must |
| `spectron analyze <path> --security` | Run security-focused analysis | Should |
| `spectron analyze <path> --perf` | Run performance-focused analysis | Should |
| `--baseline <path>` | Load baseline for comparison | Must |
| `--save-baseline <path>` | Save current results as baseline | Must |
| `--diff-mode <mode>` | `direct`, `transitive`, `full` (default: `full`) | Must |
| `--format <format>` | `json`, `sarif`, `junit`, `github` (default: `json`) | Must |
| `--fail-on <severity>` | Exit non-zero if findings at this severity or above | Must |
| `--max-findings <n>` | Exit non-zero if more than N findings | Must |
| `--list-rules` | Print all registered rules with status | Should |
| `--disable-rule <id>` | Disable a specific rule | Should |
| `--enable-rule <id>` | Enable a specific rule | Should |

### 6.2 Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success, no threshold violations |
| 1 | Analysis error |
| 2 | Findings exceed threshold (`--fail-on` or `--max-findings`) |
| 3 | Regressions detected (new findings vs baseline) |

---

## 7. Configuration

### 7.1 Project Configuration (`.spectron.toml`)

Located in the project root. Supports:

```toml
[rules]
# Disable a rule
"quality/complexity/high-cyclomatic".enabled = false

# Change a threshold
"quality/complexity/large-function".threshold = 150

# Add custom pattern rules
[rules."security/api/custom-sensitive"]
category = "security"
severity = "high"
patterns = ["my_company::internal_api::"]
match_type = "prefix"
description = "Calls to internal sensitive API"

# Architectural layer definitions
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

---

## 8. Built-in Rule Catalog

| Rule ID | Category | Type | Default Severity | Description |
|---------|----------|------|-----------------|-------------|
| `quality/complexity/high-cyclomatic` | Quality | Threshold ≥15 | Medium | Cyclomatic complexity exceeds threshold |
| `quality/complexity/large-function` | Quality | Threshold ≥100 lines | Low | Function exceeds line count threshold |
| `quality/complexity/large-module` | Quality | Threshold ≥50 symbols | Low | Module exceeds symbol count threshold |
| `quality/coupling/high-fan-in` | Quality | Threshold ≥20 | Low | Module has excessive incoming dependencies |
| `quality/coupling/high-fan-out` | Quality | Threshold ≥15 | Low | Module has excessive outgoing dependencies |
| `security/code/unsafe-function` | Security | Pattern | Medium | Function declared as unsafe |
| `security/code/unsafe-block` | Security | Pattern | Medium | Unsafe block detected |
| `security/boundary/ffi-call` | Security | Pattern | High | Foreign function interface call |
| `security/api/filesystem-access` | Security | Pattern | Medium | Filesystem API access |
| `security/api/network-access` | Security | Pattern | Medium | Network API access |
| `security/api/subprocess-exec` | Security | Pattern | High | Subprocess execution |
| `security/taint/sql-injection` | Security | Pattern | Critical | Unsanitized input reaches SQL execution |
| `security/taint/command-injection` | Security | Pattern | Critical | Unsanitized input reaches shell execution |
| `security/taint/ssrf` | Security | Pattern | High | Unsanitized input used in outbound HTTP request |
| `security/auth/missing-guard` | Security | MustCall | High | HTTP handler missing authorization check |
| `perf/cost/db-in-loop` | Performance | Pattern | High | Database query inside loop body (N+1) |
| `perf/cost/network-in-loop` | Performance | Pattern | High | Network call inside loop body |
| `perf/cost/allocation-in-loop` | Performance | Pattern | Medium | Heap allocation inside loop body |
| `perf/cost/hotspot` | Performance | Threshold | Varies | Function exceeds cost threshold |
| `arch/structure/cyclic-dependency` | Architecture | Pattern | Medium–Critical | Cyclic dependency between modules or crates |
| `arch/structure/layer-violation` | Architecture | Layer | High | Import violates layer ordering |
| `arch/structure/excessive-api-surface` | Architecture | Threshold >0.8 | Low | Module exposes too many public symbols |
| `arch/structure/god-module` | Architecture | Threshold | Medium | Module is too large and highly coupled |
| `resource/lock/held-across-await` | Resource | Pattern | High | Mutex guard potentially held across await point |
| `resource/transaction/uncommitted` | Resource | Pattern | Medium | Transaction opened without commit/rollback |

---

## 9. Non-Functional Requirements

### 9.1 Performance

| ID | Requirement | Target |
|----|-------------|--------|
| NFR-PERF-01 | Full analysis of a medium Rust project (50k LOC) | < 5 minutes |
| NFR-PERF-02 | Diff-mode analysis reduces reported findings | > 80% noise reduction |
| NFR-PERF-03 | File parsing leverages multi-core parallelism | rayon-based |
| NFR-PERF-04 | GUI maintains interactive frame rate during graph rendering | ≥ 30 FPS |
| NFR-PERF-05 | Graph layout completes in bounded time | 200 iterations max |

### 9.2 Correctness

| ID | Requirement |
|----|-------------|
| NFR-CORR-01 | All findings must include a valid explanation (no empty explanations) |
| NFR-CORR-02 | Security taint analysis must not produce false negatives for direct source→sink paths |
| NFR-CORR-03 | RAII/Drop semantics must be considered to reduce false positives in resource analysis |
| NFR-CORR-04 | SCC cycle detection must find all directed cycles (no missed cycles) |
| NFR-CORR-05 | Cost propagation must handle recursive calls without infinite loops |

### 9.3 Usability

| ID | Requirement |
|----|-------------|
| NFR-USE-01 | Findings list sorted by priority as default view entry point |
| NFR-USE-02 | Each specialized view uses a distinct color palette for visual clarity |
| NFR-USE-03 | Inspector panel provides drill-down from any finding to source location |
| NFR-USE-04 | Filter panel adapts to the current view mode |
| NFR-USE-05 | CLI output formats are machine-parseable for CI integration |

### 9.4 Extensibility

| ID | Requirement |
|----|-------------|
| NFR-EXT-01 | New analysis modules added as submodules of `spectron-analysis` |
| NFR-EXT-02 | New findings emitted through unified `Finding` type |
| NFR-EXT-03 | Rule registry supports runtime registration of new rules |
| NFR-EXT-04 | Function summary struct uses `Option<T>` for incrementally-added fields |
| NFR-EXT-05 | Pattern catalogs (taint sources/sinks, cost patterns, resource APIs) are data-driven and extensible |

### 9.5 Reliability

| ID | Requirement |
|----|-------------|
| NFR-REL-01 | Malformed Cargo.toml files produce error diagnostics, not panics |
| NFR-REL-02 | Parse errors in individual files do not abort analysis of remaining files |
| NFR-REL-03 | All analysis modules return `Result<T, SpectronError>` |
| NFR-REL-04 | Unresolved symbols tracked as diagnostics, not crashes |

### 9.6 Serialization & Interoperability

| ID | Requirement |
|----|-------------|
| NFR-SER-01 | All core domain types implement `Serialize`/`Deserialize` |
| NFR-SER-02 | `GraphSet` and `AnalysisOutput` are fully serializable for caching |
| NFR-SER-03 | Baseline format is human-inspectable JSON |
| NFR-SER-04 | SARIF output conforms to SARIF 2.1.0 specification |

---

## 10. Phased Delivery Roadmap

### Phase 1 — Foundation (Mostly Complete)

- Rust parser integration via `syn`
- Symbol index with 9 symbol kinds
- Module graph, call graph, CFG
- Metrics and complexity flags
- Entrypoint detection (6 rules)
- Security indicator detection
- CLI with `--json` and `--cli` modes
- Interactive GUI with graph visualization
- **Remaining:** Unified findings system, structural cycle detection, loop-based performance detections

### Phase 2 — Security Core

- Unified `Finding` type and evidence system
- Security taint engine (source→sink tracking)
- Auth & trust boundary analyzer (must-call enforcement)
- Convert existing security indicators and complexity flags to unified findings

### Phase 3 — Interprocedural Intelligence

- Function summary computation (bottom-up propagation)
- SCC fixed-point for recursive functions
- Refactor all analysis modules to query summaries
- Variable/field-level symbol extraction (for data flow)

### Phase 4 — Performance & Resource Depth

- Performance cost model and propagation
- N+1 / loop-based cost detection
- Hotspot ranking
- Resource lifecycle tracking
- Lock-across-await detection
- Transaction lifecycle analysis

### Phase 5 — Architecture & Policy Engine

- Rule engine with `RuleRegistry`
- Built-in rule catalog (all hardcoded checks extracted)
- `.spectron.toml` configuration file support
- Layer violation detection
- Coupling/cohesion metrics
- God module detection
- Custom org rules via configuration

### Phase 6 — Diff-Aware & CI Integration

- Baseline save/load system
- File hash-based change detection
- Diff-aware finding filtering (direct and transitive modes)
- Regression detection (new/fixed/unchanged)
- SARIF, JUnit, GitHub Annotations output formats
- CI exit codes and quality gates
- `spectron-storage` activation

### Phase 7 — Advanced (Future)

- Rule DSL for complex graph-pattern constraints
- True incremental analysis (re-parse only changed files)
- GPU-accelerated rendering via `spectron-render`
- Multi-language support
- Use-after-release detection via data-flow analysis
- Interprocedural resource tracking
- Code churn factor from git history
- ML-based ranking models

---

## 11. Testing Strategy

### 11.1 Unit Tests

Every crate contains `#[cfg(test)] mod tests` blocks covering:

- ID generation (concurrent allocation, type safety, serde roundtrip)
- Symbol extraction correctness
- Graph construction edge cases
- Metric computation accuracy
- Pattern matching for taint sources/sinks, cost patterns, resource APIs
- Rule registration and configuration
- Score computation and factor weighting

### 11.2 Integration Tests

| Test Suite | Location | Coverage |
|-----------|----------|----------|
| Loader integration | `spectron-loader/tests/integration_tests.rs` | Single crate, workspace, dual-target, malformed inputs |
| Parser integration | `spectron-parser/tests/integration_tests.rs` | Symbol extraction, relationship resolution, parse errors |
| Full pipeline | Workspace root | End-to-end: load → parse → graph → analyze → assert non-empty output |
| Taint engine | `spectron-analysis` | Fixture project with known taint vulnerability |
| Auth analysis | `spectron-analysis` | Fixture with HTTP handler missing auth check |
| Cost analysis | `spectron-analysis` | Fixture with N+1 query pattern |
| Structural analysis | `spectron-analysis` | Fixture workspace with cyclic module imports |
| Baseline/diff | `spectron-storage` | Save baseline, modify fixture, re-analyze, verify regression report |

### 11.3 Test Fixtures

| Fixture | Path | Purpose |
|---------|------|---------|
| `single_crate` | `spectron-loader/tests/fixtures/` | Single library crate |
| `single_crate_both` | `spectron-loader/tests/fixtures/` | Library + binary targets |
| `workspace` | `spectron-loader/tests/fixtures/` | Multi-crate workspace |
| `malformed/*` | `spectron-loader/tests/fixtures/` | Invalid configs (missing member, bad TOML, etc.) |
| `with_parse_error` | `spectron-parser/tests/fixtures/` | Parse error test cases |

---

## 12. Glossary

| Term | Definition |
|------|-----------|
| **ArchGraph** | The main directed graph (`petgraph::DiGraph<GraphNode, GraphEdge>`) representing code structure |
| **Call graph** | Subgraph containing only Function/Method nodes with Calls edges |
| **CFG** | Control flow graph; built per-function with branch/loop/await/return nodes |
| **Entrypoint** | A function that serves as an entry to the program (main, HTTP handler, test, etc.) |
| **Finding** | A unified analysis result with severity, confidence, explanation, and location |
| **Function summary** | Interprocedural summary capturing a function's side effects, cost, taint, auth, and resource behavior |
| **N+1 pattern** | Database query executed inside a loop, causing one query per iteration |
| **RAII** | Resource Acquisition Is Initialization; Rust pattern where resources are released on Drop |
| **SCC** | Strongly Connected Component; a maximal set of mutually reachable nodes (used for cycle detection) |
| **Structure graph** | The full ArchGraph with all node types and all edge types |
| **Taint** | Data originating from untrusted input that has not been sanitized |
| **Trust boundary** | Classification of entrypoints by exposure level (External, Internal, Test) |
