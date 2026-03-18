//! spectron-ui: interactive visual codebase explorer.

pub mod filter_panel;
pub mod graph_view;
pub mod inspector;
pub mod layout;

use std::collections::{HashMap, HashSet};

use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use spectron_core::{
    CrateId, CrateInfo, CrateType, FileId, FileInfo, ModuleId, ModuleInfo, ProjectInfo,
    Symbol, SymbolId, SymbolKind,
};
use spectron_graph::GraphSet;

use crate::graph_view::ClickResult;
use crate::inspector::InspectorTarget;

// ---------------------------------------------------------------------------
// Colors
// ---------------------------------------------------------------------------

const BLUE: Color32 = Color32::from_rgb(77, 84, 245); // #4D54F5
const RED: Color32 = Color32::from_rgb(254, 75, 66); // #FE4B42
const GREEN: Color32 = Color32::from_rgb(82, 242, 132); // #52F284
const PURPLE: Color32 = Color32::from_rgb(182, 83, 249); // #B653F9
const YELLOW_GREEN: Color32 = Color32::from_rgb(171, 240, 18); // #ABF012
const DIM: Color32 = Color32::from_rgb(150, 150, 150);
const BORDER: Color32 = Color32::from_rgb(40, 40, 40);

const TITLEBAR_HEIGHT: f32 = 36.0;
const WINDOW_BTN_SIZE: f32 = 28.0;
const WINDOW_ICON_SIZE: f32 = 10.0;
const WINDOW_ICON_STROKE: f32 = 1.5;

// ---------------------------------------------------------------------------
// Window control buttons
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum WindowControl {
    Minimize,
    Maximize,
    Close,
}

fn window_control_button(ui: &mut Ui, control: WindowControl) -> egui::Response {
    let size = Vec2::splat(WINDOW_BTN_SIZE);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());

    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let painter = ui.painter();

        if hovered {
            let bg = match control {
                WindowControl::Close => Color32::from_rgb(200, 50, 50),
                _ => Color32::from_rgb(45, 45, 45),
            };
            painter.rect_filled(rect, 4.0, bg);
        }

        let icon_color = if hovered && control == WindowControl::Close {
            Color32::WHITE
        } else if hovered {
            Color32::from_rgb(200, 200, 200)
        } else {
            Color32::from_rgb(120, 120, 120)
        };
        let stroke = Stroke::new(WINDOW_ICON_STROKE, icon_color);
        let center = rect.center();
        let half = WINDOW_ICON_SIZE / 2.0;

        match control {
            WindowControl::Minimize => {
                painter.line_segment(
                    [
                        Pos2::new(center.x - half, center.y),
                        Pos2::new(center.x + half, center.y),
                    ],
                    stroke,
                );
            }
            WindowControl::Maximize => {
                let sq = Rect::from_center_size(center, Vec2::splat(WINDOW_ICON_SIZE));
                painter.rect_stroke(sq, 0.0, stroke);
            }
            WindowControl::Close => {
                painter.line_segment(
                    [
                        Pos2::new(center.x - half, center.y - half),
                        Pos2::new(center.x + half, center.y + half),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        Pos2::new(center.x + half, center.y - half),
                        Pos2::new(center.x - half, center.y + half),
                    ],
                    stroke,
                );
            }
        }
    }

    response
}

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
    /// Reverse mapping: module -> owning crate.
    pub module_to_crate: HashMap<ModuleId, CrateId>,
    /// O(1) lookup from CrateId to index in `crates` vec.
    pub crate_index: HashMap<CrateId, usize>,
    /// O(1) lookup from FileId to index in `files` vec.
    pub file_index: HashMap<FileId, usize>,
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
        let mut module_to_crate = HashMap::new();
        for krate in &crates {
            for &mid in &krate.module_ids {
                module_to_crate.insert(mid, krate.id);
            }
        }
        let crate_index: HashMap<CrateId, usize> = crates
            .iter()
            .enumerate()
            .map(|(i, c)| (c.id, i))
            .collect();
        let file_index: HashMap<FileId, usize> = files
            .iter()
            .enumerate()
            .map(|(i, f)| (f.id, i))
            .collect();
        Self {
            project,
            crates,
            modules,
            files,
            total_lines,
            symbols,
            graph_set,
            analysis,
            module_to_crate,
            crate_index,
            file_index,
        }
    }
}

