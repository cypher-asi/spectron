# 00 -- System Overview

## PRD Reference

Spectra (internally **Spectron**) is a unified, multi-module code analysis platform
that analyzes large Rust codebases for security vulnerabilities, performance
inefficiencies, architectural weaknesses, and refactor opportunities.

### Core Principles (from PRD)

1. Multi-layer analysis (cheap to deep)
2. Graph-first architecture
3. Interprocedural summaries (scalable)
4. Explainable findings (no black box)
5. Diff-aware (focus on new risk)
6. Actionable outputs (not just detection)

### PRD High-Level Layers

```
Language Frontends -> Unified Semantic Graph -> Analysis Engine Layer
-> Findings + Evidence Engine -> Ranking / Prioritization -> UI / API / CLI
```

---

## Current Implementation

Spectron is a Rust workspace with 9 crates, organized as a linear pipeline:

```
spectron-loader -> spectron-parser -> spectron-graph -> spectron-analysis -> spectron-ui
                                                                              ^
                                                                         spectron-app (CLI entry)
```

### Crate Map

| Crate | Role | Status |
|---|---|---|
| `spectron-core` | Shared domain types: IDs, Symbol, graph types, metrics, security indicators | Complete |
| `spectron-loader` | Project/crate/module/file discovery from Cargo manifests | Complete, tested |
| `spectron-parser` | AST parsing via `syn`, symbol + relationship extraction, name resolution | Complete, tested |
| `spectron-graph` | Structure graph, call graph, CFG construction, graph algorithms | Complete |
| `spectron-analysis` | Metrics, entrypoint detection, security indicators, complexity flags | Complete |
| `spectron-storage` | Persistence layer for analysis artifacts | Stub only |
| `spectron-render` | GPU rendering engine (wgpu) | Stub only |
| `spectron-ui` | Egui-based GUI: graph visualization, filter panel, inspector | Complete |
| `spectron-app` | CLI entry point with `--json` and `--cli` modes | Complete |

### Data Flow (Current)

```
CLI arg (path)
  |
  v
spectron_loader::load_project()     -> LoadResult  { project, crates, modules, files }
  |
  v
spectron_parser::parse_project()    -> ParseResult { symbols, relationships, errors }
  |
  v
spectron_graph::build_graphs()      -> GraphSet    { structure_graph, call_graph, CFGs, index }
  |
  v
spectron_analysis::analyze()        -> AnalysisOutput { metrics, security, entrypoints, flags }
  |
  v
spectron_ui::run()                  -> GUI window
```

### Dependency Graph (Internal Crates)

```
spectron-app
  ├── spectron-core
  ├── spectron-loader     -> spectron-core
  ├── spectron-parser     -> spectron-core, spectron-loader
  ├── spectron-graph      -> spectron-core, spectron-parser, spectron-loader
  ├── spectron-analysis   -> spectron-core, spectron-graph
  ├── spectron-storage    -> spectron-core   (stub)
  ├── spectron-render     -> spectron-core   (stub)
  └── spectron-ui         -> spectron-core, spectron-parser, spectron-graph, spectron-analysis
```

---

## Gap Analysis: PRD vs Implementation

| PRD Layer | Current Status | Gap |
|---|---|---|
| Language Frontends | Rust only via `syn` + `spectron-loader` | Only Rust supported (acceptable for v1) |
| Unified Semantic Graph | `ArchGraph` (petgraph DiGraph) with 6 edge types | Missing: DataFlow, ControlFlow, Ownership, TaintFlow, LockRelationship edges |
| Analysis Engine Layer | Metrics + security indicators + entrypoints | Missing: taint engine, auth analyzer, cost analyzer, structural rules, resource tracking |
| Findings + Evidence Engine | Ad-hoc types: `SecurityIndicator`, `ComplexityFlag`, `ParseError` | No unified `Finding` type with severity/confidence/explanation |
| Ranking / Prioritization | Not implemented | No ranking algorithm |
| UI / API / CLI | Egui GUI + `--json` + `--cli` tree output | Missing: security/perf/arch-specific views |

### What Exists and Works Well

