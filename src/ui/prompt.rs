//! The question a command asks for its argument: a text, an address, a value, a choice, a file.

use crate::commands::{Arg, ArgKind, ArgPrompt, CommandTarget};

use super::files::FileBrowser;

pub enum Answer {
    Waiting,
    Cancelled,
    Given(&'static str, Arg),
    /// A value applied while the window stays open (the monitor's knobs).
    Live(&'static str, Arg),
}

/// The settings that are tried by ear or by eye: their window applies each change at once, and
/// its Reset goes back to the value it opened with.
const LIVE: [&str; 5] = [
    "video-hhold-set",
    "video-vhold-set",
    "video-curvature-set",
    "video-brightness-set",
    "audio-volume-set",
];

pub struct Prompt {
    question: ArgPrompt,
    text: String,
    number: i64,
    decimal: f64,
    /// The value the window opened with, for Reset.
    initial: (i64, f64),
    browser: Option<FileBrowser>,
    focused: bool,
}

impl Prompt {
    pub fn new(question: ArgPrompt) -> Self {
        let number = match question.kind {
            ArgKind::Number { default, .. } => default,
            _ => 0,
        };
        let decimal = match question.kind {
            ArgKind::Decimal { default, .. } => default,
            _ => 0.0,
        };
        let browser = match question.kind {
            ArgKind::OpenFile(extensions) => Some(FileBrowser::open(extensions)),
            ArgKind::SaveFile(extensions, name) => Some(FileBrowser::save(extensions, name)),
            ArgKind::Folder => Some(FileBrowser::folder()),
            _ => None,
        };
        Self {
            question,
            text: String::new(),
            number,
            decimal,
            initial: (number, decimal),
            browser,
            focused: false,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, target: &dyn CommandTarget) -> Answer {
        let mut answer = Answer::Waiting;
        let mut open = true;
        let mut close = false;
        let id = self.question.id;
        let escape = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        egui::Window::new(self.question.title)
            .id(egui::Id::new(("prompt", id)))
            .collapsible(false)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(460.0)
            .show(ctx, |ui| {
                if let Some(browser) = &mut self.browser {
                    if let Some(path) = browser.show(ui) {
                        answer = Answer::Given(id, Arg::Path(path));
                    }
                    return;
                }
                match &self.question.kind {
                    ArgKind::Choice(choices) => {
                        for (value, label) in *choices {
                            if ui.button(*label).clicked() {
                                answer = Answer::Given(id, Arg::Text((*value).to_owned()));
                            }
                        }
                    }
                    ArgKind::Decimal { min, max, .. } if LIVE.contains(&id) => {
                        ui.horizontal(|ui| {
                            let changed = ui
                                .add(
                                    egui::Slider::new(&mut self.decimal, *min..=*max)
                                        .clamping(egui::SliderClamping::Always),
                                )
                                .changed();
                            let reset = ui
                                .button("Reset")
                                .on_hover_text("Back to the value it had when this window opened")
                                .clicked();
                            if reset {
                                self.decimal = self.initial.1;
                            }
                            if changed || reset {
                                answer = Answer::Live(id, Arg::Text(self.decimal.to_string()));
                            }
                            if ui.button("Close").clicked() {
                                close = true;
                            }
                        });
                    }
                    ArgKind::Number { min, max, .. } if LIVE.contains(&id) => {
                        ui.horizontal(|ui| {
                            let changed = ui
                                .add(
                                    egui::Slider::new(&mut self.number, *min..=*max)
                                        .logarithmic(*max - *min > 1000)
                                        .clamping(egui::SliderClamping::Always),
                                )
                                .changed();
                            let reset = ui
                                .button("Reset")
                                .on_hover_text("Back to the value it had when this window opened")
                                .clicked();
                            if reset {
                                self.number = self.initial.0;
                            }
                            if changed || reset {
                                answer = Answer::Live(id, Arg::Number(self.number));
                            }
                            if ui.button("Close").clicked() {
                                close = true;
                            }
                        });
                    }
                    ArgKind::Decimal { min, max, .. } => {
                        ui.horizontal(|ui| {
                            // the slider and its field (typed in, or dragged)
                            ui.add(
                                egui::Slider::new(&mut self.decimal, *min..=*max)
                                    .clamping(egui::SliderClamping::Always),
                            );
                            if ui.button("OK").clicked()
                                || ui.input(|i| i.key_pressed(egui::Key::Enter))
                            {
                                answer = Answer::Given(id, Arg::Text(self.decimal.to_string()));
                            }
                        });
                    }
                    ArgKind::Number { min, max, .. } => {
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::Slider::new(&mut self.number, *min..=*max)
                                    .logarithmic(*max - *min > 1000)
                                    .clamping(egui::SliderClamping::Always),
                            );
                            if ui.button("OK").clicked()
                                || ui.input(|i| i.key_pressed(egui::Key::Enter))
                            {
                                answer = Answer::Given(id, Arg::Number(self.number));
                            }
                        });
                    }
                    _ => {
                        let field = ui.add(
                            egui::TextEdit::singleline(&mut self.text)
                                .desired_width(f32::INFINITY)
                                .hint_text(match self.question.kind {
                                    ArgKind::Address => "&BE00, #38, 1000, or a symbol",
                                    _ => "",
                                }),
                        );
                        if !self.focused {
                            field.request_focus();
                            self.focused = true;
                        }
                        if let (ArgKind::Address, Some(debugger)) =
                            (&self.question.kind, target.debugger())
                        {
                            if let Some(address) = debugger.symbols().resolve(self.text.trim()) {
                                ui.weak(format!("= &{address:04X}"));
                            }
                        }
                        let entered =
                            field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if ui.button("OK").clicked() || entered {
                            answer = Answer::Given(id, Arg::Text(self.text.clone()));
                        }
                    }
                }
            });
        if !open || escape || close {
            return Answer::Cancelled;
        }
        answer
    }
}
