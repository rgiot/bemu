//! The Watch window: addresses, symbols and expressions with their values now, lit when they
//! change (see [`super::heat`]). An address or a symbol is the byte (or the word) there; an
//! expression is anything a breakpoint condition understands - a register (`HL`), a byte or a
//! word (`peek(HL)`, `deek(&BE00)`), arithmetic on them. An address can also stop the machine
//! when its value changes: a write breakpoint whose condition is `VALUE != OLDVALUE`.

use super::views::{number, push};
use super::window::{Cx, ViewWindow};
use crate::breakpoints::Condition;
use crate::commands::{Arg, CommandBus, CommandTarget};
use crate::symbols::SymbolTable;

/// How many bytes an address watch shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Size {
    #[default]
    Byte,
    Word,
}

impl Size {
    fn bytes(self) -> u16 {
        match self {
            Self::Byte => 1,
            Self::Word => 2,
        }
    }
}

/// One thing watched, as the user typed it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Watch {
    pub text: String,
    #[serde(default)]
    pub size: Size,
}

/// What a watch's text means.
#[derive(Debug, PartialEq)]
pub enum Subject {
    /// The byte or word at this address.
    Address(u16),
    Expression(Condition),
}

/// `text` as a number (`&BE00`, `#38`, `0x4000`, `49152`), a symbol of `symbols`, or else an
/// expression.
pub fn parse(text: &str, symbols: Option<&SymbolTable>) -> Result<Subject, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("nothing to watch".to_owned());
    }
    if let Some(address) = number(text) {
        return Ok(Subject::Address(address));
    }
    if let Some(address) = symbols.and_then(|s| s.resolve(text)) {
        return Ok(Subject::Address(address));
    }
    Condition::parse(text).map(Subject::Expression)
}

/// The value of `subject` on the machine now.
pub fn value(subject: &Subject, size: Size, mb: &crate::motherboard::Motherboard) -> i64 {
    match subject {
        Subject::Address(address) => match size {
            Size::Byte => i64::from(mb.read_byte(*address)),
            Size::Word => i64::from(mb.read_word(*address)),
        },
        Subject::Expression(condition) => crate::debug_server::expression_value(mb, condition),
    }
}

/// The name of the breakpoint that stops on a change of the byte `index` of `watch`.
fn breakpoint_name(watch: &Watch, index: u16) -> String {
    format!("watch {} #{index}", watch.text.trim())
}

/// The REMU text of that breakpoint: a write to the byte, whose new value is not the old one.
fn breakpoint_text(watch: &Watch, address: u16, index: u16) -> String {
    format!(
        "MEM,W,STOP,addr={address},mask=65535,size=1,name=\"{}\",condition=VALUE != OLDVALUE",
        breakpoint_name(watch, index)
    )
}

pub(super) struct WatchWindow {
    watches: Vec<Watch>,
    entry: String,
    /// The field takes the keyboard when the window opens: a watch can be typed at once.
    focus_entry: bool,
    error: Option<String>,
    heat: super::heat::Heat,
}

impl Default for WatchWindow {
    fn default() -> Self {
        Self {
            watches: Vec::new(),
            entry: String::new(),
            focus_entry: true,
            error: None,
            heat: Default::default(),
        }
    }
}

impl WatchWindow {
    /// The breakpoints of `watch` that exist (one a byte), by their index in the debugger.
    fn breakpoints(
        target: &dyn CommandTarget,
        watch: &Watch,
        subject: &Subject,
    ) -> Vec<Option<usize>> {
        let Subject::Address(_) = subject else {
            return Vec::new();
        };
        (0..watch.size.bytes())
            .map(|i| {
                target
                    .debugger()
                    .and_then(|d| d.advanced_named(&breakpoint_name(watch, i)))
            })
            .collect()
    }

    fn stops(target: &dyn CommandTarget, watch: &Watch, subject: &Subject) -> bool {
        let found = Self::breakpoints(target, watch, subject);
        !found.is_empty() && found.iter().all(Option::is_some)
    }

