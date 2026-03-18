//! spectron-ui: interactive visual codebase explorer.

pub mod graph_view;
pub mod inspector;
pub mod layout;

use std::collections::{HashMap, HashSet};

use egui::{Color32, RichText, Ui};
use spectron_core::{
    CrateInfo, CrateType, FileInfo, ModuleId, ModuleInfo, ProjectInfo,
    Symbol, SymbolId, SymbolKind,
};
use spectron_graph::GraphSet;

use crate::graph_view::ClickResult;
use crate::inspector::InspectorTarget;

// ---------------------------------------------------------------------------
// Colors
// ---------------------------------------------------------------------------

const BLUE: Color32 = Color32::from_rgb(110, 180, 255);
const ORANGE: Color32 = Color32::from_rgb(255, 170, 100);
const GREEN: Color32 = Color32::from_rgb(160, 215, 140);
const PURPLE: Color32 = Color32::from_rgb(200, 165, 255);
const DIM: Color32 = Color32::from_rgb(150, 150, 150);

// ---------------------------------------------------------------------------
// ProjectData
// ---------------------------------------------------------------------------

/// All data produced by the analysis pipeline, ready for the UI.
pub struct ProjectData {
    pub project: ProjectInfo,
    pub crates: Vec<CrateInfo>,
    pub modules: HashMap<ModuleId, ModuleInfo>,
    pub files: Vec<FileInfo>,
    pub total_lines: u32,
    pub symbols: HashMap<SymbolId, Symbol>,
    pub graph_set: GraphSet,
    pub analysis: spectron_analysis::AnalysisOutput,
}

impl ProjectData {
    pub fn new(
        project: ProjectInfo,
        crates: Vec<CrateInfo>,
        modules: HashMap<ModuleId, ModuleInfo>,
        files: Vec<FileInfo>,
        symbols: HashMap<SymbolId, Symbol>,
        graph_set: GraphSet,
        analysis: spectron_analysis::AnalysisOutput,
    ) -> Self {
        let total_lines = files.iter().map(|f| f.line_count).sum();
        Self {
            project,
            crates,
            modules,
            files,
            total_lines,
            symbols,
            graph_set,
            analysis,
        }
    }
}

// ---------------------------------------------------------------------------
// View mode
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq)]
enum ViewMode {
    Overview,
    StructureGraph,
    CallGraph,
    ModuleDetail(ModuleId),
}

// ---------------------------------------------------------------------------
// SpectronApp
// ---------------------------------------------------------------------------

struct SpectronApp {
    data: ProjectData,
    view_mode: ViewMode,
    structure_state: graph_view::GraphViewState,
    call_state: graph_view::GraphViewState,
    search: String,
    inspector_target: Option<InspectorTarget>,
    entrypoints: HashSet<SymbolId>,
}

