mod client;

use crate::project::{GRAPH_SYNC_VERSION, GraphPageLink, Project};
use client::{
    CreatedPage, DeviceCode, GraphNotebook, GraphSection, TokenSecret, UpdatePageOutcome,
};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

#[derive(Default, Serialize, Deserialize)]
struct GraphConfig {
    client_id: String,
}

enum GraphEvent {
    DeviceCode(DeviceCode),
    SignedIn(TokenSecret),
    SessionLoaded(Option<TokenSecret>),
    Notebooks(TokenSecret, Vec<GraphNotebook>),
    Sections(TokenSecret, Vec<GraphSection>),
    PageCreated {
        token: TokenSecret,
        local_page_id: u64,
        notebook: GraphNotebook,
        section: GraphSection,
        page: CreatedPage,
        warnings: Vec<String>,
    },
    PageUpdated {
        token: TokenSecret,
        local_page_id: u64,
        page: CreatedPage,
        warnings: Vec<String>,
    },
    PageConflict {
        token: TokenSecret,
        remote_modified_at: String,
    },
    SignedOut,
    Error(String),
}

pub struct GraphState {
    pub open: bool,
    config: GraphConfig,
    token: Option<TokenSecret>,
    device_code: Option<DeviceCode>,
    notebooks: Vec<GraphNotebook>,
    sections: Vec<GraphSection>,
    selected_notebook: usize,
    selected_section: usize,
    receiver: Option<Receiver<GraphEvent>>,
    busy: bool,
    status: String,
    warnings: Vec<String>,
}

impl Default for GraphState {
    fn default() -> Self {
        let mut config = load_config().unwrap_or_default();
        if let Ok(client_id) = std::env::var("ONENOTE_LINUX_CLIENT_ID")
            && !client_id.trim().is_empty()
        {
            config.client_id = client_id;
        }
        Self {
            open: false,
            config,
            token: None,
            device_code: None,
            notebooks: Vec::new(),
            sections: Vec::new(),
            selected_notebook: 0,
            selected_section: 0,
            receiver: None,
            busy: false,
            status: "Not signed in".to_owned(),
            warnings: Vec::new(),
        }
    }
}

