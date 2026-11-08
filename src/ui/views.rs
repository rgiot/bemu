//! The debugger's windows (Debug > View): they show what the debug server's API shows (the
//! same data, see [`DebugServer::inspect`](crate::debug_server::DebugServer::inspect)), and
//! change things through the commands.

use serde_json::Value;

use crate::commands::{self, Arg, Command, CommandBus, CommandTarget, View};

/// What the windows remember between frames.
#[derive(Default)]
pub struct ViewState {
    memory_address: String,
    memory_poke: String,
    disassembly_address: String,
    symbol_filter: String,
    breakpoint_address: String,
    advanced: AdvancedForm,
    counter_name: String,
}

/// The fields of a memory / I/O breakpoint (`POST /api/breakpoints`).
struct AdvancedForm {
    kind: usize,
    access: usize,
    run: usize,
    address: String,
    mask: String,
    value: String,
    name: String,
}

impl Default for AdvancedForm {
    fn default() -> Self {
        Self {
            kind: 1,
            access: 1,
            run: 0,
            address: String::new(),
            mask: "&FFFF".into(),
            value: String::new(),
            name: String::new(),
        }
    }
}

const KINDS: [&str; 3] = ["exec", "mem", "io"];
const ACCESSES: [&str; 3] = ["r", "w", "rw"];
const RUNS: [&str; 2] = ["stop", "watch"];

fn push(bus: &mut CommandBus, id: &str, arg: Option<Arg>) {
    let command = match arg {
        Some(arg) => Command::with_arg(id, arg),
        None => Command::new(id),
    };
    if let Ok(command) = command {
        bus.push(command);
    }
}

/// A JSON value as the windows print it: numbers in hexadecimal.
fn text_of(value: &Value) -> String {
    match value {
        Value::Number(n) => match n.as_u64() {
            Some(n) if n < 0x100 => format!("&{n:02X}  {n}"),
            Some(n) if n < 0x10000 => format!("&{n:04X}  {n}"),
            _ => n.to_string(),
        },
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "-".into(),
        other => other.to_string(),
    }
}

/// An object as a grid of its fields; arrays and objects in it unfold.
fn json_grid(ui: &mut egui::Ui, id: &str, value: &Value) {
    match value {
        Value::Object(fields) => {
            egui::Grid::new(id)
                .striped(true)
                .num_columns(2)
                .show(ui, |ui| {
                    for (key, value) in fields {
                        match value {
                            Value::Object(_) => {
                                ui.label(key);
                                ui.vertical(|ui| json_grid(ui, &format!("{id}.{key}"), value));
                            }
                            Value::Array(items) if items.iter().all(|v| !v.is_object()) => {
                                ui.label(key);
                                let text: Vec<String> = items
                                    .iter()
                                    .map(|v| match v.as_u64() {
                                        Some(n) => format!("{n:02X}"),
                                        None => text_of(v),
                                    })
                                    .collect();
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(text.join(" ")).monospace(),
                                    )
                                    .wrap(),
                                );
                            }
                            Value::Array(items) => {
                                ui.label(key);
                                ui.vertical(|ui| {
                                    for (i, item) in items.iter().enumerate() {
                                        json_grid(ui, &format!("{id}.{key}.{i}"), item);
                                        ui.separator();
                                    }
                                });
                            }
                            _ => {
                                ui.label(key);
                                ui.monospace(text_of(value));
                            }
                        }
                        ui.end_row();
                    }
                });
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                json_grid(ui, &format!("{id}.{i}"), item);
            }
        }
        other => {
            ui.monospace(text_of(other));
        }
    }
}

fn number(text: &str) -> Option<u16> {
    Arg::Text(text.to_owned())
        .number()
        .ok()
        .and_then(|n| u16::try_from(n).ok())
}

/// Show the window of `view`.
pub fn show(
    view: View,
    open: &mut bool,
    ctx: &egui::Context,
    target: &dyn CommandTarget,
    bus: &mut CommandBus,
    state: &mut ViewState,
) {
    let title = match view {
        View::Z80 => "Z80",
        View::Stack => "Stack",
        View::Crtc => "CRTC",
        View::GateArray => "Gate Array",
        View::Psg => "PSG",
        View::Fdc => "FDC",
        View::MemoryMap => "Memory map",
        View::Memory => "Memory",
        View::Disassembly => "Disassembly",
        View::History => "Instruction history",
        View::Symbols => "Symbols",
        View::Beam => "Beam",
        View::Breakpoints => "Breakpoints",
        View::Counters => "Counters",
        View::Ppi => "PPI",
        View::Tape => "Tape",
        View::Asic => "ASIC (CPC+)",
        View::Preferences => "Preferences",
        View::Shortcuts => "Keyboard shortcuts",
        View::About => "About bemu",
        View::Palette => return,
    };
    egui::Window::new(title)
        .open(open)
        .default_width(360.0)
        .resizable(true)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .max_height(520.0)
                .show(ui, |ui| {
                    contents(view, ui, target, bus, state);
                });
        });
}

