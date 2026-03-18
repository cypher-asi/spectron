//! spectron-app: application entry point and orchestration.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use spectron_core::{CrateInfo, CrateType, ModuleId, ModuleInfo, SymbolId};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "spectron", about = "Rust codebase visualization")]
struct Cli {
    /// Path to the Rust project to analyze (defaults to current directory)
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Output results as JSON
    #[arg(long)]
    json: bool,

    /// Print text tree to terminal instead of opening the GUI
    #[arg(long)]
    cli: bool,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("spectron=info,warn")),
        )
        .init();

    let cli = Cli::parse();

    let path = cli
        .path
        .canonicalize()
        .with_context(|| format!("path not found: {}", cli.path.display()))?;

    let load_result = spectron_loader::load_project(&path)
        .with_context(|| format!("failed to load project at {}", path.display()))?;

    if cli.json {
        let json = serde_json::json!({
            "project": load_result.project,
            "crates": load_result.crates,
            "modules": load_result.modules,
            "files": load_result.files,
        });
        println!("{}", serde_json::to_string_pretty(&json)?);
        return Ok(());
    }

    if cli.cli {
        print_cli(&load_result);
        return Ok(());
    }

    // Run the full analysis pipeline.
    let parse_result = spectron_parser::parse_project(&load_result);
    let graph_set = spectron_graph::build_graphs(&load_result, &parse_result);

    let symbols: HashMap<SymbolId, spectron_core::Symbol> = parse_result
        .symbols
        .into_iter()
        .map(|s| (s.id, s))
        .collect();

    let modules: HashMap<ModuleId, ModuleInfo> = load_result
        .modules
        .into_iter()
        .map(|m| (m.id, m))
        .collect();

    let analysis = spectron_analysis::analyze(&graph_set, &symbols, &modules);

    let data = spectron_ui::ProjectData::new(
        load_result.project,
        load_result.crates,
        modules,
        load_result.files,
        symbols,
        graph_set,
        analysis,
    );
    spectron_ui::run(data)
}

fn print_cli(result: &spectron_loader::LoadResult) {
    let project = &result.project;
    let kind = if project.is_workspace {
        "workspace"
    } else {
        "crate"
    };

    println!();
    println!("  {} ({})", project.name, kind);
    println!("  {}", project.root_path.display());
    println!(
        "  {} crate(s) · {} module(s) · {} file(s)",
        result.crates.len(),
        result.modules.len(),
        result.files.len(),
    );

    let total_lines: u32 = result.files.iter().map(|f| f.line_count).sum();
    println!("  {} total lines of Rust", total_lines);
    println!();

    let module_map: HashMap<ModuleId, &ModuleInfo> =
        result.modules.iter().map(|m| (m.id, m)).collect();

    for krate in &result.crates {
        print_crate(krate, &module_map);
    }
}

fn print_crate(krate: &CrateInfo, modules: &HashMap<ModuleId, &ModuleInfo>) {
    let type_label = match krate.crate_type {
        CrateType::Library => "lib",
        CrateType::Binary => "bin",
    };
    println!("  {} [{}]", krate.name, type_label);

    let roots: Vec<_> = krate
        .module_ids
        .iter()
        .filter_map(|id| modules.get(id))
        .filter(|m| m.parent.is_none())
        .collect();

    for root in roots {
        print_module(root, modules, "    ", true);
    }
    println!();
}

fn print_module(
    module: &ModuleInfo,
    modules: &HashMap<ModuleId, &ModuleInfo>,
    prefix: &str,
    is_last: bool,
) {
    let connector = if is_last { "\u{2514}\u{2500}\u{2500} " } else { "\u{251C}\u{2500}\u{2500} " };
    println!("{}{}{}", prefix, connector, module.name);

    let child_prefix = if is_last {
        format!("{}    ", prefix)
    } else {
        format!("{}\u{2502}   ", prefix)
    };

    let children: Vec<_> = module
        .children
        .iter()
        .filter_map(|id| modules.get(id))
        .collect();

    for (i, child) in children.iter().enumerate() {
        let last = i == children.len() - 1;
        print_module(child, modules, &child_prefix, last);
    }
}