impl eframe::App for SpectronApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ---- Top header ----
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let kind = if self.data.project.is_workspace {
                    "workspace"
                } else {
                    "crate"
                };
                if ui
                    .link(RichText::new(&self.data.project.name).heading().strong())
                    .clicked()
                {
                    self.view_mode = ViewMode::Overview;
                    self.inspector_target = None;
                }
                ui.label(RichText::new(format!("({})", kind)).color(DIM));
                ui.add_space(12.0);
                ui.separator();
                ui.label(format!("{} crates", self.data.crates.len()));
                ui.separator();
                ui.label(format!("{} modules", self.data.modules.len()));
                ui.separator();
                ui.label(format!("{} files", self.data.files.len()));
                ui.separator();
                ui.label(format!(
                    "{} symbols",
                    self.data.symbols.len()
                ));
                ui.separator();
                ui.label(format!(
                    "{} lines",
                    format_number(self.data.total_lines)
                ));
            });
            ui.add_space(6.0);
        });

        // ---- Right inspector panel ----
        let mut nav_to_symbol: Option<SymbolId> = None;

        if self.inspector_target.is_some() {
            egui::SidePanel::right("inspector")
                .default_width(280.0)
                .min_width(200.0)
                .max_width(400.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Inspector").strong());
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if ui.small_button("\u{2715}").clicked() {
                                    self.inspector_target = None;
                                }
                            },
                        );
                    });
                    ui.separator();
                    if let Some(ref target) = self.inspector_target {
                        inspector::show_inspector(
                            ui,
                            target,
                            &self.data,
                            &mut nav_to_symbol,
                        );
                    }
                });
        }

        // Handle inspector navigation (callers/callees clicked).
        if let Some(sid) = nav_to_symbol {
            self.inspector_target = Some(InspectorTarget::Symbol(sid));
            if let Some(&ni) = self.data.graph_set.index.symbol_nodes.get(&sid) {
                self.structure_state.selected = Some(ni);
            }
            if let Some(&ni) = self.data.graph_set.index.call_nodes.get(&sid) {
                self.call_state.selected = Some(ni);
            }
        }

        // ---- Left sidebar ----
        let mut sidebar_select_module: Option<ModuleId> = None;
        let mut sidebar_select_symbol: Option<SymbolId> = None;

        egui::SidePanel::left("sidebar")
            .default_width(300.0)
            .min_width(200.0)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Filter:");
                    ui.text_edit_singleline(&mut self.search);
                });
                ui.separator();

                let search = self.search.to_lowercase();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (i, krate) in self.data.crates.iter().enumerate() {
                            if !search.is_empty()
                                && !krate.name.to_lowercase().contains(&search)
                                && !any_module_matches(
                                    &krate.module_ids,
                                    &self.data.modules,
                                    &self.data.symbols,
                                    &search,
                                )
                            {
                                continue;
                            }

                            let (color, label) = crate_style(&krate.crate_type);
                            let title = RichText::new(format!(
                                "{} [{}]",
                                krate.name, label
                            ))
                            .color(color)
                            .strong();

                            ui.push_id(i, |ui| {
                                egui::CollapsingHeader::new(title)
                                    .default_open(true)
                                    .show(ui, |ui| {
                                        let roots: Vec<ModuleId> = krate
                                            .module_ids
                                            .iter()
                                            .filter(|id| {
                                                self.data
                                                    .modules
                                                    .get(id)
                                                    .map_or(false, |m| m.parent.is_none())
                                            })
                                            .copied()
                                            .collect();

                                        for mid in roots {
                                            show_module_node(
                                                ui,
                                                mid,
                                                &self.data.modules,
                                                &self.data.symbols,
                                                &mut sidebar_select_module,
                                                &mut sidebar_select_symbol,
                                                &search,
                                            );
                                        }
                                    });
                            });
                        }
                    });
            });

        // Apply sidebar selections.
        if let Some(mid) = sidebar_select_module {
            self.view_mode = ViewMode::ModuleDetail(mid);
        }
        if let Some(sid) = sidebar_select_symbol {
            self.inspector_target = Some(InspectorTarget::Symbol(sid));
            if let Some(&ni) = self.data.graph_set.index.symbol_nodes.get(&sid) {
                self.structure_state.selected = Some(ni);
            }
            if let Some(&ni) = self.data.graph_set.index.call_nodes.get(&sid) {
                self.call_state.selected = Some(ni);
            }
        }

        // ---- Central panel ----
        egui::CentralPanel::default().show(ctx, |ui| {
            // Tab bar
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(
                        self.view_mode == ViewMode::Overview,
                        RichText::new("Overview"),
                    )
                    .clicked()
                {
                    self.view_mode = ViewMode::Overview;
                }
                if ui
                    .selectable_label(
                        self.view_mode == ViewMode::StructureGraph,
                        RichText::new("Structure"),
                    )
                    .clicked()
                {
                    self.view_mode = ViewMode::StructureGraph;
                }
                if ui
                    .selectable_label(
                        self.view_mode == ViewMode::CallGraph,
                        RichText::new("Call Graph"),
                    )
                    .clicked()
                {
                    self.view_mode = ViewMode::CallGraph;
                }
                if matches!(self.view_mode, ViewMode::ModuleDetail(_)) {
                    let _ = ui.selectable_label(true, RichText::new("Module"));
                }
            });
            ui.separator();

            match self.view_mode.clone() {
                ViewMode::Overview => {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            show_overview(ui, &self.data);
                        });
                }
                ViewMode::StructureGraph => {
                    graph_view::show_toolbar(ui, &mut self.structure_state);
                    let result = graph_view::show_canvas(
                        ui,
                        &self.data.graph_set.structure_graph,
                        &mut self.structure_state,
                        &self.data,
                        &self.entrypoints,
                    );
                    match result {
                        ClickResult::NodeClicked(node) => {
                            self.inspector_target =
                                InspectorTarget::from_graph_node(&node);
                        }
                        ClickResult::BackgroundClicked => {
                            self.inspector_target = None;
                        }
                        ClickResult::Nothing => {}
                    }
                }
                ViewMode::CallGraph => {
                    graph_view::show_toolbar(ui, &mut self.call_state);
                    let result = graph_view::show_canvas(
                        ui,
                        &self.data.graph_set.call_graph,
                        &mut self.call_state,
                        &self.data,
                        &self.entrypoints,
                    );
                    match result {
                        ClickResult::NodeClicked(node) => {
                            self.inspector_target =
                                InspectorTarget::from_graph_node(&node);
                        }
                        ClickResult::BackgroundClicked => {
                            self.inspector_target = None;
                        }
                        ClickResult::Nothing => {}
                    }
                }
                ViewMode::ModuleDetail(mid) => {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if let Some(module) = self.data.modules.get(&mid) {
                                show_module_detail(
                                    ui,
                                    module,
                                    &self.data.modules,
                                    &self.data.symbols,
                                );
                            }
                        });
                }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Sidebar helpers