fn inspect(target: &dyn CommandTarget, what: &str) -> Value {
    target.debugger().map_or(Value::Null, |debugger| {
        debugger.inspect(what, target.motherboard())
    })
}

/// The run controls at the top of the debugger's windows.
fn controls(ui: &mut egui::Ui, target: &dyn CommandTarget, bus: &mut CommandBus) {
    ui.horizontal(|ui| {
        for (id, label) in [
            ("debug-run", "▶ Run"),
            ("debug-pause", "⏸ Pause"),
            ("debug-step-in", "Step in"),
            ("debug-step-over", "Step over"),
            ("debug-step-out", "Step out"),
            ("debug-step-frame", "Frame"),
        ] {
            let enabled = commands::state(id, target).enabled;
            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                push(bus, id, None);
            }
        }
    });
    ui.separator();
}

fn contents(
    view: View,
    ui: &mut egui::Ui,
    target: &dyn CommandTarget,
    bus: &mut CommandBus,
    state: &mut ViewState,
) {
    let mb = target.motherboard();
    match view {
        View::Z80 => {
            controls(ui, target, bus);
            let z80 = inspect(target, "z80");
            let flags = z80["F"].as_u64().unwrap_or(0) as u8;
            ui.monospace(format!(
                "F = {}",
                cpclib_dap::inspect::describe_flags(flags)
            ));
            json_grid(ui, "z80", &z80);
            ui.label("Set: Debug > Set register… (HL=&C000)");
        }
        View::Stack => {
            let sp = mb.sp();
            let symbols = target.debugger().map(|d| d.symbols());
            let name = |address: u16| {
                symbols
                    .and_then(|s| s.describe(address))
                    .unwrap_or_else(|| format!("&{address:04X}"))
            };
            // the calls the stack holds (cpclib-dap's walk: the return addresses after a CALL)
            let stack: Vec<u8> = (0..256u16)
                .map(|i| mb.read_byte(sp.wrapping_add(i)))
                .collect();
            let frames = cpclib_dap::callstack::walk(&stack, |address| Some(mb.read_byte(address)));
            ui.strong(format!("Calls (SP = &{sp:04X})"));
            if frames.is_empty() {
                ui.weak("no CALL found on the stack");
            }
            egui::Grid::new("calls").striped(true).show(ui, |ui| {
                for frame in &frames {
                    ui.monospace(name(frame.called));
                    ui.weak(format!(
                        "called at &{:04X}, returns to &{:04X}",
                        frame.call_site, frame.return_address
                    ));
                    ui.end_row();
                }
            });
            ui.separator();
            ui.strong("Words");
            egui::Grid::new("stack").striped(true).show(ui, |ui| {
                for i in 0..16u16 {
                    let at = sp.wrapping_add(i * 2);
                    let word = mb.read_word(at);
                    ui.monospace(format!("&{at:04X}"));
                    ui.monospace(format!("&{word:04X}"));
                    ui.weak(symbols.and_then(|s| s.describe(word)).unwrap_or_default());
                    ui.end_row();
                }
            });
        }
        View::Crtc => {
            let crtc = inspect(target, "crtc");
            ui.label(format!(
                "{} ({})",
                mb.crtc().chip_name(),
                mb.crtc().crtc_type() as u8
            ));
            if let Some(regs) = crtc["regs"].as_array() {
                egui::Grid::new("crtc-regs").striped(true).show(ui, |ui| {
                    for (r, value) in regs.iter().enumerate() {
                        ui.monospace(format!("R{r}"));
                        ui.monospace(text_of(value));
                        if r % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
            }
            ui.separator();
            super::chips::crtc(ui, mb);
            let registers = mb.crtc().registers();
            for warning in cpclib_dap::inspect::validate_crtc(&registers) {
                let text = format!("{}: {}", warning.registers.join(", "), warning.message);
                ui.colored_label(ui.visuals().warn_fg_color, text);
            }
            ui.separator();
            let mut counters = crtc.clone();
            if let Some(fields) = counters.as_object_mut() {
                fields.remove("regs");
            }
            json_grid(ui, "crtc", &counters);
        }
        View::GateArray => {
            super::chips::gate_array(ui, mb);
            ui.collapsing("Raw (as /api/ga)", |ui| {
                json_grid(ui, "ga", &inspect(target, "ga"))
            });
        }
        View::Psg => {
            super::chips::psg(ui, mb);
            ui.collapsing("Raw (as /api/psg)", |ui| {
                json_grid(ui, "psg", &inspect(target, "psg"))
            });
        }
        View::Ppi => super::chips::ppi(ui, mb),
        View::Tape => super::chips::tape(ui, mb, bus),
        View::Asic => super::chips::asic(ui, mb),
        View::Fdc => json_grid(ui, "fdc", &inspect(target, "fdc")),
        View::MemoryMap => json_grid(ui, "memmap", &inspect(target, "memmap")),
        View::Beam => {
            json_grid(ui, "beam", &inspect(target, "beam"));
            ui.separator();
            json_grid(ui, "state", &inspect(target, "state"));
        }
        View::Memory => {
            ui.horizontal(|ui| {
                ui.label("Address");
                ui.add(egui::TextEdit::singleline(&mut state.memory_address).desired_width(80.0));
            });
            let start = number(&state.memory_address).unwrap_or(0) & 0xfff0;
            let mut text = String::new();
            for row in 0..32u16 {
                let at = start.wrapping_add(row * 16);
                let bytes: Vec<u8> = (0..16).map(|i| mb.read_byte(at.wrapping_add(i))).collect();
                let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02X}")).collect();
                let ascii: String = bytes
                    .iter()
                    .map(|&b| {
                        if (0x20..0x7f).contains(&b) {
                            b as char
                        } else {
                            '.'
                        }
                    })
                    .collect();
                text.push_str(&format!("{at:04X}  {}  {ascii}\n", hex.join(" ")));
            }
            ui.add(egui::Label::new(egui::RichText::new(text).monospace()));
            ui.horizontal(|ui| {
                ui.label("Poke");
                ui.add(
                    egui::TextEdit::singleline(&mut state.memory_poke)
                        .hint_text("&4000 3E 01 C9")
                        .desired_width(200.0),
                );
                if ui.button("Write").clicked() {
                    push(
                        bus,
                        "memory-poke",
                        Some(Arg::Text(state.memory_poke.clone())),
                    );
                }
            });
        }
        View::Disassembly => {
            controls(ui, target, bus);
            ui.horizontal(|ui| {
                ui.label("From");
                ui.add(
                    egui::TextEdit::singleline(&mut state.disassembly_address)
                        .hint_text("PC")
                        .desired_width(80.0),
                );
            });
            let pc = mb.pc();
            let symbols = target.debugger().map(|d| d.symbols());
            let mut at = match state.disassembly_address.trim() {
                "" => pc,
                text => number(text)
                    .or_else(|| symbols.and_then(|s| s.resolve(text)))
                    .unwrap_or(pc),
            };
            // cpclib-dap's disassembly: the text and the cost in NOPs
            let bytes: Vec<u8> = (0..160u16)
                .map(|i| mb.read_byte(at.wrapping_add(i)))
                .collect();
            let instructions = cpclib_dap::disassemble::decode(at, &bytes, 32);
            egui::Grid::new("disassembly").striped(true).show(ui, |ui| {
                for instruction in &instructions {
                    at = instruction.address;
                    let marked = target
                        .debugger()
                        .is_some_and(|d| d.has_exec_breakpoint(at, mb));
                    if ui
                        .small_button(if marked { "●" } else { "○" })
                        .on_hover_text("Breakpoint here")
                        .clicked()
                    {
                        push(bus, "breakpoint-add", Some(Arg::Number(i64::from(at))));
                    }
                    let here = if at == pc { "▶" } else { " " };
                    match symbols.and_then(|s| s.name_at(at)) {
                        Some(name) => ui.monospace(format!("{here}{at:04X} {name}:")),
                        None => ui.monospace(format!("{here}{at:04X}")),
                    };
                    let hex: Vec<String> = instruction
                        .bytes
                        .iter()
                        .map(|b| format!("{b:02X}"))
                        .collect();
                    ui.weak(egui::RichText::new(hex.join(" ")).monospace());
                    ui.monospace(&instruction.text);
                    ui.weak(match instruction.cost {
                        Some(nops) => format!("{nops} NOP{}", if nops == 1 { "" } else { "s" }),
                        None => String::new(),
                    });
                    ui.end_row();
                }
            });
        }
        View::History => {
            let Some(debugger) = target.debugger() else {
                ui.label("No debugger");
                return;
            };
            ui.label(format!(
                "The last {} instructions (Debug > Set history size)",
                debugger.history_size()
            ));
            let mut text = String::new();
            for (pc, bytes) in debugger.history() {
                let name = debugger.symbols().describe(*pc).unwrap_or_default();
                let decoded = cpclib_dap::disassemble::decode(*pc, bytes, 1);
                let (code, cost) = decoded.first().map_or_else(
                    || (String::from("?"), String::new()),
                    |i| {
                        (
                            i.text.clone(),
                            i.cost.map_or(String::new(), |n| format!("{n} NOPs")),
                        )
                    },
                );
                text.push_str(&format!("{pc:04X}  {code:<20} {cost:<8} {name}\n"));
            }
            ui.add(egui::Label::new(egui::RichText::new(text).monospace()));
        }
        View::Symbols => {
            let Some(debugger) = target.debugger() else {
                ui.label("No debugger");
                return;
            };
            ui.horizontal(|ui| {
                ui.label("Filter");
                ui.text_edit_singleline(&mut state.symbol_filter);
                if ui.button("Load…").clicked() {
                    push(bus, "symbols-load", None);
                }
            });
            let filter = state.symbol_filter.to_lowercase();
            egui::Grid::new("symbols").striped(true).show(ui, |ui| {
                for (name, address) in debugger
                    .symbols()
                    .iter()
                    .filter(|(name, _)| name.to_lowercase().contains(&filter))
                    .take(500)
                {
                    ui.monospace(format!("&{address:04X}"));
                    ui.label(name);
                    ui.end_row();
                }
            });
        }
        View::Breakpoints => {
            controls(ui, target, bus);
            ui.horizontal(|ui| {
                ui.label("Execution");
                ui.add(
                    egui::TextEdit::singleline(&mut state.breakpoint_address)
                        .hint_text("address or symbol")
                        .desired_width(140.0),
                );
                if ui.button("Add").clicked() {
                    push(
                        bus,
                        "breakpoint-add",
                        Some(Arg::Text(state.breakpoint_address.clone())),
                    );
                }
            });
            ui.collapsing("Memory / I/O breakpoint", |ui| {
                let form = &mut state.advanced;
                egui::Grid::new("advanced").show(ui, |ui| {
                    ui.label("Kind");
                    egui::ComboBox::from_id_salt("kind").show_index(
                        ui,
                        &mut form.kind,
                        KINDS.len(),
                        |i| KINDS[i],
                    );
                    ui.end_row();
                    ui.label("Access");
                    egui::ComboBox::from_id_salt("access").show_index(
                        ui,
                        &mut form.access,
                        ACCESSES.len(),
                        |i| ACCESSES[i],
                    );
                    ui.end_row();
                    ui.label("On hit");
                    egui::ComboBox::from_id_salt("run").show_index(
                        ui,
                        &mut form.run,
                        RUNS.len(),
                        |i| RUNS[i],
                    );
                    ui.end_row();
                    for (label, field) in [
                        ("Address", &mut form.address),
                        ("Mask", &mut form.mask),
                        ("Value", &mut form.value),
                        ("Name", &mut form.name),
                    ] {
                        ui.label(label);
                        ui.text_edit_singleline(field);
                        ui.end_row();
                    }
                });
                if ui.button("Add").clicked() {
                    let mut body = serde_json::json!({
                        "kind": KINDS[form.kind],
                        "access": ACCESSES[form.access],
                        "run": RUNS[form.run],
                    });
                    if let Some(address) = number(&form.address) {
                        body["addr"] = address.into();
                    }
                    if let Some(mask) = number(&form.mask) {
                        body["mask"] = mask.into();
                    }
                    if let Some(value) = number(&form.value) {
                        body["value"] = value.into();
                    }
                    if !form.name.is_empty() {
                        body["name"] = form.name.clone().into();
                    }
                    push(
                        bus,
                        "breakpoint-add-advanced",
                        Some(Arg::Text(body.to_string())),
                    );
                }
            });
            ui.separator();
            json_grid(ui, "breakpoints", &inspect(target, "breakpoints"));
            json_grid(ui, "raster", &inspect(target, "raster_bp"));
            if ui
                .add_enabled(
                    commands::state("breakpoints-clear", target).enabled,
                    egui::Button::new("Remove all"),
                )
                .clicked()
            {
                push(bus, "breakpoints-clear", None);
            }
        }
        View::Counters => {
            let Some(debugger) = target.debugger() else {
                ui.label("No debugger");
                return;
            };
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut state.counter_name);
                if ui.button("New").clicked() && !state.counter_name.is_empty() {
                    push(
                        bus,
                        "counter-new",
                        Some(Arg::Text(state.counter_name.clone())),
                    );
                }
            });
            egui::Grid::new("counters").striped(true).show(ui, |ui| {
                for (name, nops, running) in debugger.counters() {
                    ui.label(&name);
                    ui.monospace(format!("{nops} NOPs"));
                    if ui.button(if running { "Stop" } else { "Start" }).clicked() {
                        push(bus, "counter-toggle", Some(Arg::Text(name.clone())));
                    }
                    if ui.button("Reset").clicked() {
                        push(bus, "counter-reset", Some(Arg::Text(name.clone())));
                    }
                    if ui.button("Delete").clicked() {
                        push(bus, "counter-delete", Some(Arg::Text(name)));
                    }
                    ui.end_row();
                }
            });
        }
        View::Preferences => preferences(ui, target, bus),
        View::Shortcuts => {
            egui::Grid::new("shortcuts").striped(true).show(ui, |ui| {
                for spec in commands::all().iter().filter(|s| s.shortcut.is_some()) {
                    ui.monospace(spec.shortcut.unwrap_or_default());
                    ui.label(format!("{} ({})", spec.label(), spec.menus().join(" › ")));
                    ui.end_row();
                }
                ui.monospace("F1-F3");
                ui.label("Keypad 1-3, for the hosts without a keypad");
                ui.end_row();
            });
        }
        View::About => {
            ui.heading(format!("bemu {}", env!("CARGO_PKG_VERSION")));
            ui.label("An Amstrad CPC, CPC+ and GX4000 emulator.");
            ui.label(env!("CARGO_PKG_AUTHORS"));
            ui.label("Licence: MIT or Apache-2.0");
        }
        View::Palette => {}
    }
}

