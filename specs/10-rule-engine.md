# 10 -- Rule Engine

## PRD Reference

### Rule Engine Supports

- Built-in rules
- Custom org rules
- DSL (Phase 3+)

The PRD expects a shared rule engine that all analysis modules can leverage
for configurable, extensible analysis.

---

## Current Implementation

There is **no rule engine**. All analysis logic is hardcoded:

### Hardcoded Thresholds (`spectron-analysis/src/metrics.rs`)

```rust
const CYCLOMATIC_COMPLEXITY_THRESHOLD: u32 = 15;
const LARGE_FUNCTION_THRESHOLD: u32 = 100;
const LARGE_MODULE_THRESHOLD: u32 = 50;
const MODULE_FAN_IN_THRESHOLD: u32 = 20;
const MODULE_FAN_OUT_THRESHOLD: u32 = 15;
```

### Hardcoded Patterns (`spectron-analysis/src/security.rs`)

```rust
const FILESYSTEM_PREFIXES: &[&str] = &["std::fs::", "tokio::fs::"];
const NETWORK_PREFIXES: &[&str] = &["std::net::", "tokio::net::", "reqwest::", "hyper::"];
const SUBPROCESS_PREFIXES: &[&str] = &["std::process::Command", "tokio::process::Command"];
```

### Hardcoded Entrypoint Rules (`spectron-analysis/src/entrypoints.rs`)

```rust
const ASYNC_MAIN_ATTRS: &[&str] = &["tokio::main", "async_std::main", "actix_web::main"];
const HANDLER_ATTR_PREFIXES: &[&str] = &["get", "post", "put", "delete", "handler"];
const CLI_COMMAND_ATTRS: &[&str] = &["command"];
```

### What Is Missing

- **Rule abstraction**: No shared Rule trait or type that all checkers conform to.
- **Configuration**: No way to override thresholds, add patterns, or disable rules.
- **Custom rules**: No user-defined rules.
- **Rule metadata**: No way to attach descriptions, references, or documentation to rules.

---

## Gap Analysis

| PRD Requirement | Current Status | Action |
|---|---|---|
| Built-in rules | Hardcoded logic in analysis modules | Extract into Rule instances |
| Custom org rules | Not supported | Load from config file |
| Threshold configuration | Hardcoded constants | Make configurable per-rule |
| Rule DSL | Not implemented | Phase 5 (future) |
| Rule enable/disable | Not supported | Add rule ID toggling |
| Rule documentation | Not implemented | Add metadata per rule |

---

## Design

### Architecture

For MVP, the rule engine is a lightweight configuration + matching layer
within `spectron-analysis`. It does NOT need its own crate yet.

```
spectron-analysis/src/rules/
  mod.rs          -- public API: RuleRegistry, load_rules()
  types.rs        -- Rule, RuleConfig, RuleMatch
  builtin.rs      -- built-in rule definitions
  config.rs       -- configuration file parsing
```

### Rule Type

```rust
pub struct Rule {
    pub id: String,               // e.g., "security/taint/sql-injection"
    pub name: String,             // human-readable name
    pub description: String,      // detailed description
    pub category: FindingCategory,
    pub default_severity: Severity,
    pub enabled: bool,
    pub config: RuleConfig,
}

pub enum RuleConfig {
    /// Threshold-based rule (complexity, size, coupling)
    Threshold {
        metric: String,
        operator: ThresholdOp,
        value: f64,
    },
    /// Pattern-matching rule (taint sources, sinks, sensitive APIs)
    Pattern {
        patterns: Vec<String>,
        match_type: PatternMatchType,
    },
    /// Must-call rule (auth guards)
    MustCall {
        scope: EntrypointScope,
        required_functions: Vec<String>,
    },
    /// Layer rule (architecture constraints)
    Layer {
        layers: Vec<LayerDefinition>,
    },
    /// Custom (opaque config for future DSL)
    Custom(HashMap<String, String>),
}

pub enum ThresholdOp { Gt, Gte, Lt, Lte, Eq }
pub enum PatternMatchType { Prefix, Exact, Contains, Regex }
pub enum EntrypointScope { HttpHandlers, AllExternal, All }

pub struct LayerDefinition {
    pub name: String,
    pub module_patterns: Vec<String>,
}
```

### Rule Registry

```rust
pub struct RuleRegistry {
    rules: HashMap<String, Rule>,
}

impl RuleRegistry {
    pub fn new() -> Self;
    pub fn register(&mut self, rule: Rule);
    pub fn get(&self, id: &str) -> Option<&Rule>;
    pub fn enabled_rules(&self) -> Vec<&Rule>;
    pub fn rules_for_category(&self, cat: FindingCategory) -> Vec<&Rule>;
    pub fn is_enabled(&self, id: &str) -> bool;
    pub fn set_enabled(&mut self, id: &str, enabled: bool);
}
```

### Built-in Rules

All existing hardcoded checks become named rules:

