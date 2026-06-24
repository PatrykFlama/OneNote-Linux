mod files;

use crate::canvas::CanvasEditor;
use crate::graph::GraphState;
use crate::project::{CanvasLayout, EditorBlock, EditorPage, EditorSection, Project};
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

type ImportResult = Result<(Project, PathBuf, Vec<String>), String>;
type NativeSaveResult = Result<(PathBuf, Project), String>;

pub struct OneNoteApp {
    project: Option<Project>,
    project_path: Option<PathBuf>,
    selected_section: usize,
    selected_page: usize,
    dirty: bool,
    status: String,
    error: Option<String>,
    loading: Option<Receiver<ImportResult>>,
    native_saving: Option<Receiver<NativeSaveResult>>,
    canvas: CanvasEditor,
    graph: GraphState,
}

impl OneNoteApp {
    pub fn new(context: &eframe::CreationContext<'_>, initial_path: Option<PathBuf>) -> Self {
        egui_extras::install_image_loaders(&context.egui_ctx);
        let mut app = Self {
            project: None,
            project_path: None,
            selected_section: 0,
            selected_page: 0,
            dirty: false,
            status: "Open a OneNote export or create a notebook.".to_owned(),
            error: None,
            loading: None,
            native_saving: None,
            canvas: CanvasEditor::default(),
            graph: GraphState::default(),
        };
        if let Some(path) = initial_path {
            app.open_path(path);
        }
        app
    }

    fn top_bar(&mut self, root_ui: &mut egui::Ui) {
        egui::Panel::top("toolbar").show_inside(root_ui, |ui| {
            let file_busy = self.loading.is_some() || self.native_saving.is_some();
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(!file_busy, egui::Button::new("New"))
                    .clicked()
                {
                    self.new_project();
                }
                if ui
                    .add_enabled(!file_busy, egui::Button::new("Open…"))
                    .clicked()
                {
                    self.choose_open();
                }
                ui.separator();
                if ui
                    .add_enabled(
                        self.project.is_some() && !file_busy,
                        egui::Button::new("Save"),
                    )
                    .clicked()
                {
                    self.save();
                }
                if ui
                    .add_enabled(
                        self.project.is_some() && !file_busy,
                        egui::Button::new("Save as…"),
                    )
                    .clicked()
                {
                    self.save_as();
                }
                if ui
                    .add_enabled(
                        self.project.is_some(),
                        egui::Button::new("Export Markdown…"),
                    )
                    .clicked()
                {
                    self.export_markdown();
                }
                if ui.button("OneDrive…").clicked() {
                    self.graph.open = true;
                }
                ui.separator();
                ui.label(if self.dirty { "Modified" } else { "Saved" });
            });
        });
    }

    fn navigation(&mut self, root_ui: &mut egui::Ui) {
        egui::Panel::left("notebook_navigation")
            .default_size(300.0)
            .min_size(220.0)
            .show_inside(root_ui, |ui| {
                let Some(project) = &mut self.project else {
                    ui.heading("OneNote Linux");
                    ui.label("No notebook open.");
                    return;
                };

                if ui.text_edit_singleline(&mut project.name).changed() {
                    self.dirty = true;
                }
                ui.separator();

                let Project {
                    sections, next_id, ..
                } = project;
                let mut delete_section = None;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (section_index, section) in sections.iter_mut().enumerate() {
                        let selected = self.selected_section == section_index;
                        egui::CollapsingHeader::new(section.name.clone())
                            .id_salt(section.id)
                            .default_open(selected)
                            .show(ui, |ui| {
                                if ui.text_edit_singleline(&mut section.name).changed() {
                                    self.dirty = true;
                                }
                                for (page_index, page) in section.pages.iter().enumerate() {
                                    if ui
                                        .selectable_label(
                                            selected && self.selected_page == page_index,
                                            &page.title,
                                        )
                                        .clicked()
                                    {
                                        self.selected_section = section_index;
                                        self.selected_page = page_index;
                                    }
                                }
                                ui.horizontal(|ui| {
                                    if ui.small_button("+ Page").clicked() {
                                        let page_id = take_id(next_id);
                                        let block_id = take_id(next_id);
                                        section.pages.push(empty_page(page_id, block_id));
                                        self.selected_section = section_index;
                                        self.selected_page = section.pages.len() - 1;
                                        self.dirty = true;
                                    }
                                    if ui.small_button("Delete section").clicked() {
                                        delete_section = Some(section_index);
                                    }
                                });
                            });
                    }
                });

                if let Some(index) = delete_section {
                    sections.remove(index);
                    self.selected_section =
                        self.selected_section.min(sections.len().saturating_sub(1));
                    self.selected_page = 0;
                    self.dirty = true;
                }

                if ui.button("+ Section").clicked() {
                    let id = take_id(next_id);
                    sections.push(EditorSection {
                        id,
                        name: "New section".to_owned(),
                        pages: Vec::new(),
                    });
                    self.selected_section = sections.len() - 1;
                    self.selected_page = 0;
                    self.dirty = true;
                }
            });
    }

    fn editor(&mut self, root_ui: &mut egui::Ui) {
        let Self {
            loading,
            project,
            selected_section,
            selected_page,
            dirty,
            status,
            error,
            canvas,
            ..
        } = self;

        egui::CentralPanel::default().show_inside(root_ui, |ui| {
            if loading.is_some() {
                ui.vertical_centered(|ui| {
                    ui.add_space(100.0);
                    ui.spinner();
                    ui.heading(status.as_str());
                });
                return;
            }

            let Some(project) = project else {
                welcome(ui);
                return;
            };
            let Project {
                sections,
                assets,
                next_id,
                ..
            } = project;
            let Some(section) = sections.get_mut(*selected_section) else {
                ui.heading("This notebook has no sections");
                return;
            };
            let Some(page) = section.pages.get_mut(*selected_page) else {
                ui.heading(&section.name);
                ui.label("This section has no pages.");
                return;
            };

            let mut delete_page = false;
            if ui
                .add(egui::TextEdit::singleline(&mut page.title).font(egui::TextStyle::Heading))
                .changed()
            {
                *dirty = true;
            }
            ui.horizontal_wrapped(|ui| {
                if let Some(author) = &page.author {
                    ui.label(format!("Author: {author}"));
                }
                if !page.updated_at.is_empty() {
                    ui.label(format!("Updated: {}", page.updated_at));
                }
                if ui.button("Delete page").clicked() {
                    delete_page = true;
                }
                ui.separator();
                if ui.button("+ Text box").clicked() {
                    page.blocks.push(EditorBlock::Text {
                        id: take_id(next_id),
                        text: String::new(),
                        indent: 0,
                        layout: CanvasLayout::default(),
                    });
                    *dirty = true;
                }
                if ui.button("+ Table").clicked() {
                    page.blocks.push(EditorBlock::Table {
                        id: take_id(next_id),
                        rows: vec![
                            vec!["Column 1".to_owned(), "Column 2".to_owned()],
                            vec![String::new(), String::new()],
                        ],
                        layout: CanvasLayout::new(120.0, 180.0, 520.0, 180.0),
                    });
                    *dirty = true;
                }
            });

            if canvas.show(ui, page, assets, next_id, error) {
                *dirty = true;
            }

            if delete_page {
                section.pages.remove(*selected_page);
                *selected_page = (*selected_page).min(section.pages.len().saturating_sub(1));
                *dirty = true;
            }
        });
    }

    fn status_bar(&mut self, root_ui: &mut egui::Ui) {
        egui::Panel::bottom("status").show_inside(root_ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label("Native .one/.onepkg: verified title and paragraph updates");
                });
            });
        });
    }
}