    /// Stop (or no more) on a change of `watch`: its breakpoints added, or removed.
    fn set_stop(
        bus: &mut CommandBus,
        target: &dyn CommandTarget,
        watch: &Watch,
        subject: &Subject,
        on: bool,
    ) {
        let Subject::Address(address) = subject else {
            return;
        };
        if on {
            for i in 0..watch.size.bytes() {
                if Self::breakpoints(target, watch, subject)[usize::from(i)].is_none() {
                    push(
                        bus,
                        "breakpoint-add-advanced",
                        Some(Arg::Text(breakpoint_text(
                            watch,
                            address.wrapping_add(i),
                            i,
                        ))),
                    );
                }
            }
        } else {
            // from the last: the indices of the others stay
            let mut found: Vec<usize> = Self::breakpoints(target, watch, subject)
                .into_iter()
                .flatten()
                .collect();
            found.sort_unstable();
            for index in found.into_iter().rev() {
                push(
                    bus,
                    "breakpoint-remove",
                    Some(Arg::Text(format!("#{index}"))),
                );
            }
        }
    }

    fn add(&mut self, target: &dyn CommandTarget) {
        let symbols = target.debugger().map(|d| d.symbols());
        match parse(&self.entry, symbols) {
            Ok(_) => {
                self.watches.push(Watch {
                    text: self.entry.trim().to_owned(),
                    size: Size::Byte,
                });
                self.entry.clear();
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }
}

impl ViewWindow for WatchWindow {
    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        let (target, bus, _) = cx.parts();
        let mb = target.motherboard();
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.entry)
                    .hint_text("&BE00, a symbol, HL, peek(HL), deek(&BE00)+1")
                    .desired_width(240.0),
            );
            if std::mem::take(&mut self.focus_entry) {
                field.request_focus();
            }
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Watch").clicked() || enter {
                self.add(target);
                // the next one can be typed at once
                field.request_focus();
            }
        });
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        if self.watches.is_empty() {
            ui.weak("Nothing watched: add an address, a symbol or an expression.");
            return;
        }
        // each one's value now, lit when it changed lately
        let symbols = target.debugger().map(|d| d.symbols());
        let subjects: Vec<Result<Subject, String>> = self
            .watches
            .iter()
            .map(|w| parse(&w.text, symbols))
            .collect();
        let values: Vec<u32> = self
            .watches
            .iter()
            .zip(&subjects)
            .map(|(w, s)| s.as_ref().map_or(0, |s| value(s, w.size, mb) as u32))
            .collect();
        // another list, other values: none lit
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            self.watches
                .iter()
                .map(|w| (&w.text, w.size == Size::Word))
                .collect::<Vec<_>>()
                .hash(&mut hasher);
            hasher.finish()
        };
        super::heat::observe(ui, &mut self.heat, key, mb, &values);
        let mut remove = None;
        egui::Grid::new("watches").striped(true).show(ui, |ui| {
            for heading in ["Watch", "Value", "Size", "Stop on change", ""] {
                ui.weak(heading);
            }
            ui.end_row();
            for (index, watch) in self.watches.iter_mut().enumerate() {
                ui.monospace(&watch.text);
                let heat = self.heat.heat(index);
                let value = i64::from(values[index]);
                let shown = match (&subjects[index], watch.size) {
                    (Ok(Subject::Address(_)), Size::Byte) | (Ok(Subject::Expression(_)), _)
                        if value < 256 =>
                    {
                        format!("&{value:02X}  {value:>5}")
                    }
                    _ => format!("&{value:04X}  {value:>5}"),
                };
                ui.label(super::heat::text(
                    egui::RichText::new(shown).monospace(),
                    heat,
                ));
                match &subjects[index] {
                    Ok(Subject::Address(address)) => {
                        // the size of what is at the address
                        let mut word = watch.size == Size::Word;
                        if ui
                            .checkbox(&mut word, "word")
                            .on_hover_text(format!("The word at &{address:04X}, not the byte"))
                            .changed()
                        {
                            // its breakpoints go with the old size
                            let subject = Subject::Address(*address);
                            let stopped = Self::stops(target, watch, &subject);
                            Self::set_stop(bus, target, watch, &subject, false);
                            watch.size = if word { Size::Word } else { Size::Byte };
                            if stopped {
                                Self::set_stop(bus, target, watch, &subject, true);
                            }
                        }
                        let subject = Subject::Address(*address);
                        let mut stops = Self::stops(target, watch, &subject);
                        if ui
                            .add_enabled(
                                target.debugger().is_some(),
                                egui::Checkbox::new(&mut stops, "stop"),
                            )
                            .on_hover_text(
                                "Stop the machine when the value changes (a write breakpoint)",
                            )
                            .changed()
                        {
                            Self::set_stop(bus, target, watch, &subject, stops);
                        }
                    }
                    Ok(Subject::Expression(_)) => {
                        ui.weak("expression");
                        ui.weak("");
                    }
                    Err(e) => {
                        ui.colored_label(ui.visuals().error_fg_color, e);
                        ui.weak("");
                    }
                }
                if ui
                    .small_button("Remove")
                    .on_hover_text("Stop watching it")
                    .clicked()
                {
                    remove = Some(index);
                }
                ui.end_row();
            }
        });
        if let Some(index) = remove {
            let watch = self.watches.remove(index);
            if let Ok(subject) = &subjects[index] {
                // its breakpoints with it
                Self::set_stop(bus, target, &watch, subject, false);
            }
        }
    }

    fn save(&self) -> serde_json::Value {
        serde_json::to_value(&self.watches).unwrap_or_default()
    }

    fn load(&mut self, saved: &serde_json::Value) {
        if let Ok(watches) = serde_json::from_value::<Vec<Watch>>(saved.clone()) {
            self.watches = watches;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motherboard::Motherboard;

    /// A number is an address, a symbol is its address, the rest an expression.
    #[test]
    fn a_text_is_an_address_a_symbol_or_an_expression() {
        assert_eq!(parse("&BE00", None), Ok(Subject::Address(0xbe00)));
        assert_eq!(parse("49152", None), Ok(Subject::Address(49152)));
        assert!(matches!(parse("HL", None), Ok(Subject::Expression(_))));
        assert!(matches!(
            parse("peek(HL)+1", None),
            Ok(Subject::Expression(_))
        ));
        assert!(parse("", None).is_err());
        assert!(parse("1 +", None).is_err());
    }

    /// Bytes, words and expressions read the machine's memory and registers.
    #[test]
    fn a_watch_reads_the_machine() {
        let mut mb = Motherboard::default();
        mb.write_byte(0x8000, 0x34);
        mb.write_byte(0x8001, 0x12);
        assert_eq!(value(&Subject::Address(0x8000), Size::Byte, &mb), 0x34);
        assert_eq!(value(&Subject::Address(0x8000), Size::Word, &mb), 0x1234);
        let expression = parse("peek(&8000)+1", None).unwrap();
        assert_eq!(value(&expression, Size::Byte, &mb), 0x35);
        let pc = parse("PC", None).unwrap();
        assert_eq!(value(&pc, Size::Byte, &mb), i64::from(mb.pc()));
    }

    /// The breakpoint a watch makes parses, stops on a write, and holds only for a change.
    #[test]
    fn stop_on_change_is_a_write_breakpoint_on_a_different_value() {
        use crate::breakpoints::{AdvancedBreakpoint, BreakpointAccess, BreakpointKind};
        let watch = Watch {
            text: "&8000".to_owned(),
            size: Size::Byte,
        };
        let text = breakpoint_text(&watch, 0x8000, 0);
        let brk = AdvancedBreakpoint::parse_remu(&text).expect(&text);
        assert_eq!(brk.kind, BreakpointKind::Memory);
        assert_eq!(brk.access, BreakpointAccess::Write);
        assert_eq!(brk.address, 0x8000);
        assert_eq!(
            brk.name.as_deref(),
            Some(breakpoint_name(&watch, 0).as_str())
        );
        let condition = brk.condition.expect("a condition");
        // the byte is 5: writing 5 does not hold, writing 6 does
        struct Bus(i64, i64);
        impl crate::breakpoints::ConditionContext for Bus {
            fn variable(&self, name: &str) -> Option<i64> {
                match name {
                    "VALUE" => Some(self.1),
                    "OLDVALUE" => Some(self.0),
                    _ => None,
                }
            }
            fn peek(&self, _: u16) -> u8 {
                0
            }
        }
        assert!(!condition.holds(&Bus(5, 5)));
        assert!(condition.holds(&Bus(5, 6)));
    }
}