| Rule ID | Category | Current Location | Type |
|---|---|---|---|
| `quality/complexity/high-cyclomatic` | Quality | metrics.rs | Threshold (>=15) |
| `quality/complexity/large-function` | Quality | metrics.rs | Threshold (>=100 lines) |
| `quality/complexity/large-module` | Quality | metrics.rs | Threshold (>=50 symbols) |
| `quality/coupling/high-fan-in` | Quality | metrics.rs | Threshold (>=20) |
| `quality/coupling/high-fan-out` | Quality | metrics.rs | Threshold (>=15) |
| `security/code/unsafe-function` | Security | security.rs | Pattern |
| `security/code/unsafe-block` | Security | security.rs | Pattern |
| `security/boundary/ffi-call` | Security | security.rs | Pattern |
| `security/api/filesystem-access` | Security | security.rs | Pattern |
| `security/api/network-access` | Security | security.rs | Pattern |
| `security/api/subprocess-exec` | Security | security.rs | Pattern |
| `security/taint/sql-injection` | Security | (new, spec 03) | Pattern |
| `security/taint/command-injection` | Security | (new, spec 03) | Pattern |
| `security/auth/missing-guard` | Security | (new, spec 04) | MustCall |
| `perf/cost/db-in-loop` | Performance | (new, spec 05) | Pattern |
| `perf/cost/network-in-loop` | Performance | (new, spec 05) | Pattern |
| `perf/cost/hotspot` | Performance | (new, spec 05) | Threshold |
| `arch/structure/cyclic-dependency` | Architecture | (new, spec 06) | Pattern |
| `arch/structure/layer-violation` | Architecture | (new, spec 06) | Layer |
| `resource/lock/held-across-await` | Resource | (new, spec 07) | Pattern |
| `resource/transaction/uncommitted` | Resource | (new, spec 07) | Pattern |

### Configuration File

Users can override rules via `.spectron.toml` in the project root:

```toml
[rules]
# Disable a rule
"quality/complexity/high-cyclomatic".enabled = false

# Change a threshold
"quality/complexity/large-function".threshold = 150

# Add custom patterns
[rules."security/api/custom-sensitive"]
category = "security"
severity = "high"
patterns = ["my_company::internal_api::"]
match_type = "prefix"
description = "Calls to internal sensitive API"
```

### Migration Strategy

1. **Phase 1**: Extract existing constants into Rule instances. The analysis
   logic stays the same but reads thresholds from the RuleRegistry instead
   of constants.

2. **Phase 2-4**: New analysis modules register their rules in the registry
   at startup.

3. **Phase 5**: Load user-defined rules from `.spectron.toml` and merge with
   built-in rules.

4. **Phase 5+**: Introduce DSL for complex rules (graph pattern matching,
   cross-module constraints).

---

## Design Decisions

1. **Rules are data, not code (for now)**: Rules are configuration structures
   loaded into a registry, not arbitrary code. This keeps them serializable,
   configurable, and documentable. Code-based rules (via a DSL or plugin
   system) are Phase 5+.

2. **Registry is populated at startup**: All built-in rules are registered
   before analysis begins. This allows the config file to override any
   built-in rule.

3. **Rule IDs follow a hierarchical convention**: `category/module/check`.
   This enables filtering by prefix (e.g., "show all security rules").

4. **No DSL in MVP**: The PRD mentions DSL for Phase 3+. For now, TOML
   configuration covers threshold overrides, pattern additions, and
   enable/disable. A DSL would be needed for complex cross-module rules.

5. **Rules are separate from findings**: A Rule defines WHAT to check.
   A Finding is the RESULT of checking. The rule_id in Finding links
   back to the rule that produced it.

---

## Tasks

- [ ] **T-10.1** Create `spectron-analysis/src/rules/` module directory
- [ ] **T-10.2** Define `Rule`, `RuleConfig`, `RuleRegistry` types
- [ ] **T-10.3** Implement `RuleRegistry` with register, get, enable/disable, filter by category
- [ ] **T-10.4** Register all existing hardcoded checks as built-in Rule instances
- [ ] **T-10.5** Refactor `metrics.rs` to read thresholds from RuleRegistry instead of constants
- [ ] **T-10.6** Refactor `security.rs` to read patterns from RuleRegistry instead of constants
- [ ] **T-10.7** Refactor `entrypoints.rs` to read attribute patterns from RuleRegistry
- [ ] **T-10.8** Implement `.spectron.toml` config file parser for rule overrides
- [ ] **T-10.9** Load config at startup and merge with built-in rules
- [ ] **T-10.10** Add `--list-rules` CLI flag that prints all registered rules with their status
- [ ] **T-10.11** Add `--disable-rule <id>` and `--enable-rule <id>` CLI flags
- [ ] **T-10.12** Add unit tests: rule registration, config override, enable/disable
- [ ] **T-10.13** (Phase 5) Design and implement rule DSL for complex constraints