// ---------------------------------------------------------------------------

fn crate_style(ct: &CrateType) -> (Color32, &'static str) {
    match ct {
        CrateType::Library => (BLUE, "lib"),
        CrateType::Binary => (ORANGE, "bin"),
    }
}

fn any_module_matches(
    ids: &[ModuleId],
    modules: &HashMap<ModuleId, ModuleInfo>,
    symbols: &HashMap<SymbolId, Symbol>,
    search: &str,
) -> bool {
    ids.iter().any(|id| {
        modules.get(id).map_or(false, |m| {
            m.name.to_lowercase().contains(search)
                || m.symbol_ids
                    .iter()
                    .any(|sid| {
                        symbols
                            .get(sid)
                            .map_or(false, |s| s.name.to_lowercase().contains(search))
                    })
                || any_module_matches(&m.children, modules, symbols, search)
        })
    })
}

fn show_module_node(
    ui: &mut Ui,
    mid: ModuleId,
    modules: &HashMap<ModuleId, ModuleInfo>,
    symbols: &HashMap<SymbolId, Symbol>,
    select_module: &mut Option<ModuleId>,
    select_symbol: &mut Option<SymbolId>,
    search: &str,
) {
    let Some(module) = modules.get(&mid) else {
        return;
    };

    let name = &module.name;

    if !search.is_empty()
        && !name.to_lowercase().contains(search)
        && !module.symbol_ids.iter().any(|sid| {
            symbols
                .get(sid)
                .map_or(false, |s| s.name.to_lowercase().contains(search))
        })
        && !any_module_matches(&module.children, modules, symbols, search)
    {
        return;
    }

    let has_children = !module.children.is_empty() || !module.symbol_ids.is_empty();

    if !has_children {
        // Leaf module: clickable label.
        ui.horizontal(|ui| {
            ui.add_space(18.0);
            if ui
                .selectable_label(false, RichText::new(name).color(GREEN))
                .clicked()
            {
                *select_module = Some(mid);
            }
        });
    } else {
        let children_ids: Vec<ModuleId> = module.children.clone();
        let sym_ids: Vec<SymbolId> = module.symbol_ids.clone();

        let header = egui::CollapsingHeader::new(RichText::new(name).color(GREEN))
            .id_source(format!("m{}", mid.0))
            .default_open(search.is_empty() && module.children.len() < 10);

        let resp = header.show(ui, |ui| {
            // Child modules.
            for cid in &children_ids {
                show_module_node(
                    ui,
                    *cid,
                    modules,
                    symbols,
                    select_module,
                    select_symbol,
                    search,
                );
            }
            // Symbols in this module.
            for sid in &sym_ids {
                if let Some(sym) = symbols.get(sid) {
                    if !search.is_empty()
                        && !sym.name.to_lowercase().contains(search)
                    {
                        continue;
                    }
                    let color = inspector::symbol_kind_color(&sym.kind);
                    let prefix = inspector::symbol_kind_prefix(&sym.kind);
                    let label = format!("{} {}", prefix, sym.name);
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        if ui
                            .selectable_label(
                                false,
                                RichText::new(label).color(color).small(),
                            )
                            .clicked()
                        {
                            *select_symbol = Some(*sid);
                        }
                    });
                }
            }
        });

        // Clicking the header text also selects the module.
        if resp.header_response.clicked() {
            *select_module = Some(mid);
        }
    }
}

