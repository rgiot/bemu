//! The Memory and Disassembly windows: several of each can be open, each with its own address,
//! paging and options (a [`MemoryWindow`], a [`DisassemblyWindow`]). What one window asks of
//! another (show this address in the memory, disassemble from there, in a new window or not) is
//! a [`Link`] the interface follows after the frame.

use crate::commands::{Arg, CommandBus, CommandTarget, View};

use super::views::{breakpoint_dot, controls, number, push};

/// A window asked to show `address` in a window of `view`: a new one (`new`), or the first.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Link {
    pub view: View,
    pub address: u16,
    pub new: bool,
}

/// A Memory window's own settings.
#[derive(Default)]
pub struct MemoryWindow {
    /// The bytes it showed last (its first address and them), to mark the changed ones.
    seen: Option<(u16, Vec<u8>)>,
    pub address: String,
    paging: super::paging::Paging,
    /// The byte last clicked.
    selected: Option<u16>,
    /// A few rows around each register that points.
    follow: bool,
    poke: String,
}

/// A Disassembly window's own settings.
pub struct DisassemblyWindow {
    paging: super::paging::Paging,
    /// Where it starts: empty, the PC (it follows it).
    pub address: String,
    /// Its remembered addresses.
    bookmarks: Vec<u16>,
    /// The numbers, not the labels.
    no_labels: bool,
    /// Instructions shown before the start, and from it.
    before: usize,
    rows: usize,
}

impl MemoryWindow {
    /// Show `address` (a text: a number or a symbol), not the registers.
    pub fn go_to(&mut self, address: String) {
        self.address = address;
        self.follow = false;
    }
}

impl DisassemblyWindow {
    /// Disassemble from `address` (a text: a number or a symbol).
    pub fn go_to(&mut self, address: String) {
        self.address = address;
        self.before = 0;
    }
}

impl Default for DisassemblyWindow {
    fn default() -> Self {
        Self {
            paging: super::paging::Paging::default(),
            address: String::new(),
            bookmarks: Vec::new(),
            no_labels: false,
            before: 0,
            rows: 32,
        }
    }
}

/// Bytes a Z80 instruction takes at most (a prefix, an opcode, a displacement, an operand).
const LONGEST: u16 = 4;

/// Where to decode from to show `count` instructions before `anchor`, `anchor` itself starting
/// an instruction: the farthest start (more bytes to fall into step) whose instructions lead
/// exactly to it. `read` gives a byte. `None` when no start leads there (data, not code).
pub fn start_before(anchor: u16, count: usize, read: impl Fn(u16) -> u8) -> Option<u16> {
    if count == 0 {
        return Some(anchor);
    }
    let reach = (count as u16 + 4) * LONGEST;
    for back in (1..=reach).rev() {
        let start = anchor.wrapping_sub(back);
        let bytes: Vec<u8> = (0..back + LONGEST)
            .map(|i| read(start.wrapping_add(i)))
            .collect();
        let decoded = cpclib_dap::disassemble::decode(start, &bytes, usize::from(back) + 1);
        // the instructions up to the anchor: they must land on it
        let addresses: Vec<u16> = decoded
            .iter()
            .map(|i| i.address)
            .take_while(|a| a.wrapping_sub(start) < back)
            .collect();
        let next = decoded.get(addresses.len()).map(|i| i.address);
        if next == Some(anchor) && addresses.len() >= count {
            return Some(addresses[addresses.len() - count]);
        }
    }
    None
}