- Full Rust project loading pipeline (workspace + single crate)
- Comprehensive symbol extraction (9 SymbolKind variants)
- Call graph + structure graph with petgraph
- CFG construction with branch/loop/await/return nodes
- Graph algorithms: path finding, descendants, reachable subgraph, components
- Entrypoint detection (6 heuristic rules)
- Security indicators (unsafe, FFI, filesystem, network, subprocess)
- Metrics (cyclomatic complexity, line count, parameter count, fan-in/out)
- Interactive GUI with force-directed layout, filtering, inspector

### What Is Missing Entirely

- Taint engine (Module 1)
- Auth/trust boundary analyzer (Module 2)
- Performance cost model + propagation (Module 3)
- Architecture rule enforcement (Module 4 -- graph exists but no rules)
- Resource/lifetime tracking (Module 5)
- Interprocedural function summaries
- Unified findings system
- Finding ranking/prioritization
- Rule engine (built-in + custom)
- Diff-aware analysis
- CI integration
- Storage layer (stub)

---

## Phased Roadmap (Mapped to Current State)

### Phase 1 -- Foundation (MOSTLY COMPLETE)

| Deliverable | Status | Notes |
|---|---|---|
| Rust parser integration | Done | `spectron-parser` via `syn` |
| Symbol index | Done | `SymbolId` -> `Symbol` maps |
| Module graph | Done | Structure graph with Contains/DependsOn |
| Call graph | Done | Separate call graph with callers/callees |
| CFG (basic) | Done | CfgNode/CfgEdge with branch/loop/await/return |
| Findings system | **Not done** | Ad-hoc types exist, no unified Finding |
| CLI interface | Done | `--json`, `--cli` modes |
| Structural Analyzer (basic) | Partial | Graph + algorithms exist, no cycle/rule analysis |
| Performance Analyzer (loop detect) | Partial | CFG has loop nodes, no explicit "DB in loop" |

**Remaining Phase 1 work:** Unified findings system, structural cycle detection, loop-based performance detections.

### Phase 2 -- Security Core (NOT STARTED)

Entrypoint detection exists. Everything else is new.

### Phase 3 -- Interprocedural Intelligence (NOT STARTED)

No function summaries exist.

### Phase 4 -- Performance + Resource Depth (NOT STARTED)

Metrics foundation exists (complexity, fan-in/out). Cost model is new.

### Phase 5 -- Architecture + Policy Engine (NOT STARTED)

Graph infrastructure exists. Rules/DSL are new.

### Phase 6 -- Diff + CI Integration (NOT STARTED)

`spectron-storage` is a stub. Everything is new.

### Phase 7 -- Advanced (NOT STARTED)

Future scope.

---

## Design Decisions

1. **Naming**: The codebase uses `spectron` (not `spectra`). All specs use the codebase name.
2. **Rust-only for v1**: The PRD mentions "Language Frontends" (plural). For v1, only Rust is in scope, which matches the current `syn`-based parser.
3. **New analysis modules live in `spectron-analysis`**: Rather than creating a new crate per PRD module, extend the existing `spectron-analysis` crate with submodules (e.g., `taint`, `auth`, `cost`, `structural`, `resource`).
4. **Findings live in `spectron-core`**: The unified `Finding` type is a domain type used by all crates, so it belongs in core alongside `SecurityIndicator` and `SymbolMetrics`.
5. **Rule engine as separate crate**: When the rule engine reaches sufficient complexity, extract it to `spectron-rules`. Start inline in `spectron-analysis`.
6. **Storage activates in Phase 6**: `spectron-storage` remains a stub until diff/baseline support is needed.

---

## Tasks

- [ ] **T-00.1** Define naming convention and update module-level doc comments to reference "Spectron" consistently
- [ ] **T-00.2** Add a top-level `README.md` describing the project, crate roles, and build instructions
- [ ] **T-00.3** Extend `spectron-app` CLI with subcommands for future analysis modes (e.g., `spectron analyze --security`, `spectron analyze --perf`)
- [ ] **T-00.4** Define shared error handling strategy: ensure all new analysis modules return `Result<T, SpectronError>` with appropriate new error variants
- [ ] **T-00.5** Add workspace-level integration test that runs the full pipeline on a fixture project and asserts non-empty output