// ---------------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------------

fn format_number(n: u32) -> String {
    if n >= 1_000_000 {
        format!(
            "{},{:03},{:03}",
            n / 1_000_000,
            (n / 1000) % 1000,
            n % 1000
        )
    } else if n >= 1000 {
        format!("{},{:03}", n / 1000, n % 1000)
    } else {
        n.to_string()
    }
}

fn show_overview(ui: &mut Ui, data: &ProjectData) {
    ui.heading(
        RichText::new(format!("{} \u{2014} Overview", data.project.name)).strong(),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new(data.project.root_path.display().to_string())
            .monospace()
            .color(DIM),
    );
    ui.add_space(16.0);
    ui.separator();
    ui.add_space(12.0);

    let libs = data
        .crates
        .iter()
        .filter(|c| c.crate_type == CrateType::Library)
        .count();
    let bins = data
        .crates
        .iter()
        .filter(|c| c.crate_type == CrateType::Binary)
        .count();
    let fns = data
        .symbols
        .values()
        .filter(|s| s.kind == SymbolKind::Function || s.kind == SymbolKind::Method)
        .count();
    let structs = data
        .symbols
        .values()
        .filter(|s| s.kind == SymbolKind::Struct)
        .count();
    let traits = data
        .symbols
        .values()
        .filter(|s| s.kind == SymbolKind::Trait)
        .count();

    egui::Grid::new("stats")
        .num_columns(2)
        .spacing([40.0, 10.0])
        .show(ui, |ui| {
            stat_row(ui, "Crates", &data.crates.len().to_string(), Color32::WHITE);
            stat_row(ui, "  Libraries", &libs.to_string(), BLUE);
            stat_row(ui, "  Binaries", &bins.to_string(), ORANGE);
            stat_row(
                ui,
                "Modules",
                &data.modules.len().to_string(),
                GREEN,
            );
            stat_row(
                ui,
                "Files",
                &data.files.len().to_string(),
                Color32::WHITE,
            );
            stat_row(
                ui,
                "Symbols",
                &data.symbols.len().to_string(),
                Color32::WHITE,
            );
            stat_row(ui, "  Functions", &fns.to_string(), ORANGE);
            stat_row(ui, "  Structs", &structs.to_string(), PURPLE);
            stat_row(ui, "  Traits", &traits.to_string(), Color32::from_rgb(100, 210, 210));
            stat_row(
                ui,
                "Total Lines",
                &format_number(data.total_lines),
                Color32::WHITE,
            );
            stat_row(
                ui,
                "Entrypoints",
                &data.analysis.entrypoints.len().to_string(),
                Color32::from_rgb(255, 200, 50),
            );
            stat_row(
                ui,
                "Complexity Flags",
                &data.analysis.complexity_flags.len().to_string(),
                Color32::from_rgb(255, 110, 110),
            );
        });

    ui.add_space(20.0);
    ui.separator();
    ui.add_space(12.0);
    ui.label(RichText::new("Crates").size(16.0).strong());
    ui.add_space(6.0);

    for krate in &data.crates {
        let (color, label) = crate_style(&krate.crate_type);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("[{label}]"))
                    .monospace()
                    .color(color),
            );
            ui.label(RichText::new(&krate.name).strong());
            ui.label(
                RichText::new(format!("\u{2014} {} modules", krate.module_ids.len()))
                    .color(DIM),
            );
        });
    }
}

