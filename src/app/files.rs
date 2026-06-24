use super::{ImportResult, NativeSaveResult, OneNoteApp, safe_project_name};
use crate::project::Project;
use eframe::egui;
use libonenote::{BinaryDataPolicy, InkDataPolicy, Loader};
use rfd::FileDialog;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FileFormat {
    WorkingCopy,
    Section,
    Package,
    NotebookIndex,
    Unsupported,
}

impl OneNoteApp {
    pub(super) fn new_project(&mut self) {
        self.project = Some(Project::empty());
        self.project_path = None;
        self.selected_section = 0;
        self.selected_page = 0;
        self.dirty = true;
        self.status = "New working copy".to_owned();
    }

    pub(super) fn choose_open(&mut self) {
        if let Some(path) = FileDialog::new()
            .set_title("Open a notebook")
            .add_filter(
                "All supported notebooks",
                &["onl", "one", "onepkg", "onetoc2"],
            )
            .add_filter("Microsoft OneNote", &["one", "onepkg", "onetoc2"])
            .add_filter("OneNote Linux project", &["onl"])
            .pick_file()
        {
            self.open_path(path);
        }
    }

    pub(super) fn open_path(&mut self, path: PathBuf) {
        match file_format(&path) {
            FileFormat::WorkingCopy => self.open_project(path),
            FileFormat::Section | FileFormat::Package | FileFormat::NotebookIndex => {
                self.begin_import(path);
            }
            FileFormat::Unsupported => {
                self.error = Some(format!(
                    "Unsupported notebook type: {}. Open a .onl, .one, .onepkg, or .onetoc2 file.",
                    path.display()
                ));
            }
        }
    }

    fn begin_import(&mut self, path: PathBuf) {
        let (sender, receiver) = mpsc::channel();
        self.loading = Some(receiver);
        self.status = format!("Opening {}…", path.display());
        self.error = None;

        std::thread::spawn(move || {
            let result: ImportResult = Loader::new()
                .binary_data(BinaryDataPolicy::UpTo(64 * 1024 * 1024))
                .ink_data(InkDataPolicy::All)
                .open(&path)
                .map(|document| {
                    let diagnostics = document
                        .diagnostics()
                        .iter()
                        .map(|diagnostic| diagnostic.message.clone())
                        .collect();
                    (Project::import(&document), path, diagnostics)
                })
                .map_err(|error| error.to_string());
            let _ = sender.send(result);
        });
    }