impl eframe::App for OneNoteApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        self.poll_import(&context);
        self.poll_native_save(&context);
        if self.graph.poll(&context, self.project.as_mut()) {
            self.dirty = true;
        }
        self.top_bar(ui);
        self.status_bar(ui);
        self.navigation(ui);
        self.editor(ui);
        self.graph.show(
            &context,
            self.project.as_ref(),
            self.project
                .as_ref()
                .map(|_| (self.selected_section, self.selected_page)),
        );

        if let Some(error) = self.error.clone() {
            egui::Window::new("Error")
                .collapsible(false)
                .resizable(true)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(&context, |ui| {
                    ui.label(error);
                    if ui.button("Close").clicked() {
                        self.error = None;
                    }
                });
        }
    }
}

fn empty_page(id: u64, block_id: u64) -> EditorPage {
    EditorPage {
        id,
        title: "New page".to_owned(),
        level: 0,
        author: None,
        created_at: String::new(),
        updated_at: String::new(),
        canvas_width: 1600.0,
        canvas_height: 1200.0,
        blocks: vec![EditorBlock::Text {
            id: block_id,
            text: String::new(),
            indent: 0,
            layout: CanvasLayout::default(),
        }],
    }
}

fn take_id(next_id: &mut u64) -> u64 {
    let id = *next_id;
    *next_id += 1;
    id
}

fn welcome(ui: &mut egui::Ui) {
    ui.vertical_centered(|ui| {
        ui.add_space(100.0);
        ui.heading("OneNote Linux");
        ui.label("Open a .onepkg, .one, .onetoc2, or .onl notebook to begin.");
        ui.add_space(12.0);
        ui.weak("Save As can create a full-fidelity .onl working copy.");
        ui.weak("Eligible title and paragraph edits can also be saved to .one or .onepkg.");
    });
}

fn safe_project_name(name: &str) -> String {
    let filtered = name
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let filtered = filtered.trim_matches('-');
    if filtered.is_empty() {
        "notebook".to_owned()
    } else {
        filtered.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_name_is_safe_for_default_files() {
        assert_eq!(safe_project_name("Algorithms / 2026"), "Algorithms---2026");
    }
}
