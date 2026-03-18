# 12 -- Diff-Aware Analysis & CI Integration

## PRD Reference

### Phase 6 Goals

Production adoption.

### Deliverables

- Diff-aware analysis
- Baseline system
- Regression detection
- CI integration

### Success Criteria (from PRD)

- Diff mode reduces noise by >80%
- <5 min analysis for medium repo

---

## Current Implementation

### What Exists

1. **File hashes** (`FileInfo.hash`): Each source file gets a SHA-256 hash
   during loading (`spectron-loader/src/files.rs`). This is the building block
   for change detection.

2. **Storage stub** (`spectron-storage/src/lib.rs`): The crate exists with
   `sled` as a dependency but contains no implementation. Its doc comment:
   `//! spectron-storage: persistence layer for analysis artifacts.`

3. **JSON output** (`spectron-app`): The `--json` flag serializes loader
   results. This could be extended to serialize full analysis output for
   baseline snapshots.

4. **Serde on core types**: Most core types (`FileInfo`, `Symbol`, `SourceSpan`,
   `Relationship`, `GraphNode`, `GraphEdge`, `SecurityIndicator`, etc.) derive
   `Serialize`/`Deserialize`.

### What Is Missing

- **Baseline storage**: No way to persist analysis results between runs.
- **Diff computation**: No way to determine which files/symbols changed.
- **Incremental analysis**: Re-analyzes everything on every run.
- **Regression detection**: No comparison between current and baseline findings.
- **CI output formats**: No SARIF, JUnit, or GitHub Annotations output.
- **Exit codes**: No non-zero exit for threshold violations.

---

## Gap Analysis

| PRD Requirement | Current Status | Action |
|---|---|---|
| Diff-aware analysis | File hashes exist; no diffing | Compare hashes between runs |
| Baseline system | Storage is a stub | Implement baseline save/load |
| Regression detection | Not implemented | Compare current findings against baseline |
| CI integration | Not implemented | Add output formats and exit codes |
| Noise reduction (>80%) | N/A | Only report findings on changed code paths |

---

## Design

### Architecture

```
spectron-storage/src/
  lib.rs          -- public API
  baseline.rs     -- baseline save/load
  diff.rs         -- change set computation

spectron-app/src/
  main.rs          -- extended with CI flags
  ci_output.rs     -- SARIF/JUnit formatters (new file)
```

### Baseline System

A **baseline** is a snapshot of analysis results from a known-good state
(e.g., the main branch).

```rust
pub struct Baseline {
    pub timestamp: u64,
    pub project_name: String,
    pub file_hashes: HashMap<PathBuf, String>,  // path -> SHA-256
    pub findings: Vec<Finding>,
    pub summary: BaselineSummary,
}

pub struct BaselineSummary {
    pub total_findings: usize,
    pub by_severity: HashMap<Severity, usize>,
    pub by_category: HashMap<FindingCategory, usize>,
}
```

**Storage format**: JSON file (`.spectron-baseline.json`) in the project root,
or sled database for large projects.

For MVP, use JSON file:
- Simple to inspect, version-control, and debug.
- Sled can be activated later for projects with thousands of findings.

### Diff Computation

**Change detection** using file hashes:

```rust
pub struct ChangeSet {
    pub added_files: Vec<PathBuf>,
    pub removed_files: Vec<PathBuf>,
    pub modified_files: Vec<PathBuf>,
    pub unchanged_files: Vec<PathBuf>,
}
```

**Algorithm:**

1. Load baseline file hashes.
2. Load current file hashes (from `LoadResult.files`).
3. Compare:
   - File in current but not baseline: `added`.
   - File in baseline but not current: `removed`.
   - File in both but hash differs: `modified`.
   - File in both with same hash: `unchanged`.

### Diff-Aware Filtering

Once the change set is known, filter findings to only those **affected by
changes**:

1. **Direct**: Finding is in a modified/added file.
2. **Transitive**: Finding is in a function called by a function in a
   modified file (use call graph descendants from changed symbols).

Configurable modes:
- `--diff-mode direct` -- only findings in changed files.
- `--diff-mode transitive` -- findings in changed files + their callers.
- `--diff-mode full` -- all findings (default, no filtering).

### Regression Detection

Compare current findings against baseline:

```rust
pub struct RegressionReport {
    pub new_findings: Vec<Finding>,       // in current, not in baseline
    pub fixed_findings: Vec<Finding>,     // in baseline, not in current
    pub unchanged_findings: Vec<Finding>, // in both
}
```