// ---------------------------------------------------------------------------
// View mode
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum ViewMode {
    Overview,
    Architecture,
    StructureGraph,
    CallGraph,
    CycleView,
    HotspotView,
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
        let is_maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));

        // ---- Custom titlebar ----
        let titlebar_response = egui::TopBottomPanel::top("titlebar")
            .exact_height(TITLEBAR_HEIGHT)
            .frame(egui::Frame::none().fill(Color32::BLACK).inner_margin(egui::Margin {
                left: 14.0,
                right: 6.0,
                top: 0.0,
                bottom: 0.0,
            }))
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(RichText::new("SPECTRON").strong().size(13.0).color(Color32::WHITE));
                    ui.add_space(8.0);

                    let kind = if self.data.project.is_workspace { "workspace" } else { "crate" };
                    if ui
                        .link(RichText::new(&self.data.project.name).size(12.0).color(DIM))
                        .clicked()
                    {
                        self.view_mode = ViewMode::Overview;
                        self.inspector_target = None;
                    }
                    ui.label(RichText::new(format!("({})", kind)).size(11.0).color(Color32::from_rgb(80, 80, 80)));
                    ui.add_space(8.0);
                    ui.label(RichText::new("\u{2022}").size(8.0).color(Color32::from_rgb(50, 50, 50)));
                    ui.add_space(4.0);

                    let stats = format!(
                        "{} crates  \u{00B7}  {} modules  \u{00B7}  {} files  \u{00B7}  {} symbols  \u{00B7}  {} lines",
                        self.data.crates.len(),
                        self.data.modules.len(),
                        self.data.files.len(),
                        self.data.symbols.len(),
                        format_number(self.data.total_lines),
                    );
                    ui.label(RichText::new(stats).size(11.0).color(Color32::from_rgb(70, 70, 70)));

                    // Window control buttons on the right
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(2.0);
                        if window_control_button(ui, WindowControl::Close).clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if window_control_button(ui, WindowControl::Maximize).clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_maximized));
                        }
                        if window_control_button(ui, WindowControl::Minimize).clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                    });
                });
            });

        // Paint bottom border on the titlebar
        let titlebar_rect = titlebar_response.response.rect;
        let painter = ctx.layer_painter(egui::LayerId::background());
        painter.line_segment(
            [
                Pos2::new(titlebar_rect.left(), titlebar_rect.bottom()),
                Pos2::new(titlebar_rect.right(), titlebar_rect.bottom()),
            ],
            Stroke::new(1.0, BORDER),
        );

        // Drag & double-click on titlebar
        let titlebar_resp = titlebar_response.response;
        if titlebar_resp.is_pointer_button_down_on() {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if titlebar_resp.double_clicked() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_maximized));
        }

        // ---- Right panel: filter panel + inspector ----
        let mut nav_to_symbol: Option<SymbolId> = None;
        let is_graph_view = matches!(
            self.view_mode,
            ViewMode::Architecture
                | ViewMode::StructureGraph
                | ViewMode::CallGraph
                | ViewMode::CycleView
                | ViewMode::HotspotView
        );

        if is_graph_view || self.inspector_target.is_some() {
            egui::SidePanel::right("right_panel")
                .default_width(260.0)
                .min_width(200.0)
                .max_width(400.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if is_graph_view {
                                let state = match self.view_mode {
                                    ViewMode::CallGraph => &mut self.call_state,
                                    _ => &mut self.structure_state,
                                };
                                let filters_changed = filter_panel::show_filter_panel(
                                    ui,
                                    state,
                                    &self.data,
                                    &self.entrypoints,
                                );
                                if filters_changed {
                                    state.initialized = false;
                                }
                            }

                            if self.inspector_target.is_some() {
                                ui.add_space(8.0);
                                ui.separator();
                                ui.add_space(4.0);
                                let mut close = false;
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("Inspector").strong());
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui.small_button("\u{2715}").clicked() {
                                                close = true;
                                            }
                                        },
                                    );
                                });
                                if close {
                                    self.inspector_target = None;
                                } else {
                                    ui.separator();
                                    let target = self.inspector_target.as_ref().unwrap();
                                    if let Some(focus_ni) =
                                        inspector::show_inspector_with_actions(
                                            ui,
                                            target,
                                            &self.data,
                                            &mut nav_to_symbol,
                                        )
                                    {
                                        self.structure_state.focus_node = Some(focus_ni);
                                    }
                                }
                            }
                        });
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
                    ui.label("Search:");
                    let resp = ui.text_edit_singleline(&mut self.search);
                    if resp.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                        && !self.search.is_empty()
                    {
                        let query = self.search.to_lowercase();
                        for (sid, sym) in &self.data.symbols {
                            if sym.name.to_lowercase().contains(&query) {
                                if let Some(&ni) =
                                    self.data.graph_set.index.symbol_nodes.get(sid)
                                {
                                    self.structure_state.focus_node = Some(ni);
                                    self.structure_state.selected = Some(ni);
                                    self.inspector_target =
                                        Some(InspectorTarget::Symbol(*sid));
                                    if !matches!(
                                        self.view_mode,
                                        ViewMode::StructureGraph
                                            | ViewMode::Architecture
                                    ) {
                                        self.view_mode = ViewMode::StructureGraph;
                                    }
                                    break;
                                }
                                break;
                            }
                        }
                    }
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
                        self.view_mode == ViewMode::Architecture,
                        RichText::new("Architecture"),
                    )
                    .clicked()
                {
                    self.view_mode = ViewMode::Architecture;
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
                if ui
                    .selectable_label(
                        self.view_mode == ViewMode::CycleView,
                        RichText::new("Cycles"),
                    )
                    .clicked()
                {
                    self.view_mode = ViewMode::CycleView;
                }
                if ui
                    .selectable_label(
                        self.view_mode == ViewMode::HotspotView,
                        RichText::new("Hotspots"),
                    )
                    .clicked()
                {
                    self.view_mode = ViewMode::HotspotView;
                }
                if matches!(self.view_mode, ViewMode::ModuleDetail(_)) {
                    let _ = ui.selectable_label(true, RichText::new("Module"));
                }
            });
            ui.separator();

            match self.view_mode {
                ViewMode::Overview => {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            show_overview(ui, &self.data);
                        });
                }
                ViewMode::Architecture | ViewMode::StructureGraph | ViewMode::CycleView | ViewMode::HotspotView => {
                    graph_view::show_toolbar(ui, &mut self.structure_state);
                    let result = graph_view::show_canvas(
                        ui,
                        &self.data.graph_set.structure_graph,
                        &mut self.structure_state,
                        &self.data,
                        &self.entrypoints,
                    );
                    match result {
                        ClickResult::NodeClicked(ni) => {
                            self.inspector_target =
                                InspectorTarget::from_graph_node(
                                    &self.data.graph_set.structure_graph[ni],
                                );
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
                        ClickResult::NodeClicked(ni) => {
                            self.inspector_target =
                                InspectorTarget::from_graph_node(
                                    &self.data.graph_set.call_graph[ni],
                                );
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
        CrateType::Binary => (RED, "bin"),
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
        let header = egui::CollapsingHeader::new(RichText::new(name).color(GREEN))
            .id_source(format!("m{}", mid.0))
            .default_open(search.is_empty() && module.children.len() < 10);

        let resp = header.show(ui, |ui| {
            for &cid in &module.children {
                show_module_node(
                    ui,
                    cid,
                    modules,
                    symbols,
                    select_module,
                    select_symbol,
                    search,
                );
            }
            for sid in &module.symbol_ids {
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
            stat_row(ui, "  Binaries", &bins.to_string(), RED);
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
            stat_row(ui, "  Functions", &fns.to_string(), RED);
            stat_row(ui, "  Structs", &structs.to_string(), PURPLE);
            stat_row(ui, "  Traits", &traits.to_string(), YELLOW_GREEN);
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
                YELLOW_GREEN,
            );
            stat_row(
                ui,
                "Complexity Flags",
                &data.analysis.complexity_flags.len().to_string(),
                RED,
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
            .with_decorations(false)
            .with_inner_size([1400.0, 900.0])
            .with_icon(egui::IconData::default()),
        ..Default::default()
    };

    let crate_ids: Vec<CrateId> = data.crates.iter().map(|c| c.id).collect();

    eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            let mut visuals = egui::Visuals::dark();
            visuals.panel_fill = Color32::BLACK;
            visuals.window_fill = Color32::BLACK;
            visuals.extreme_bg_color = Color32::BLACK;
            visuals.faint_bg_color = Color32::from_rgb(10, 10, 10);
            cc.egui_ctx.set_visuals(visuals);
            let mut structure_state = graph_view::GraphViewState::new_structure();
            let mut call_state = graph_view::GraphViewState::new_call();
            structure_state.init_crate_filters(&crate_ids);
            call_state.init_crate_filters(&crate_ids);
            Ok(Box::new(SpectronApp {
                data,
                view_mode: ViewMode::Architecture,
                structure_state,
                call_state,
                search: String::new(),
                inspector_target: None,
                entrypoints,
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