fn stat_row(ui: &mut Ui, label: &str, value: &str, color: Color32) {
    ui.label(RichText::new(label).color(DIM));
    ui.label(RichText::new(value).size(18.0).strong().color(color));
    ui.end_row();
}

// ---------------------------------------------------------------------------
// Module detail
// ---------------------------------------------------------------------------

fn show_module_detail(
    ui: &mut Ui,
    module: &ModuleInfo,
    modules: &HashMap<ModuleId, ModuleInfo>,
    symbols: &HashMap<SymbolId, Symbol>,
) {
    ui.heading(RichText::new(&module.name).strong());
    ui.add_space(4.0);
    ui.label(
        RichText::new(module.path.as_str())
            .monospace()
            .color(PURPLE),
    );
    ui.add_space(16.0);
    ui.separator();
    ui.add_space(12.0);

    egui::Grid::new("mod_detail")
        .num_columns(2)
        .spacing([40.0, 8.0])
        .show(ui, |ui| {
            ui.label(RichText::new("Module Path").color(DIM));
            ui.label(RichText::new(module.path.as_str()).monospace());
            ui.end_row();

            if let Some(ref fp) = module.file_path {
                ui.label(RichText::new("Source File").color(DIM));
                ui.label(
                    RichText::new(fp.display().to_string()).monospace(),
                );
                ui.end_row();
            }

            if let Some(pid) = module.parent {
                ui.label(RichText::new("Parent").color(DIM));
                let pname = modules.get(&pid).map_or("\u{2014}", |m| &m.name);
                ui.label(RichText::new(pname).color(GREEN));
                ui.end_row();
            }

            ui.label(RichText::new("Children").color(DIM));
            ui.label(
                RichText::new(module.children.len().to_string()).strong(),
            );
            ui.end_row();

            ui.label(RichText::new("Symbols").color(DIM));
            ui.label(
                RichText::new(module.symbol_ids.len().to_string()).strong(),
            );
            ui.end_row();
        });

    if !module.children.is_empty() {
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);
        ui.label(RichText::new("Child Modules").size(15.0).strong());
        ui.add_space(4.0);

        for cid in &module.children {
            if let Some(child) = modules.get(cid) {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(RichText::new(&child.name).color(GREEN));
                    if let Some(ref fp) = child.file_path {
                        ui.label(
                            RichText::new(fp.display().to_string())
                                .monospace()
                                .small()
                                .color(DIM),
                        );
                    }
                });
            }
        }
    }

    if !module.symbol_ids.is_empty() {
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);
        ui.label(RichText::new("Symbols").size(15.0).strong());
        ui.add_space(4.0);

        for sid in &module.symbol_ids {
            if let Some(sym) = symbols.get(sid) {
                let color = inspector::symbol_kind_color(&sym.kind);
                let prefix = inspector::symbol_kind_prefix(&sym.kind);
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(prefix)
                            .monospace()
                            .small()
                            .color(DIM),
                    );
                    ui.label(RichText::new(&sym.name).color(color));
                    if let Some(ref sig) = sym.signature {
                        ui.label(
                            RichText::new(sig)
                                .monospace()
                                .small()
                                .color(DIM),
                        );
                    }
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Launch the Spectron visual explorer window.
pub fn run(data: ProjectData) -> anyhow::Result<()> {
    let entrypoints: HashSet<SymbolId> =
        data.analysis.entrypoints.iter().copied().collect();
    let title = format!("Spectron \u{2014} {}", data.project.name);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(&title)
            .with_inner_size([1400.0, 900.0]),
        ..Default::default()
    };

    eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(SpectronApp {
                data,
                view_mode: ViewMode::Overview,
                structure_state: graph_view::GraphViewState::new_structure(),
                call_state: graph_view::GraphViewState::new_call(),
                search: String::new(),
                inspector_target: None,
                entrypoints,
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