impl GraphState {
    pub fn poll(&mut self, context: &egui::Context, mut project: Option<&mut Project>) -> bool {
        let mut project_changed = false;
        let mut received = Vec::new();
        if let Some(receiver) = &self.receiver {
            while let Ok(event) = receiver.try_recv() {
                received.push(event);
            }
        }
        for event in received {
            match event {
                GraphEvent::DeviceCode(code) => {
                    self.status = "Waiting for Microsoft sign-in".to_owned();
                    self.device_code = Some(code);
                }
                GraphEvent::SignedIn(token) => {
                    let storage_error = self.store_token(&token);
                    self.token = Some(token);
                    self.device_code = None;
                    self.busy = false;
                    self.status =
                        storage_error.unwrap_or_else(|| "Signed in to Microsoft".to_owned());
                }
                GraphEvent::SessionLoaded(token) => {
                    self.token = token;
                    self.busy = false;
                    self.status = if self.token.is_some() {
                        "Restored Microsoft session".to_owned()
                    } else {
                        "No saved Microsoft session".to_owned()
                    };
                }
                GraphEvent::Notebooks(token, notebooks) => {
                    let storage_error = self.store_token(&token);
                    self.token = Some(token);
                    self.notebooks = notebooks;
                    self.selected_notebook = self
                        .notebooks
                        .iter()
                        .position(|notebook| notebook.is_default)
                        .unwrap_or_default();
                    self.sections.clear();
                    self.busy = false;
                    self.status = storage_error
                        .unwrap_or_else(|| format!("Loaded {} notebook(s)", self.notebooks.len()));
                }
                GraphEvent::Sections(token, sections) => {
                    let storage_error = self.store_token(&token);
                    self.token = Some(token);
                    self.sections = sections;
                    self.selected_section = self
                        .sections
                        .iter()
                        .position(|section| section.is_default)
                        .unwrap_or_default();
                    self.busy = false;
                    self.status = storage_error
                        .unwrap_or_else(|| format!("Loaded {} section(s)", self.sections.len()));
                }
                GraphEvent::PageCreated {
                    token,
                    local_page_id,
                    notebook,
                    section,
                    page,
                    warnings,
                } => {
                    let storage_error = self.store_token(&token);
                    self.token = Some(token);
                    self.busy = false;
                    self.status =
                        storage_error.unwrap_or_else(|| format!("Uploaded page “{}”", page.title));
                    self.warnings = warnings;
                    if let Some(project) = project.as_deref_mut() {
                        project.graph_sync.notebook_id = Some(notebook.id);
                        project.graph_sync.notebook_name = Some(notebook.display_name);
                        project.graph_sync.section_id = Some(section.id);
                        project.graph_sync.section_name = Some(section.display_name);
                        project.graph_sync.pages.insert(
                            local_page_id,
                            GraphPageLink {
                                graph_page_id: page.id,
                                uploaded_at: page.last_modified_date_time,
                                sync_version: GRAPH_SYNC_VERSION,
                            },
                        );
                        project_changed = true;
                    }
                }
                GraphEvent::PageUpdated {
                    token,
                    local_page_id,
                    page,
                    warnings,
                } => {
                    let storage_error = self.store_token(&token);
                    self.token = Some(token);
                    self.busy = false;
                    self.status =
                        storage_error.unwrap_or_else(|| format!("Updated page “{}”", page.title));
                    self.warnings = warnings;
                    if let Some(project) = project.as_deref_mut()
                        && let Some(link) = project.graph_sync.pages.get_mut(&local_page_id)
                    {
                        link.graph_page_id = page.id;
                        link.uploaded_at = page.last_modified_date_time;
                        link.sync_version = GRAPH_SYNC_VERSION;
                        project_changed = true;
                    }
                }
                GraphEvent::PageConflict {
                    token,
                    remote_modified_at,
                } => {
                    let storage_error = self.store_token(&token);
                    self.token = Some(token);
                    self.busy = false;
                    self.status = storage_error.unwrap_or_else(|| {
                        format!(
                            "Update blocked: the OneNote page changed remotely at \
                             {remote_modified_at}"
                        )
                    });
                    self.warnings.clear();
                }
                GraphEvent::SignedOut => {
                    self.token = None;
                    self.notebooks.clear();
                    self.sections.clear();
                    self.busy = false;
                    self.status = "Signed out".to_owned();
                }
                GraphEvent::Error(error) => {
                    self.busy = false;
                    self.status = error;
                }
            }
        }
        if self.busy {
            context.request_repaint_after(std::time::Duration::from_millis(100));
        }
        project_changed
    }