**Matching logic**: Two findings match if they have the same `rule_id` AND
the same `location` (file + span). If a finding moves due to code changes
(different line number), use fuzzy matching on rule_id + symbol name.

### CI Output Formats

**SARIF** (Static Analysis Results Interchange Format):
- Standard format consumed by GitHub Code Scanning, Azure DevOps, etc.
- Each finding maps to a SARIF `Result` with `ruleId`, `message`, `location`,
  `level` (error/warning/note).

**JUnit XML**:
- For CI systems that understand test results.
- Each finding category becomes a `testsuite`.
- Each finding becomes a `testcase` (pass if below threshold, fail otherwise).

**GitHub Annotations**:
- `::warning file={},line={}::{message}` format for inline PR comments.

**JSON**:
- Full analysis output as JSON (extend existing `--json` flag).

### CLI Extensions

```
spectron analyze <path> [flags]

Flags:
  --baseline <path>        Load baseline for comparison
  --save-baseline <path>   Save current results as baseline
  --diff-mode <mode>       direct | transitive | full (default: full)
  --format <format>        json | sarif | junit | github (default: json)
  --fail-on <severity>     Exit non-zero if findings at this severity or above
  --max-findings <n>       Exit non-zero if more than N findings
```

### Exit Codes

| Code | Meaning |
|---|---|
| 0 | Success, no threshold violations |
| 1 | Analysis error |
| 2 | Findings exceed threshold (--fail-on or --max-findings) |
| 3 | Regressions detected (new findings vs baseline) |

### Incremental Analysis (Future)

Full incremental analysis (re-parse only changed files, update only affected
graph nodes) is complex and deferred to a later phase. For now, diff-aware
means "re-analyze everything but only REPORT changed findings."

The pipeline still runs end-to-end. The diff filter is applied to the output,
not the computation. This is simpler and still meets the >80% noise reduction
target for CI.

---

## Design Decisions

1. **JSON baseline for MVP**: Simpler than sled, inspectable, version-controllable.
   Switch to sled when baseline size exceeds practical JSON limits (~10MB).

2. **Filter output, not computation**: Rather than building true incremental
   analysis (which requires dependency tracking at the symbol level), run the
   full pipeline and filter findings at the end. This gives correct results
   with minimal implementation effort.

3. **SARIF as primary CI format**: SARIF is the industry standard for static
   analysis results and is supported by GitHub, Azure DevOps, and many other
   CI platforms.

4. **Exit codes for CI gating**: CI pipelines need non-zero exit codes to
   fail builds. The `--fail-on` flag enables quality gates.

5. **Fuzzy matching for regressions**: Code changes often shift line numbers.
   Matching findings by rule_id + symbol name (not just line number) reduces
   false "new finding" reports from cosmetic code changes.

6. **spectron-storage remains simple**: Rather than building a complex database,
   storage handles baseline serialization. Sled is available when needed for
   caching parsed ASTs or graph snapshots for true incremental analysis.

---

## Tasks

- [ ] **T-12.1** Implement `Baseline` type with `save()` and `load()` methods (JSON format)
- [ ] **T-12.2** Implement `ChangeSet` computation from file hash comparison
- [ ] **T-12.3** Implement diff-aware finding filter: direct mode (findings in changed files)
- [ ] **T-12.4** Implement diff-aware finding filter: transitive mode (findings in changed files + callers)
- [ ] **T-12.5** Implement `RegressionReport`: new/fixed/unchanged finding comparison
- [ ] **T-12.6** Implement fuzzy finding matching (rule_id + symbol name)
- [ ] **T-12.7** Add `--baseline` and `--save-baseline` CLI flags
- [ ] **T-12.8** Add `--diff-mode` CLI flag (direct, transitive, full)
- [ ] **T-12.9** Implement SARIF output format
- [ ] **T-12.10** Implement GitHub Annotations output format
- [ ] **T-12.11** Implement JUnit XML output format
- [ ] **T-12.12** Add `--format` CLI flag for output format selection
- [ ] **T-12.13** Add `--fail-on` and `--max-findings` CLI flags with appropriate exit codes
- [ ] **T-12.14** Add integration test: save baseline, modify fixture, re-analyze, verify regression report
- [ ] **T-12.15** (Future) Implement true incremental analysis: re-parse only changed files, update graph incrementally