/// A Memory window: ACE's form (the paging looked through, where to go), the bytes (the
/// changed ones highlighted, as their characters), the byte clicked, a poke.
pub fn memory(
    ui: &mut egui::Ui,
    target: &dyn CommandTarget,
    bus: &mut CommandBus,
    window: &mut MemoryWindow,
    links: &mut Vec<Link>,
    id: &str,
) {
    let mb = target.motherboard();
    super::paging::show(ui, id, mb, &mut window.paging);
    super::widgets::rule(ui);
    ui.horizontal(|ui| {
        ui.label("Go to");
        ui.add(
            egui::TextEdit::singleline(&mut window.address)
                .hint_text("&C000 or a symbol")
                .desired_width(110.0),
        );
        ui.checkbox(&mut window.follow, "Follow the registers")
            .on_hover_text("PC, SP, HL, DE, BC, IX and IY: the bytes around each");
        if ui
            .button("Another window")
            .on_hover_text("A memory window of its own (another address, another paging)")
            .clicked()
        {
            let address = number(&window.address).unwrap_or(0);
            links.push(Link {
                view: View::Memory,
                address,
                new: true,
            });
        }
    });
    ui.horizontal(|ui| {
        ui.label("Pointed by");
        if let Some(address) = super::widgets::goto_buttons(ui, mb) {
            window.address = format!("&{address:04X}");
            window.follow = false;
        }
    });
    let symbols = target.debugger().map(|d| d.symbols());
    let paging = window.paging.clone();
    if window.follow {
        super::chips::follow_registers(
            ui,
            mb,
            |address| symbols.and_then(|s| s.describe(address)),
            |address| paging.read(mb, address),
        );
        return;
    }
    let start = number(&window.address)
        .or_else(|| symbols.and_then(|s| s.resolve(&window.address)))
        .unwrap_or(0)
        & 0xfff0;
    const ROWS: u16 = 32;
    let bytes: Vec<u8> = (0..ROWS * 16)
        .map(|i| paging.read(mb, start.wrapping_add(i)))
        .collect();
    // the bytes that changed since the last frame shown, highlighted (and their characters)
    let seen = window
        .seen
        .as_ref()
        .filter(|(at, _)| *at == start)
        .map(|(_, bytes)| bytes.clone());
    let changed = ui.visuals().warn_fg_color;
    let normal = ui.visuals().text_color();
    egui::Grid::new((id, "bytes"))
        .spacing([6.0, 0.0])
        .min_col_width(0.0)
        .show(ui, |ui| {
            for row in 0..ROWS {
                let at = start.wrapping_add(row * 16);
                ui.monospace(format!("{at:04X}"));
                let mut ascii = egui::text::LayoutJob::default();
                for column in 0..16u16 {
                    let index = usize::from(row * 16 + column);
                    let byte = bytes[index];
                    let is_changed = seen.as_ref().is_some_and(|seen| seen[index] != byte);
                    let mut text = egui::RichText::new(format!("{byte:02X}")).monospace();
                    if is_changed {
                        text = text.color(changed);
                    }
                    let address = at.wrapping_add(column);
                    // a byte a label starts at: underlined, the label in its tooltip
                    let label = symbols.and_then(|s| s.name_at(address));
                    if label.is_some() {
                        text = text.underline().strong();
                    }
                    let tooltip = match label {
                        Some(label) => format!("{label}: &{address:04X}: {byte} ({byte:#010b})"),
                        None => format!("&{address:04X}: {byte} ({byte:#010b})"),
                    };
                    // a click: that address in the poke field
                    if ui
                        .add(egui::Label::new(text).sense(egui::Sense::click()))
                        .on_hover_text(tooltip)
                        .clicked()
                    {
                        window.poke = format!("&{address:04X} ");
                        window.selected = Some(address);
                    }
                    let character = if (0x20..0x7f).contains(&byte) {
                        byte as char
                    } else {
                        '.'
                    };
                    ascii.append(
                        &character.to_string(),
                        0.0,
                        egui::TextFormat {
                            font_id: egui::FontId::monospace(12.0),
                            color: if is_changed { changed } else { normal },
                            ..Default::default()
                        },
                    );
                }
                ui.label(ascii);
                ui.end_row();
            }
        });
    window.seen = Some((start, bytes));
    // the byte clicked: its address, and the disassembly from it
    if let Some(address) = window.selected {
        ui.horizontal(|ui| {
            ui.label(format!("Address &{address:04X} ({address})"));
            if ui.button("» Disassemble…").clicked() {
                links.push(Link {
                    view: View::Disassembly,
                    address,
                    new: false,
                });
            }
        });
    }
    ui.horizontal(|ui| {
        ui.label("Poke");
        ui.add(
            egui::TextEdit::singleline(&mut window.poke)
                .hint_text("&4000 3E 01 C9")
                .desired_width(200.0),
        );
        if ui.button("Write").clicked() {
            push(bus, "memory-poke", Some(Arg::Text(window.poke.clone())));
        }
    });
}