    pub(super) fn poll_import(&mut self, context: &egui::Context) {
        let Some(receiver) = &self.loading else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok((project, source, diagnostics))) => {
                self.project = Some(project);
                self.project_path = match file_format(&source) {
                    FileFormat::Section | FileFormat::Package => Some(source.clone()),
                    FileFormat::WorkingCopy
                    | FileFormat::NotebookIndex
                    | FileFormat::Unsupported => None,
                };
                self.selected_section = 0;
                self.selected_page = 0;
                self.dirty = false;
                self.status = if diagnostics.is_empty() {
                    match file_format(&source) {
                        FileFormat::NotebookIndex => format!(
                            "Opened {} (use Save As to choose a writable file)",
                            source.display()
                        ),
                        _ => format!("Opened {}", source.display()),
                    }
                } else {
                    format!(
                        "Opened {} with {} parser warning(s)",
                        source.display(),
                        diagnostics.len()
                    )
                };
                self.loading = None;
            }
            Ok(Err(error)) => {
                self.error = Some(error);
                self.status = "Open failed".to_owned();
                self.loading = None;
            }
            Err(mpsc::TryRecvError::Empty) => {
                context.request_repaint_after(Duration::from_millis(100));
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.error = Some("Importer stopped unexpectedly".to_owned());
                self.loading = None;
            }
        }
    }

    fn open_project(&mut self, path: PathBuf) {
        match Project::load(&path) {
            Ok(project) => {
                self.project = Some(project);
                self.project_path = Some(path.clone());
                self.selected_section = 0;
                self.selected_page = 0;
                self.dirty = false;
                self.status = format!("Opened {}", path.display());
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    pub(super) fn save(&mut self) {
        let path = if let Some(path) = self.project_path.clone() {
            Some(path)
        } else {
            self.project.as_ref().and_then(choose_save_path)
        };
        let Some(path) = path else {
            return;
        };
        self.save_to(path);
    }

    pub(super) fn save_as(&mut self) {
        let Some(project) = &self.project else {
            return;
        };
        let Some(path) = choose_save_path(project) else {
            return;
        };
        self.save_to(path);
    }

    fn save_to(&mut self, path: PathBuf) {
        match file_format(&path) {
            FileFormat::WorkingCopy => {
                let Some(project) = &self.project else {
                    return;
                };
                match project.save(&path) {
                    Ok(()) => {
                        self.project_path = Some(path.clone());
                        self.dirty = false;
                        self.status = format!("Saved {}", path.display());
                        self.error = None;
                    }
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
            FileFormat::Section | FileFormat::Package => self.start_native_save(path),
            FileFormat::NotebookIndex => {
                self.error = Some(
                    "Saving to .onetoc2 is not supported. Save as .onl, or use the matching \
                     .one/.onepkg type from the original notebook."
                        .to_owned(),
                );
            }
            FileFormat::Unsupported => {
                self.error =
                    Some("Unsupported output type. Use .onl, .one, or .onepkg.".to_owned());
            }
        }
    }

    pub(super) fn export_markdown(&mut self) {
        let Some(project) = &self.project else {
            return;
        };
        let Some(path) = FileDialog::new()
            .set_title("Export notebook as Markdown")
            .set_file_name(format!("{}.md", safe_project_name(&project.name)))
            .add_filter("Markdown", &["md"])
            .save_file()
        else {
            return;
        };
        match project.export_markdown(&path) {
            Ok(()) => self.status = format!("Exported {}", path.display()),
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn start_native_save(&mut self, output: PathBuf) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let Some(source) = project.source.as_deref().map(PathBuf::from) else {
            self.error =
                Some("This project has no native OneNote baseline. Save it as .onl.".to_owned());
            return;
        };

        let source_format = file_format(&source);
        let output_format = file_format(&output);
        if !matches!(source_format, FileFormat::Section | FileFormat::Package) {
            self.error = Some(format!(
                "{} is not a writable .one or .onepkg native baseline. Save as .onl.",
                source.display()
            ));
            return;
        }
        if source_format != output_format {
            self.error = Some(format!(
                "Native container conversion is not implemented: {} can only be saved as another \
                 .{}, or as .onl.",
                source.display(),
                native_extension(source_format)
            ));
            return;
        }

        let (sender, receiver) = mpsc::channel();
        self.native_saving = Some(receiver);
        self.status = format!("Saving {}…", output.display());
        self.error = None;
        std::thread::spawn(move || {
            let result: NativeSaveResult = (|| {
                let mut document = Loader::new()
                    .binary_data(BinaryDataPolicy::UpTo(64 * 1024 * 1024))
                    .ink_data(InkDataPolicy::All)
                    .preserve_original(true)
                    .open(&source)
                    .map_err(|error| error.to_string())?;
                let plan = project
                    .native_edit_plan(&document)
                    .map_err(|error| error.to_string())?;
                if !plan.is_empty() {
                    document
                        .edit(|notebook| plan.apply(notebook))
                        .map_err(|error| error.to_string())?;
                }
                document
                    .save_native(&output)
                    .map_err(|error| error.to_string())?;
                Ok((output, project))
            })();
            let _ = sender.send(result);
        });
    }

    pub(super) fn poll_native_save(&mut self, context: &egui::Context) {
        let Some(receiver) = &self.native_saving else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok((path, saved_project))) => {
                let unchanged_since_save = self
                    .project
                    .as_ref()
                    .is_some_and(|project| same_project(project, &saved_project));
                if let Some(project) = &mut self.project {
                    project.source = Some(path.display().to_string());
                }
                self.project_path = Some(path.clone());
                self.dirty = !unchanged_since_save;
                self.status = if unchanged_since_save {
                    format!("Saved {}", path.display())
                } else {
                    format!(
                        "Saved {}; newer in-app edits remain unsaved",
                        path.display()
                    )
                };
                self.error = None;
                self.native_saving = None;
            }
            Ok(Err(error)) => {
                self.status = "Native save rejected".to_owned();
                self.error = Some(error);
                self.native_saving = None;
            }
            Err(mpsc::TryRecvError::Empty) => {
                context.request_repaint_after(Duration::from_millis(100));
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.status = "Native save failed".to_owned();
                self.error = Some("Native writer stopped unexpectedly".to_owned());
                self.native_saving = None;
            }
        }
    }
}

fn choose_save_path(project: &Project) -> Option<PathBuf> {
    let mut dialog = FileDialog::new()
        .set_title("Save notebook as")
        .set_file_name(format!("{}.onl", safe_project_name(&project.name)))
        .add_filter("OneNote Linux project", &["onl"]);

    if let Some(source) = project.source.as_deref() {
        match file_format(Path::new(source)) {
            FileFormat::Section => {
                dialog = dialog.add_filter("Microsoft OneNote section", &["one"]);
            }
            FileFormat::Package => {
                dialog = dialog.add_filter("Microsoft OneNote package", &["onepkg"]);
            }
            FileFormat::WorkingCopy | FileFormat::NotebookIndex | FileFormat::Unsupported => {}
        }
    }

    dialog.save_file()
}

fn file_format(path: &Path) -> FileFormat {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("onl") => FileFormat::WorkingCopy,
        Some("one") => FileFormat::Section,
        Some("onepkg") => FileFormat::Package,
        Some("onetoc2") => FileFormat::NotebookIndex,
        _ => FileFormat::Unsupported,
    }
}

fn native_extension(format: FileFormat) -> &'static str {
    match format {
        FileFormat::Section => "one",
        FileFormat::Package => "onepkg",
        _ => "onl",
    }
}

fn same_project(left: &Project, right: &Project) -> bool {
    match (serde_json::to_value(left), serde_json::to_value(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_all_openable_formats_case_insensitively() {
        assert_eq!(
            file_format(Path::new("Notebook.onl")),
            FileFormat::WorkingCopy
        );
        assert_eq!(file_format(Path::new("Section.ONE")), FileFormat::Section);
        assert_eq!(
            file_format(Path::new("Notebook.OnePkg")),
            FileFormat::Package
        );
        assert_eq!(
            file_format(Path::new("Open Notebook.onetoc2")),
            FileFormat::NotebookIndex
        );
        assert_eq!(file_format(Path::new("notes.txt")), FileFormat::Unsupported);
    }
}
