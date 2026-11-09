//! The debugger's windows (Debug > View): they show what the debug server's API shows (the
//! same data, see [`DebugServer::inspect`](crate::debug_server::DebugServer::inspect)), and
//! change things through the commands.

use serde_json::Value;

use crate::commands::{self, Arg, Command, CommandBus, CommandTarget, View};

/// What the windows remember between frames.
#[derive(Default)]
pub struct ViewState {
    manager: super::breakpoints::Manager,
    screen: super::screen::ScreenState,
    /// The bytes the memory window showed last (its first address and them), to mark the
    /// changed ones.
    memory_seen: Option<(u16, Vec<u8>)>,
    memory_address: String,
    /// The memory window follows the registers: a few rows around each pointer.
    memory_follow: bool,
    memory_poke: String,
    disassembly_address: String,
    symbol_filter: String,
    counter_name: String,
}

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
    let title = title(view);
    if title.is_empty() {
        return;
    }
    // as large as its content, a long one scrolled
    egui::Window::new(title)
        .open(open)
        .resizable(false)
        .show(ctx, |ui| {
            egui::ScrollArea::both()
                .auto_shrink([true, true])
                .max_height(560.0)
                .max_width(760.0)
                .show(ui, |ui| {
                    contents(view, ui, target, bus, state);
                });
        });
}

/// The debugger's windows (the ones a desktop opens as windows of the system), not the
/// preferences, the shortcuts or the about box.
pub fn is_debugger(view: View) -> bool {
    !matches!(
        view,
        View::Preferences | View::Shortcuts | View::About | View::Palette
    )
}

/// The window's title ("" for the palette, which is no window).
pub fn title(view: View) -> &'static str {
    match view {
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
        View::Screen => "Screen memory",
        View::Preferences => "Preferences",
        View::Shortcuts => "Keyboard shortcuts",
        View::About => "About bemu",
        View::Palette => "",
    }
}

/// The breakpoint column of the disassembly: a red dot, or a ring to click (drawn: the fonts
/// have no such glyphs).
fn breakpoint_dot(ui: &mut egui::Ui, marked: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::click());
    let center = rect.center();
    let painter = ui.painter();
    if marked {
        painter.circle_filled(center, 5.0, egui::Color32::from_rgb(220, 50, 50));
    } else {
        let color = if response.hovered() {
            egui::Color32::from_rgb(220, 50, 50)
        } else {
            ui.visuals().weak_text_color()
        };
        painter.circle_stroke(center, 4.5, egui::Stroke::new(1.0_f32, color));
    }
    response
}

/// An address to click: the disassembly from it (a right click: the memory window there, or a
/// breakpoint on it).
fn address_link(
    ui: &mut egui::Ui,
    text: String,
    address: u16,
    state: &mut ViewState,
    bus: &mut CommandBus,
) {
    let response = ui
        .add(egui::Link::new(egui::RichText::new(text).monospace()))
        .on_hover_text(format!("&{address:04X}: disassemble (right click: more)"));
    if response.clicked() {
        state.disassembly_address = format!("&{address:04X}");
        push(bus, "view-disassembly", None);
    }
    response.context_menu(|ui| {
        if ui.button("Disassemble from here").clicked() {
            state.disassembly_address = format!("&{address:04X}");
            push(bus, "view-disassembly", None);
            ui.close();
        }
        if ui.button("Show in the memory window").clicked() {
            state.memory_address = format!("&{address:04X}");
            push(bus, "view-memory", None);
            ui.close();
        }
        if ui.button("Toggle a breakpoint").clicked() {
            push(
                bus,
                "breakpoint-toggle",
                Some(Arg::Number(i64::from(address))),
            );
            ui.close();
        }
        if ui.button("Run to here").clicked() {
            push(
                bus,
                "debug-run-to-address",
                Some(Arg::Number(i64::from(address))),
            );
            ui.close();
        }
    });
}