/// A Disassembly window: the controls, ACE's form (the paging, where to start, the remembered
/// addresses), the instructions with their labels, arcs, costs and breakpoints; more before
/// (the start keeps its instructions) or after on demand.
pub fn disassembly(
    ui: &mut egui::Ui,
    target: &dyn CommandTarget,
    bus: &mut CommandBus,
    window: &mut DisassemblyWindow,
    links: &mut Vec<Link>,
    id: &str,
) {
    let mb = target.motherboard();
    controls(ui, target, bus);
    super::paging::show(ui, id, mb, &mut window.paging);
    super::widgets::rule(ui);
    let pc = mb.pc();
    ui.horizontal(|ui| {
        ui.label("From");
        ui.add(
            egui::TextEdit::singleline(&mut window.address)
                .hint_text("PC")
                .desired_width(90.0),
        );
        ui.label("Set to");
        for (name, value) in [
            ("PC", pc),
            ("HL", mb.hl()),
            ("IX", mb.ix()),
            ("IY", mb.iy()),
        ] {
            if ui
                .small_button(name)
                .on_hover_text(format!("&{value:04X}"))
                .clicked()
            {
                window.address = format!("&{value:04X}");
                window.before = 0;
            }
        }
        let mut follow = window.address.is_empty();
        if ui.checkbox(&mut follow, "Follow PC").changed() {
            window.address = if follow {
                String::new()
            } else {
                format!("&{pc:04X}")
            };
        }
        ui.checkbox(&mut window.no_labels, "Numbers, no labels");
    });
    let symbols = target
        .debugger()
        .map(|d| d.symbols())
        .filter(|_| !window.no_labels);
    let anchor = match window.address.trim() {
        "" => pc,
        text => number(text)
            .or_else(|| symbols.and_then(|s| s.resolve(text)))
            .unwrap_or(pc),
    };
    // the addresses remembered: a click goes there, + remembers this one
    ui.horizontal(|ui| {
        ui.label("Remembered");
        let mut forget = None;
        for (index, address) in window.bookmarks.iter().enumerate() {
            let response = ui
                .small_button(format!("&{address:04X}"))
                .on_hover_text("Go there (right click: forget it)");
            if response.clicked() {
                window.address = format!("&{address:04X}");
                window.before = 0;
            }
            if response.secondary_clicked() {
                forget = Some(index);
            }
        }
        if let Some(index) = forget {
            window.bookmarks.remove(index);
        }
        if ui
            .small_button("+")
            .on_hover_text(format!("Remember &{anchor:04X}"))
            .clicked()
            && !window.bookmarks.contains(&anchor)
        {
            window.bookmarks.push(anchor);
            if window.bookmarks.len() > 8 {
                window.bookmarks.remove(0);
            }
        }
        if ui
            .button("Another window")
            .on_hover_text("A disassembly window of its own (another address, another paging)")
            .clicked()
        {
            links.push(Link {
                view: View::Disassembly,
                address: anchor,
                new: true,
            });
        }
    });
    // more before (decoded so that the start keeps its instructions), or after
    let paging = window.paging.clone();
    let read = |address: u16| paging.read(mb, address);
    ui.horizontal(|ui| {
        if ui
            .small_button("Earlier")
            .on_hover_text("8 instructions more before the start")
            .clicked()
        {
            window.before += 8;
        }
        if ui
            .small_button("Later")
            .on_hover_text("16 instructions more after")
            .clicked()
        {
            window.rows += 16;
        }
        if (window.before > 0 || window.rows > 32)
            && ui
                .small_button("Reset")
                .on_hover_text("From the start, 32 instructions")
                .clicked()
        {
            window.before = 0;
            window.rows = 32;
        }
    });
    let mut start = anchor;
    let mut shown_before = 0;
    if window.before > 0 {
        // as many as fall into step before the start (none in data)
        for count in (1..=window.before).rev() {
            if let Some(found) = start_before(anchor, count, read) {
                start = found;
                shown_before = count;
                break;
            }
        }
    }
    let total = shown_before + window.rows;
    let length = (total as u16 + 4) * LONGEST;
    let bytes: Vec<u8> = (0..length).map(|i| read(start.wrapping_add(i))).collect();
    let instructions = cpclib_dap::disassemble::decode(start, &bytes, total);
    // the jumps and calls, drawn in a gutter left of the instructions
    const GUTTER: f32 = 44.0;
    let rows: Vec<(u16, &[u8])> = instructions
        .iter()
        .map(|i| (i.address, i.bytes.as_slice()))
        .collect();
    let arcs = super::chips::arcs(&rows, 6);
    let mut rows_y = Vec::with_capacity(instructions.len());
    let mut gutter = egui::Rect::NOTHING;
    let name_at = |address: u16| symbols.and_then(|s| s.name_at(address)).map(str::to_owned);
    let f = crate::z80::HasValue::value(mb.cpu().reg8(crate::z80::Reg8::F));
    egui::Grid::new((id, "instructions"))
        .striped(true)
        .show(ui, |ui| {
            for instruction in &instructions {
                let at = instruction.address;
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
                // the PC, and the start (where the earlier rows end)
                let here = if at == pc {
                    "▶"
                } else if at == anchor && shown_before > 0 {
                    "·"
                } else {
                    " "
                };
                let label = match symbols.and_then(|s| s.name_at(at)) {
                    Some(name) => format!("{here}{at:04X} {name}:"),
                    None => format!("{here}{at:04X}"),
                };
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
                        window.address = format!("&{at:04X}");
                        window.before = 0;
                        ui.close();
                    }
                    if ui
                        .button("Disassemble from here, in another window")
                        .clicked()
                    {
                        links.push(Link {
                            view: View::Disassembly,
                            address: at,
                            new: true,
                        });
                        ui.close();
                    }
                    if ui.button("Show in the memory window").clicked() {
                        links.push(Link {
                            view: View::Memory,
                            address: at,
                            new: false,
                        });
                        ui.close();
                    }
                });
                let hex: Vec<String> = instruction
                    .bytes
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect();
                ui.weak(egui::RichText::new(hex.join(" ")).monospace());
                let mut text =
                    super::chips::with_labels(&instruction.text, at, &instruction.bytes, name_at);
                let branch = super::chips::branch(at, &instruction.bytes, f);
                if let Some((_, taken)) = branch {
                    if let (true, Some(taken)) = (at == pc, taken) {
                        text.push_str(if taken { "  (taken)" } else { "  (not taken)" });
                    }
                }
                ui.monospace(text);
                ui.weak(match instruction.cost {
                    Some(nops) => format!("{nops} NOP{}", if nops == 1 { "" } else { "s" }),
                    None => String::new(),
                });
                // ACE's "» &1F25": the disassembly from where it goes
                match branch.and_then(|(target, _)| target) {
                    Some(destination) => {
                        if ui
                            .small_button(format!("» &{destination:04X}"))
                            .on_hover_text("Disassemble from there")
                            .clicked()
                        {
                            window.address = format!("&{destination:04X}");
                            window.before = 0;
                        }
                    }
                    None => {
                        ui.label("");
                    }
                }
                ui.end_row();
            }
        });
    if !rows_y.is_empty() {
        let pc_row = instructions.iter().position(|i| i.address == pc);
        super::chips::paint_arcs(ui.painter(), gutter, &rows_y, &arcs, pc_row);
    }
    let first = instructions.first().map_or(anchor, |i| i.address);
    ui.horizontal(|ui| {
        ui.label(format!("From &{first:04X} ({first})"));
        if ui
            .button("» Edit…")
            .on_hover_text("The memory window there")
            .clicked()
        {
            links.push(Link {
                view: View::Memory,
                address: first,
                new: false,
            });
        }
    });
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn the_rows_before_a_start_fall_into_step_with_it() {
        // NOPs, then LD HL,&2121 (21 21 21) right before the start: from far enough back the
        // decoding must take 21 21 21 as one instruction and land on the start
        let mut memory = vec![0u8; 0x100];
        memory[0x40..0x43].copy_from_slice(&[0x21, 0x21, 0x21]);
        memory[0x43] = 0xc9; // the start: RET
        let read = |address: u16| memory[usize::from(address) & 0xff];
        let start = start_before(0x43, 2, read).unwrap();
        // two instructions before: a NOP at &3F, the LD at &40
        assert_eq!(start, 0x3f);
        let one = start_before(0x43, 1, read).unwrap();
        assert_eq!(one, 0x40);
        assert_eq!(start_before(0x43, 0, read), Some(0x43));
    }
}