fn preferences(ui: &mut egui::Ui, target: &dyn CommandTarget, bus: &mut CommandBus) {
    let Some(settings) = target.settings() else {
        ui.label("No preferences here");
        return;
    };
    egui::Grid::new("preferences")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Rendering");
            ui.horizontal(|ui| {
                for (id, label) in [
                    ("video-rendering-raw", "Raw"),
                    ("video-rendering-ctm", "CTM"),
                    ("video-rendering-gt65", "GT65"),
                    ("video-rendering-trinitron", "Trinitron"),
                    ("video-rendering-atari", "Atari"),
                ] {
                    if ui
                        .radio(commands::state(id, target).checked, label)
                        .clicked()
                    {
                        push(bus, id, None);
                    }
                }
            });
            ui.end_row();
            ui.label("Curvature");
            let mut curvature = settings.curvature;
            if ui
                .add(egui::Slider::new(&mut curvature, 0.0..=4.0))
                .changed()
            {
                push(
                    bus,
                    "video-curvature-set",
                    Some(Arg::Text(curvature.to_string())),
                );
            }
            ui.end_row();
            ui.label("Black level");
            let mut level = settings.black_level;
            if ui.add(egui::Slider::new(&mut level, 0..=255)).changed() {
                push(
                    bus,
                    "video-black-level-set",
                    Some(Arg::Number(i64::from(level))),
                );
            }
            ui.end_row();
            ui.label("Volume");
            let mut volume = settings.volume;
            if ui
                .add(egui::Slider::new(&mut volume, 0..=100).suffix(" %"))
                .changed()
            {
                push(
                    bus,
                    "audio-volume-set",
                    Some(Arg::Number(i64::from(volume))),
                );
            }
            ui.end_row();
            ui.label("Keyboard");
            ui.horizontal(|ui| {
                for (id, label) in [
                    ("keyboard-mapping-symbolic", "Symbolic"),
                    ("keyboard-mapping-positional", "Positional"),
                ] {
                    if ui
                        .radio(commands::state(id, target).checked, label)
                        .clicked()
                    {
                        push(bus, id, None);
                    }
                }
            });
            ui.end_row();
            for (id, label) in [
                ("ui-menu-toggle", "Menu bar"),
                ("audio-mute-toggle", "Mute"),
                ("discs-save-automatically-toggle", "Write discs back"),
                ("overlay-toggle", "CRTC / GA overlay"),
            ] {
                let state = commands::state(id, target);
                let mut checked = state.checked;
                ui.label("");
                if ui
                    .add_enabled(state.enabled, egui::Checkbox::new(&mut checked, label))
                    .clicked()
                {
                    push(bus, id, None);
                }
                ui.end_row();
            }
        });
}
