//! A small file browser drawn by egui, for the commands that open or save a file on the
//! desktop: no dialog library, the same on every system.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Open,
    Save,
    Folder,
}

pub struct FileBrowser {
    mode: Mode,
    extensions: &'static [&'static str],
    folder: PathBuf,
    name: String,
    entries: Vec<(String, bool)>,
    error: Option<String>,
    all_files: bool,
    /// The File field takes the keyboard at the first drawing: a name typed, Enter opens it.
    focus_file_field: bool,
}

/// The folder last browsed, where the next browser opens.
static LAST_FOLDER: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

impl FileBrowser {
    fn new(mode: Mode, extensions: &'static [&'static str], name: &str) -> Self {
        let folder = LAST_FOLDER
            .lock()
            .ok()
            .and_then(|last| last.clone())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let mut browser = Self {
            mode,
            extensions,
            folder,
            name: name.to_owned(),
            entries: Vec::new(),
            error: None,
            all_files: extensions.is_empty(),
            focus_file_field: true,
        };
        browser.read();
        browser
    }

    pub fn open(extensions: &'static [&'static str]) -> Self {
        Self::new(Mode::Open, extensions, "")
    }

    pub fn save(extensions: &'static [&'static str], name: &str) -> Self {
        Self::new(Mode::Save, extensions, name)
    }

    pub fn folder() -> Self {
        Self::new(Mode::Folder, &[], "")
    }

    fn shown(&self, name: &str) -> bool {
        self.all_files
            || self.extensions.iter().any(|ext| {
                name.to_lowercase()
                    .rsplit_once('.')
                    .is_some_and(|(_, e)| e == *ext)
            })
    }

    fn read(&mut self) {
        self.entries.clear();
        self.error = None;
        match std::fs::read_dir(&self.folder) {
            Ok(dir) => {
                for entry in dir.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') {
                        continue;
                    }
                    let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
                    if is_dir || self.mode == Mode::Folder || self.shown(&name) {
                        self.entries.push((name, is_dir));
                    }
                }
                self.entries.sort_by(|a, b| {
                    b.1.cmp(&a.1)
                        .then(a.0.to_lowercase().cmp(&b.0.to_lowercase()))
                });
            }
            Err(e) => self.error = Some(format!("{}: {e}", self.folder.display())),
        }
    }

    fn enter(&mut self, folder: &Path) {
        self.folder = folder.to_path_buf();
        if let Ok(mut last) = LAST_FOLDER.lock() {
            *last = Some(self.folder.clone());
        }
        self.read();
    }

    /// The browser; the path chosen, once it is.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<PathBuf> {
        let mut chosen = None;
        ui.horizontal(|ui| {
            if ui.button("⬆").on_hover_text("Parent folder").clicked() {
                if let Some(parent) = self.folder.parent().map(Path::to_path_buf) {
                    self.enter(&parent);
                }
            }
            let mut text = self.folder.display().to_string();
            if ui
                .add(egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY))
                .changed()
            {
                let path = PathBuf::from(&text);
                if path.is_dir() {
                    self.enter(&path);
                } else {
                    self.folder = path;
                }
            }
        });
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        let mut enter = None;
        egui::ScrollArea::vertical()
            .max_height(280.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (name, is_dir) in &self.entries {
                    let label = if *is_dir {
                        format!("📁 {name}")
                    } else {
                        name.clone()
                    };
                    let response = ui.selectable_label(!is_dir && *name == self.name, label);
                    if response.clicked() {
                        if *is_dir {
                            enter = Some(self.folder.join(name));
                        } else {
                            self.name = name.clone();
                        }
                    }
                    if response.double_clicked() && !is_dir {
                        chosen = Some(self.folder.join(name));
                    }
                }
            });
        if let Some(folder) = enter {
            self.enter(&folder);
        }
        ui.horizontal(|ui| {
            // Enter in the file's field is a click on its button
            let mut submitted = false;
            if self.mode != Mode::Folder {
                ui.label("File:");
                // (read before the field, which may take the key)
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                let field = ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(260.0));
                if std::mem::take(&mut self.focus_file_field) {
                    field.request_focus();
                }
                submitted = field.lost_focus() && enter;
            }
            let verb = match self.mode {
                Mode::Open => "Open",
                Mode::Save => "Save",
                Mode::Folder => "Choose this folder",
            };
            if ui.button(verb).clicked() || submitted {
                chosen = match self.mode {
                    Mode::Folder => Some(self.folder.clone()),
                    _ if self.name.is_empty() => None,
                    _ => Some(self.folder.join(&self.name)),
                };
            }
            if self.mode == Mode::Folder {
                // a FAT image is chosen as a file (the Albireo)
                if !self.name.is_empty() && ui.button("This file").clicked() {
                    chosen = Some(self.folder.join(&self.name));
                }
            } else if !self.extensions.is_empty()
                && ui.checkbox(&mut self.all_files, "All files").changed()
            {
                self.read();
            }
        });
        chosen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame of the browser with `events`; the path it chose, if it did.
    fn frame(
        ctx: &egui::Context,
        browser: &mut FileBrowser,
        events: Vec<egui::Event>,
    ) -> Option<PathBuf> {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(700.0, 500.0),
            )),
            events,
            focused: true,
            ..Default::default()
        };
        let mut chosen = None;
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| chosen = browser.show(ui));
        });
        chosen
    }

    /// Enter in the File field opens the file, as the Open button does.
    #[test]
    fn enter_in_the_file_field_opens_the_file() {
        let ctx = egui::Context::default();
        let mut browser = FileBrowser::open(&["csl"]);
        browser.name = "script.csl".to_owned();
        assert_eq!(frame(&ctx, &mut browser, Vec::new()), None);
        // the field has the keyboard from the start, Enter is typed
        assert_eq!(frame(&ctx, &mut browser, Vec::new()), None);
        let key = |pressed| egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: Some(egui::Key::Enter),
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let chosen = frame(&ctx, &mut browser, vec![key(true), key(false)]);
        assert_eq!(chosen, Some(browser.folder.join("script.csl")));
        // with nothing in the field, Enter chooses nothing
        browser.name.clear();
        browser.focus_file_field = true;
        frame(&ctx, &mut browser, Vec::new());
        frame(&ctx, &mut browser, Vec::new());
        assert_eq!(frame(&ctx, &mut browser, vec![key(true), key(false)]), None);
    }
}