    pub fn show(
        &mut self,
        context: &egui::Context,
        project: Option<&Project>,
        page: Option<(usize, usize)>,
    ) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        egui::Window::new("Microsoft OneDrive / OneNote")
            .open(&mut open)
            .default_width(560.0)
            .show(context, |ui| {
                ui.label("Microsoft Entra application client ID");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.config.client_id);
                    if ui.button("Save").clicked() {
                        match save_config(&self.config) {
                            Ok(()) => self.status = "Saved Microsoft configuration".to_owned(),
                            Err(error) => self.status = error,
                        }
                    }
                });
                ui.weak("You can also set ONENOTE_LINUX_CLIENT_ID.");
                ui.separator();

                if self.token.is_none() {
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!self.busy, egui::Button::new("Sign in"))
                            .clicked()
                        {
                            self.start_sign_in();
                        }
                        if ui
                            .add_enabled(!self.busy, egui::Button::new("Use saved sign-in"))
                            .clicked()
                        {
                            self.load_saved_session();
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!self.busy, egui::Button::new("Load notebooks"))
                            .clicked()
                        {
                            self.load_notebooks();
                        }
                        if ui
                            .add_enabled(!self.busy, egui::Button::new("Sign out"))
                            .clicked()
                        {
                            self.sign_out();
                        }
                    });
                }

                if let Some(code) = &self.device_code {
                    ui.separator();
                    ui.label(&code.message);
                    ui.horizontal(|ui| {
                        ui.monospace(&code.user_code);
                        if ui.small_button("Copy code").clicked() {
                            ui.ctx().copy_text(code.user_code.clone());
                        }
                    });
                    ui.hyperlink_to("Open Microsoft sign-in page", &code.verification_uri);
                }

                if !self.notebooks.is_empty() {
                    ui.separator();
                    egui::ComboBox::from_label("Notebook")
                        .selected_text(
                            self.notebooks
                                .get(self.selected_notebook)
                                .map(|notebook| notebook.display_name.as_str())
                                .unwrap_or("Select notebook"),
                        )
                        .show_ui(ui, |ui| {
                            for (index, notebook) in self.notebooks.iter().enumerate() {
                                if ui
                                    .selectable_value(
                                        &mut self.selected_notebook,
                                        index,
                                        &notebook.display_name,
                                    )
                                    .changed()
                                {
                                    self.sections.clear();
                                }
                            }
                        });
                    if ui
                        .add_enabled(!self.busy, egui::Button::new("Load sections"))
                        .clicked()
                    {
                        self.load_sections();
                    }
                }

                if !self.sections.is_empty() {
                    egui::ComboBox::from_label("Section")
                        .selected_text(
                            self.sections
                                .get(self.selected_section)
                                .map(|section| section.display_name.as_str())
                                .unwrap_or("Select section"),
                        )
                        .show_ui(ui, |ui| {
                            for (index, section) in self.sections.iter().enumerate() {
                                ui.selectable_value(
                                    &mut self.selected_section,
                                    index,
                                    &section.display_name,
                                );
                            }
                        });
                }

                if let (Some(project), Some((section_index, page_index))) = (project, page)
                    && let Some(local_page) = project
                        .sections
                        .get(section_index)
                        .and_then(|section| section.pages.get(page_index))
                {
                    if let Some(link) = project.graph_sync.pages.get(&local_page.id) {
                        ui.label("This page is linked to OneNote.");
                        if link.sync_version == GRAPH_SYNC_VERSION {
                            if ui
                                .add_enabled(
                                    !self.busy && self.token.is_some(),
                                    egui::Button::new("Update linked page"),
                                )
                                .clicked()
                            {
                                let section_name = project
                                    .graph_sync
                                    .section_name
                                    .as_deref()
                                    .unwrap_or("OneNote Linux");
                                self.update_page(local_page, link.clone(), section_name);
                            }
                            ui.weak(
                                "The update is blocked if OneNote changed the remote page \
                                 after the last successful sync.",
                            );
                        } else {
                            ui.weak(
                                "This link predates update support and cannot be patched safely.",
                            );
                        }
                    } else if !self.sections.is_empty()
                        && ui
                            .add_enabled(
                                !self.busy && self.token.is_some(),
                                egui::Button::new("Upload current page"),
                            )
                            .clicked()
                    {
                        self.upload_page(local_page);
                    }
                }

                ui.separator();
                if self.busy {
                    ui.spinner();
                }
                ui.label(&self.status);
                for warning in &self.warnings {
                    ui.colored_label(egui::Color32::YELLOW, warning);
                }
            });
        self.open = open;
    }

    fn start_sign_in(&mut self) {
        let client_id = self.config.client_id.trim().to_owned();
        if client_id.is_empty() {
            self.status = "Configure a Microsoft Entra client ID first".to_owned();
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.busy = true;
        self.device_code = None;
        self.status = "Requesting Microsoft sign-in code…".to_owned();
        std::thread::spawn(move || match client::request_device_code(&client_id) {
            Ok(code) => {
                let _ = sender.send(GraphEvent::DeviceCode(code.clone()));
                match client::poll_device_code(&client_id, &code) {
                    Ok(token) => {
                        let _ = sender.send(GraphEvent::SignedIn(token));
                    }
                    Err(error) => {
                        let _ = sender.send(GraphEvent::Error(error));
                    }
                }
            }
            Err(error) => {
                let _ = sender.send(GraphEvent::Error(error));
            }
        });
    }

    fn load_saved_session(&mut self) {
        let client_id = self.config.client_id.trim().to_owned();
        if client_id.is_empty() {
            self.status = "Configure a Microsoft Entra client ID first".to_owned();
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.busy = true;
        std::thread::spawn(move || {
            let event = match client::load_token(&client_id) {
                Ok(token) => GraphEvent::SessionLoaded(token),
                Err(error) => GraphEvent::Error(error),
            };
            let _ = sender.send(event);
        });
    }

    fn load_notebooks(&mut self) {
        let Some(token) = self.token.clone() else {
            return;
        };
        let client_id = self.config.client_id.clone();
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.busy = true;
        std::thread::spawn(move || {
            let event = match client::list_notebooks(&client_id, token) {
                Ok((token, notebooks)) => GraphEvent::Notebooks(token, notebooks),
                Err(error) => GraphEvent::Error(error),
            };
            let _ = sender.send(event);
        });
    }

    fn load_sections(&mut self) {
        let (Some(token), Some(notebook)) = (
            self.token.clone(),
            self.notebooks.get(self.selected_notebook).cloned(),
        ) else {
            return;
        };
        let client_id = self.config.client_id.clone();
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.busy = true;
        std::thread::spawn(move || {
            let event = match client::list_sections(&client_id, token, &notebook.id) {
                Ok((token, sections)) => GraphEvent::Sections(token, sections),
                Err(error) => GraphEvent::Error(error),
            };
            let _ = sender.send(event);
        });
    }

    fn upload_page(&mut self, page: &crate::project::EditorPage) {
        let (Some(token), Some(notebook), Some(section)) = (
            self.token.clone(),
            self.notebooks.get(self.selected_notebook).cloned(),
            self.sections.get(self.selected_section).cloned(),
        ) else {
            return;
        };
        let sync_page = match page.to_graph_page(&section.display_name) {
            Ok(page) => page,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        let warnings = sync_page
            .export
            .warnings
            .iter()
            .map(|warning| warning.message.clone())
            .collect();
        let local_page_id = page.id;
        let client_id = self.config.client_id.clone();
        let section_id = section.id.clone();
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.busy = true;
        self.status = format!("Uploading “{}”…", page.title);
        std::thread::spawn(move || {
            let event = match client::create_page(&client_id, token, &section_id, &sync_page.export)
            {
                Ok((token, page)) => GraphEvent::PageCreated {
                    token,
                    local_page_id,
                    notebook,
                    section,
                    page,
                    warnings,
                },
                Err(error) => GraphEvent::Error(error),
            };
            let _ = sender.send(event);
        });
    }

    fn update_page(
        &mut self,
        page: &crate::project::EditorPage,
        link: GraphPageLink,
        section_name: &str,
    ) {
        let Some(token) = self.token.clone() else {
            return;
        };
        let sync_page = match page.to_graph_page(section_name) {
            Ok(page) => page,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        let warnings = sync_page
            .export
            .warnings
            .iter()
            .map(|warning| warning.message.clone())
            .collect();
        let local_page_id = page.id;
        let page_id = link.graph_page_id;
        let expected_modified_at = link.uploaded_at;
        let client_id = self.config.client_id.clone();
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.busy = true;
        self.status = format!("Checking and updating “{}”…", page.title);
        std::thread::spawn(move || {
            let event = match client::update_page(
                &client_id,
                token,
                &page_id,
                &expected_modified_at,
                &sync_page,
            ) {
                Ok(UpdatePageOutcome::Updated(token, page)) => GraphEvent::PageUpdated {
                    token,
                    local_page_id,
                    page,
                    warnings,
                },
                Ok(UpdatePageOutcome::Conflict {
                    token,
                    remote_modified_at,
                }) => GraphEvent::PageConflict {
                    token,
                    remote_modified_at,
                },
                Err(error) => GraphEvent::Error(error),
            };
            let _ = sender.send(event);
        });
    }

    fn sign_out(&mut self) {
        let client_id = self.config.client_id.clone();
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.busy = true;
        std::thread::spawn(move || {
            let event = match client::delete_token(&client_id) {
                Ok(()) => GraphEvent::SignedOut,
                Err(error) => GraphEvent::Error(error),
            };
            let _ = sender.send(event);
        });
    }

    fn store_token(&self, token: &TokenSecret) -> Option<String> {
        client::save_token(&self.config.client_id, token)
            .err()
            .map(|error| format!("Microsoft operation succeeded, but {error}"))
    }
}

fn config_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(path)
            .join("onenote-linux")
            .join("config.json"));
    }
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("onenote-linux")
        .join("config.json"))
}

fn load_config() -> Result<GraphConfig, String> {
    let path = config_path()?;
    match fs::read(&path) {
        Ok(data) => serde_json::from_slice(&data).map_err(|error| error.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(GraphConfig::default()),
        Err(error) => Err(format!("failed to read {}: {error}", path.display())),
    }
}

fn save_config(config: &GraphConfig) -> Result<(), String> {
    let path = config_path()?;
    let parent = path.parent().ok_or("invalid configuration path")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    fs::write(
        &path,
        serde_json::to_vec_pretty(config).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed to write {}: {error}", path.display()))
}