/// The CRTC's state as Caprice Forever's CRTC window shows it: the beam's position, VBL and
/// HBL, each counter against the register it counts to, the address it outputs (a button to see
/// it in the memory window), the Gate Array's interrupt counter, the sync counters.
fn crtc_status(
    ui: &mut egui::Ui,
    mb: &crate::motherboard::Motherboard,
    beam: &Value,
    state: &mut ViewState,
    bus: &mut CommandBus,
) {
    let crtc = mb.crtc();
    let r = crtc.registers();
    let c = crtc.counters();
    let ma = crtc.vma();
    // the byte the CRTC addresses: its page, its offset, the raster line's 2K block
    let address = ((ma >> 12) & 3) * 0x4000 + ((ma & 0x3ff) << 1) + u16::from(c.vlc & 7) * 0x800;
    let pair = |count: u8, register: u8| format!("&{count:02X} / &{register:02X}");
    egui::Grid::new("crtc-status")
        .striped(true)
        .spacing([10.0, 3.0])
        .show(ui, |ui| {
            ui.label("Beam");
            match (beam["x"].as_u64(), beam["line"].as_u64()) {
                (Some(x), Some(line)) => {
                    ui.monospace(format!("x {x} (NOP {}), line {line}", x / 16))
                }
                _ => ui.weak("no monitor"),
            };
            ui.end_row();
            ui.label("Syncs");
            ui.horizontal(|ui| {
                let mut vbl = c.vsync;
                let mut hbl = c.hsync;
                ui.add_enabled(false, egui::Checkbox::new(&mut vbl, "VBL"));
                ui.add_enabled(false, egui::Checkbox::new(&mut hbl, "HBL"));
                if c.in_vertical_adjust {
                    ui.weak("adjust");
                }
            });
            ui.end_row();
            ui.label("Char (C0 / R0)");
            ui.monospace(pair(c.hcc, r[0]));
            ui.end_row();
            ui.label("Line (C4 / R4)");
            ui.monospace(pair(c.vcc, r[4]));
            ui.end_row();
            ui.label("Raster (C9 / R9)");
            ui.monospace(pair(c.vlc, r[9]));
            ui.end_row();
            ui.label("Memory (VMA)");
            ui.horizontal(|ui| {
                ui.monospace(format!("&{address:04X}"))
                    .on_hover_text(format!("MA &{ma:04X}, RA {}", c.vlc & 7));
                if ui
                    .small_button("›MEM")
                    .on_hover_text("Show it in the memory window")
                    .clicked()
                {
                    state.memory_address = format!("&{address:04X}");
                    push(bus, "view-memory", None);
                }
            });
            ui.end_row();
            let r52 = mb.ga().r52();
            ui.label("IT (R52)");
            ui.monospace(format!("{r52} (&{r52:02X})"));
            ui.end_row();
            ui.label("HSync (C3l)");
            ui.monospace(format!("{} (&{:02X})", c.hsc, c.hsc));
            ui.end_row();
            ui.label("VSync (C3h)");
            ui.monospace(format!("{} (&{:02X})", c.vsc, c.vsc));
            ui.end_row();
            ui.label("VTA");
            ui.monospace(format!("{} (&{:02X})", c.vtac, c.vtac));
            ui.end_row();
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

pub fn contents(
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
            super::chips::registers(ui, mb, bus);
            ui.weak(format!(
                "{}: IM {}, IFF1 {}, IFF2 {}",
                cpclib_dap::inspect::describe_flags(flags),
                z80["IM"],
                z80["IFF1"],
                z80["IFF2"]
            ));
            ui.separator();
            super::chips::z80(ui, mb);
            ui.collapsing("Raw (as /api/z80)", |ui| json_grid(ui, "z80", &z80));
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
                    address_link(ui, name(frame.called), frame.called, state, bus);
                    ui.horizontal(|ui| {
                        ui.weak("called at");
                        address_link(
                            ui,
                            format!("&{:04X}", frame.call_site),
                            frame.call_site,
                            state,
                            bus,
                        );
                        ui.weak("returns to");
                        address_link(
                            ui,
                            format!("&{:04X}", frame.return_address),
                            frame.return_address,
                            state,
                            bus,
                        );
                    });
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
                    address_link(ui, format!("&{word:04X}"), word, state, bus);
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
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.strong("Registers");
                    super::chips::crtc_registers(ui, mb);
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.strong("Status");
                    crtc_status(ui, mb, &inspect(target, "beam"), state, bus);
                });
            });
            ui.separator();
            super::chips::crtc(ui, mb);
            ui.collapsing("Raw (as /api/crtc)", |ui| json_grid(ui, "crtc", &crtc));
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
        View::Screen => super::screen::show(ui, mb, &mut state.screen),
        View::Fdc => {
            super::chips::fdc(ui, mb);
            ui.collapsing("Raw (as /api/fdc)", |ui| {
                json_grid(ui, "fdc", &inspect(target, "fdc"))
            });
        }
        View::MemoryMap => {
            super::chips::memory_map(ui, mb);
            ui.collapsing("Raw (as /api/memmap)", |ui| {
                json_grid(ui, "memmap", &inspect(target, "memmap"))
            });
        }
        View::Beam => {
            json_grid(ui, "beam", &inspect(target, "beam"));
            ui.separator();
            json_grid(ui, "state", &inspect(target, "state"));
        }
        View::Memory => {
            ui.horizontal(|ui| {
                ui.label("Address");
                ui.add(
                    egui::TextEdit::singleline(&mut state.memory_address)
                        .hint_text("&C000 or a symbol")
                        .desired_width(110.0),
                );
                for (name, value) in [
                    ("PC", mb.pc()),
                    ("SP", mb.sp()),
                    ("HL", mb.hl()),
                    ("IX", mb.ix()),
                ] {
                    if ui
                        .small_button(name)
                        .on_hover_text(format!("&{value:04X}"))
                        .clicked()
                    {
                        state.memory_address = format!("&{value:04X}");
                    }
                }
                ui.checkbox(&mut state.memory_follow, "Follow the registers")
                    .on_hover_text("PC, SP, HL, DE, BC, IX and IY: the bytes around each");
            });
            let symbols = target.debugger().map(|d| d.symbols());
            if state.memory_follow {
                super::chips::follow_registers(ui, mb, |address| {
                    symbols.and_then(|s| s.describe(address))
                });
                return;
            }
            let start = number(&state.memory_address)
                .or_else(|| symbols.and_then(|s| s.resolve(&state.memory_address)))
                .unwrap_or(0)
                & 0xfff0;
            const ROWS: u16 = 32;
            let bytes: Vec<u8> = (0..ROWS * 16)
                .map(|i| mb.read_byte(start.wrapping_add(i)))
                .collect();
            // the bytes that changed since the last frame shown, highlighted
            let seen = state
                .memory_seen
                .as_ref()
                .filter(|(at, _)| *at == start)
                .map(|(_, bytes)| bytes.clone());
            let changed = ui.visuals().warn_fg_color;
            egui::Grid::new("memory")
                .spacing([6.0, 0.0])
                .min_col_width(0.0)
                .show(ui, |ui| {
                    for row in 0..ROWS {
                        let at = start.wrapping_add(row * 16);
                        ui.monospace(format!("{at:04X}"));
                        let mut ascii = String::new();
                        for column in 0..16u16 {
                            let index = usize::from(row * 16 + column);
                            let byte = bytes[index];
                            let mut text = egui::RichText::new(format!("{byte:02X}")).monospace();
                            if seen.as_ref().is_some_and(|seen| seen[index] != byte) {
                                text = text.color(changed);
                            }
                            let address = at.wrapping_add(column);
                            // a byte a label starts at: underlined, the label in its tooltip
                            let label = symbols.and_then(|s| s.name_at(address));
                            if label.is_some() {
                                text = text.underline().strong();
                            }
                            let tooltip = match label {
                                Some(label) => {
                                    format!("{label}: &{address:04X}: {byte} ({byte:#010b})")
                                }
                                None => format!("&{address:04X}: {byte} ({byte:#010b})"),
                            };
                            // a click: that address in the poke field
                            if ui
                                .add(egui::Label::new(text).sense(egui::Sense::click()))
                                .on_hover_text(tooltip)
                                .clicked()
                            {
                                state.memory_poke = format!("&{address:04X} ");
                            }
                            ascii.push(if (0x20..0x7f).contains(&byte) {
                                byte as char
                            } else {
                                '.'
                            });
                        }
                        ui.monospace(ascii);
                        ui.end_row();
                    }
                });
            state.memory_seen = Some((start, bytes));
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
                if ui
                    .add_enabled(
                        !state.disassembly_address.is_empty(),
                        egui::Button::new("Follow the PC"),
                    )
                    .clicked()
                {
                    state.disassembly_address.clear();
                }
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
            // the jumps and calls, drawn in a gutter left of the instructions
            const GUTTER: f32 = 44.0;
            let rows: Vec<(u16, &[u8])> = instructions
                .iter()
                .map(|i| (i.address, i.bytes.as_slice()))
                .collect();
            let arcs = super::chips::arcs(&rows, 6);
            let mut rows_y = Vec::with_capacity(instructions.len());
            let mut gutter = egui::Rect::NOTHING;
            let name_at =
                |address: u16| symbols.and_then(|s| s.name_at(address)).map(str::to_owned);
            egui::Grid::new("disassembly").striped(true).show(ui, |ui| {
                for instruction in &instructions {
                    at = instruction.address;
                    let (cell, _) =
                        ui.allocate_exact_size(egui::vec2(GUTTER, 14.0), egui::Sense::hover());
                    rows_y.push(cell.center().y);
                    gutter = gutter.union(cell);
                    let marked = target
                        .debugger()
                        .is_some_and(|d| d.has_exec_breakpoint(at, mb));
                    if breakpoint_dot(ui, marked)
                        .on_hover_text(if marked {
                            "Remove the breakpoint"
                        } else {
                            "Breakpoint here"
                        })
                        .clicked()
                    {
                        push(bus, "breakpoint-toggle", Some(Arg::Number(i64::from(at))));
                    }
                    let here = if at == pc { "▶" } else { " " };
                    let label = match symbols.and_then(|s| s.name_at(at)) {
                        Some(name) => format!("{here}{at:04X} {name}:"),
                        None => format!("{here}{at:04X}"),
                    };
                    // what can be done here: a right click
                    ui.add(
                        egui::Label::new(egui::RichText::new(label).monospace())
                            .sense(egui::Sense::click()),
                    )
                    .on_hover_text("Right click: run to here, set the PC, breakpoint, memory")
                    .context_menu(|ui| {
                        if ui.button("Run to here").clicked() {
                            push(
                                bus,
                                "debug-run-to-address",
                                Some(Arg::Number(i64::from(at))),
                            );
                            ui.close();
                        }
                        if ui.button("Set the PC here").clicked() {
                            push(bus, "debug-set-pc", Some(Arg::Number(i64::from(at))));
                            ui.close();
                        }
                        if ui
                            .button(if marked {
                                "Remove the breakpoint"
                            } else {
                                "Add a breakpoint"
                            })
                            .clicked()
                        {
                            push(bus, "breakpoint-toggle", Some(Arg::Number(i64::from(at))));
                            ui.close();
                        }
                        if ui.button("Disassemble from here").clicked() {
                            state.disassembly_address = format!("&{at:04X}");
                            ui.close();
                        }
                        if ui.button("Show in the memory window").clicked() {
                            state.memory_address = format!("&{at:04X}");
                            push(bus, "view-memory", None);
                            ui.close();
                        }
                    });
                    let hex: Vec<String> = instruction
                        .bytes
                        .iter()
                        .map(|b| format!("{b:02X}"))
                        .collect();
                    ui.weak(egui::RichText::new(hex.join(" ")).monospace());
                    // where it goes (its symbol), and at the PC whether the flags take it
                    let f = crate::z80::HasValue::value(mb.cpu().reg8(crate::z80::Reg8::F));
                    // the addresses with a label: the label
                    let mut text = super::chips::with_labels(
                        &instruction.text,
                        at,
                        &instruction.bytes,
                        name_at,
                    );
                    if let Some((_, taken)) = super::chips::branch(at, &instruction.bytes, f) {
                        if let (true, Some(taken)) = (at == pc, taken) {
                            text.push_str(if taken { "  (taken)" } else { "  (not taken)" });
                        }
                    }
                    ui.monospace(text);
                    ui.weak(match instruction.cost {
                        Some(nops) => format!("{nops} NOP{}", if nops == 1 { "" } else { "s" }),
                        None => String::new(),
                    });
                    ui.end_row();
                }
            });
            if !rows_y.is_empty() {
                let pc_row = instructions.iter().position(|i| i.address == pc);
                super::chips::paint_arcs(ui.painter(), gutter, &rows_y, &arcs, pc_row);
            }
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
                    address_link(ui, format!("&{address:04X}"), address, state, bus);
                    ui.label(name);
                    ui.end_row();
                }
            });
        }
        View::Breakpoints => {
            controls(ui, target, bus);
            // the manager (cpclib's VS Code one): its tabs, presets and form
            let symbols = target.debugger().map(|d| d.symbols());
            let resolve = |name: &str| symbols.and_then(|s| s.resolve(name));
            super::breakpoints::show(ui, &mut state.manager, bus, &resolve);
            ui.separator();
            if let Some(debugger) = target.debugger() {
                let symbols = debugger.symbols();
                ui.strong("Execution");
                let mapped = debugger.exec_breakpoints_mapped(mb);
                egui::Grid::new("exec-bp").striped(true).show(ui, |ui| {
                    for address in &mapped {
                        ui.monospace(format!("&{address:04X}"));
                        ui.weak(symbols.describe(*address).unwrap_or_default());
                        if ui.small_button("Remove").clicked() {
                            push(
                                bus,
                                "breakpoint-remove",
                                Some(Arg::Number(i64::from(*address))),
                            );
                        }
                        ui.end_row();
                    }
                });
                let hidden = debugger.exec_breakpoint_count() - mapped.len();
                if hidden > 0 {
                    ui.weak(format!("{hidden} more in banks or ROMs not paged in now"));
                }
                if debugger.exec_breakpoint_count() == 0 {
                    ui.weak("none: click the disassembly's dot column, or Add above");
                }
                let advanced = inspect(target, "breakpoints");
                if let Some(list) = advanced["advanced"].as_array().filter(|l| !l.is_empty()) {
                    ui.strong("Memory / I/O");
                    egui::Grid::new("advanced-bp").striped(true).show(ui, |ui| {
                        for (index, breakpoint) in list.iter().enumerate() {
                            ui.monospace(format!("#{index}"));
                            ui.monospace(super::breakpoints::summary(breakpoint))
                                .on_hover_text(breakpoint.to_string());
                            if ui.small_button("Remove").clicked() {
                                push(
                                    bus,
                                    "breakpoint-remove",
                                    Some(Arg::Text(format!("#{index}"))),
                                );
                            }
                            ui.end_row();
                        }
                    });
                }
            }
            ui.collapsing("Raw (as /api/breakpoints, /api/raster_bp)", |ui| {
                json_grid(ui, "breakpoints", &inspect(target, "breakpoints"));
                json_grid(ui, "raster", &inspect(target, "raster_bp"));
            });
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
