//! The chips' windows: what the registers mean, decoded (as WinAPE, ACE and MAME show them),
//! over the raw values `DebugServer::inspect` serves the API.

use crate::commands::{Arg, Command, CommandBus};
use crate::ga::RgbLevels;
use crate::motherboard::Motherboard;

fn color(levels: RgbLevels) -> egui::Color32 {
    let byte = |level: f32| (level * 255.0 / 100.0).round().clamp(0.0, 255.0) as u8;
    egui::Color32::from_rgb(byte(levels.r()), byte(levels.g()), byte(levels.b()))
}

/// A coloured cell, outlined when `selected`.
fn swatch(ui: &mut egui::Ui, fill: egui::Color32, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(22.0, 16.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, fill);
    let stroke = if selected {
        egui::Stroke::new(2.0_f32, ui.visuals().selection.stroke.color)
    } else {
        egui::Stroke::new(1.0_f32, ui.visuals().weak_text_color())
    };
    painter.rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Outside);
    response
}

fn yes(on: bool) -> &'static str {
    if on {
        "yes"
    } else {
        "no"
    }
}

/// The Gate Array: the inks as colours, the modes, the ROM and interrupt state.
pub fn gate_array(ui: &mut egui::Ui, mb: &Motherboard) {
    let ga = mb.ga();
    let palette = ga.palette();
    let pen = ga.selected_pen();
    let mode = |mode: crate::ga::VideoMode| mode as u8;
    ui.label(format!(
        "{}: mode {} (next line: mode {})",
        match ga.gate_array_type() {
            crate::ga::GateArrayType::Ga40010 => "40010",
            _ => "40007/40008",
        },
        mode(ga.active_video_mode()),
        mode(ga.next_video_mode())
    ));
    egui::Grid::new("ga-inks")
        .spacing([4.0, 6.0])
        .show(ui, |ui| {
            for (index, ink) in palette.iter().enumerate() {
                ui.vertical_centered(|ui| {
                    let label = if index == 16 {
                        "B".to_owned()
                    } else {
                        index.to_string()
                    };
                    ui.small(label);
                    swatch(ui, color(RgbLevels::from(*ink)), usize::from(pen) == index)
                        .on_hover_text(format!(
                            "pen {index}: ink {} (hardware &{:02X})",
                            ink.number(),
                            ink.hardware_number()
                        ));
                });
                if index == 8 {
                    ui.end_row();
                }
            }
        });
    let rmr = ga.rmr();
    egui::Grid::new("ga-state").striped(true).show(ui, |ui| {
        ui.label("Selected pen");
        ui.monospace(if pen == 16 {
            "border".to_owned()
        } else {
            pen.to_string()
        });
        ui.end_row();
        ui.label("RMR");
        ui.monospace(format!(
            "&{rmr:02X}: mode {}, lower ROM {}, upper ROM {}",
            rmr & 3,
            if rmr & 4 == 0 { "on" } else { "off" },
            if rmr & 8 == 0 { "on" } else { "off" },
        ));
        ui.end_row();
        ui.label("Upper ROM");
        ui.monospace(format!("{}", mb.upper_rom_number()));
        ui.end_row();
        ui.label("Interrupt counter (R52)");
        ui.monospace(format!("{} / 52", ga.r52()));
        ui.end_row();
    });
}

/// Note name of a frequency (A4 = 440 Hz).
fn note(frequency: f64) -> String {
    if !(16.0..20_000.0).contains(&frequency) {
        return String::new();
    }
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let midi = (69.0 + 12.0 * (frequency / 440.0).log2()).round() as i32;
    format!("{}{}", NAMES[midi.rem_euclid(12) as usize], midi / 12 - 1)
}

/// The envelope shapes (R13), as the AY's datasheet draws them.
const SHAPES: [&str; 16] = [
    "\\___", "\\___", "\\___", "\\___", "/___", "/___", "/___", "/___", "\\\\\\\\", "\\___",
    "\\/\\/", "\\¯¯¯", "////", "/¯¯¯", "/\\/\\", "/___",
];

/// The AY's clock on a CPC: 1 MHz.
const PSG_HZ: f64 = 1_000_000.0;

/// The PSG: per channel the note, the mixer and the volume or envelope, then the noise and
/// the envelope.
pub fn psg(ui: &mut egui::Ui, mb: &Motherboard) {
    let psg = mb.psg();
    let reg = |r: u8| psg.register(r);
    let mixer = reg(7);
    let levels = psg.channel_levels();
    egui::Grid::new("psg-channels")
        .striped(true)
        .show(ui, |ui| {
            for heading in [
                "",
                "Period",
                "Frequency",
                "Note",
                "Tone",
                "Noise",
                "Volume",
                "Level",
            ] {
                ui.strong(heading);
            }
            ui.end_row();
            for channel in 0..3u8 {
                let period =
                    u16::from(reg(channel * 2)) | u16::from(reg(channel * 2 + 1) & 0x0f) << 8;
                let frequency = PSG_HZ / (16.0 * f64::from(period.max(1)));
                let volume = reg(8 + channel);
                ui.strong(["A", "B", "C"][usize::from(channel)]);
                ui.monospace(format!("{period}"));
                ui.monospace(format!("{frequency:.1} Hz"));
                ui.monospace(note(frequency));
                ui.monospace(yes(mixer & (1 << channel) == 0));
                ui.monospace(yes(mixer & (8 << channel) == 0));
                ui.monospace(if volume & 0x10 != 0 {
                    "envelope".to_owned()
                } else {
                    format!("{}", volume & 0x0f)
                });
                ui.add(
                    egui::ProgressBar::new(levels[usize::from(channel)].clamp(0.0, 1.0))
                        .desired_width(80.0),
                );
                ui.end_row();
            }
        });
    let noise = reg(6) & 0x1f;
    let envelope = u16::from(reg(11)) | u16::from(reg(12)) << 8;
    let shape = reg(13) & 0x0f;
    egui::Grid::new("psg-other").striped(true).show(ui, |ui| {
        ui.label("Noise");
        ui.monospace(format!(
            "period {noise}: {:.0} Hz",
            PSG_HZ / (16.0 * f64::from(noise.max(1)))
        ));
        ui.end_row();
        ui.label("Envelope");
        ui.monospace(format!(
            "period {envelope}: {:.2} Hz, shape {shape} {}",
            PSG_HZ / (256.0 * f64::from(envelope.max(1))),
            SHAPES[usize::from(shape)]
        ));
        ui.end_row();
        ui.label("Port A (R14)");
        ui.monospace(format!("&{:02X}", reg(14)));
        ui.end_row();
    });
}

/// The CRTC: the counters' flags and what the registers make (timings, screen address).
pub fn crtc(ui: &mut egui::Ui, mb: &Motherboard) {
    let crtc = mb.crtc();
    let r = crtc.registers();
    let counters = crtc.counters();
    let line_us = u32::from(r[0]) + 1;
    let char_lines = u32::from(r[9] & 0x1f) + 1;
    let frame_lines = (u32::from(r[4] & 0x7f) + 1) * char_lines + u32::from(r[5] & 0x1f);
    let frame_us = frame_lines * line_us;
    let vsync = match r[3] >> 4 {
        0 => 16,
        lines => lines,
    };
    let ma = u16::from(r[12] & 0x3f) << 8 | u16::from(r[13]);
    let page = (ma >> 12) & 3;
    let size_32k = (ma >> 10) & 3 == 3;
    let start = 0x4000 * page + ((ma & 0x3ff) << 1);
    egui::Grid::new("crtc-decoded").striped(true).show(ui, |ui| {
        ui.label("Flags");
        ui.monospace(format!(
            "HSYNC {} (HSC {}), VSYNC {} (VSC {}), adjust {}",
            yes(counters.hsync),
            counters.hsc,
            yes(counters.vsync),
            counters.vsc,
            yes(counters.in_vertical_adjust)
        ));
        ui.end_row();
        ui.label("Line");
        ui.monospace(format!(
            "{line_us} µs, {} characters shown, HSYNC at {} for {} µs",
            r[1],
            r[2],
            r[3] & 0x0f
        ));
        ui.end_row();
        ui.label("Frame");
        ui.monospace(format!(
            "{frame_lines} lines ({:.2} Hz), {} character rows of {char_lines} lines shown, VSYNC at row {} for {vsync} lines",
            1_000_000.0 / f64::from(frame_us.max(1)),
            r[6],
            r[7]
        ));
        ui.end_row();
        ui.label("Screen");
        ui.monospace(format!(
            "&{start:04X} ({}K, page &{:04X}, offset &{:03X})",
            if size_32k { 32 } else { 16 },
            0x4000 * page,
            (ma & 0x3ff) << 1
        ));
        ui.end_row();
        ui.label("Cursor (R14-R15)");
        ui.monospace(format!("&{:02X}{:02X}", r[14], r[15]));
        ui.end_row();
        ui.label("Light pen (R16-R17)");
        ui.monospace(format!("&{:02X}{:02X}", r[16], r[17]));
        ui.end_row();
    });
}

/// The PPI: its ports decoded and the keyboard matrix.
pub fn ppi(ui: &mut egui::Ui, mb: &Motherboard) {
    let ppi = mb.ppi();
    let b = ppi.get_data_b_from_pins();
    let c = ppi.get_data_c_from_pins();
    let psg_function = match c >> 6 {
        0 => "inactive",
        1 => "read",
        2 => "write",
        _ => "select register",
    };
    egui::Grid::new("ppi-ports").striped(true).show(ui, |ui| {
        ui.label("Control");
        ui.monospace(format!(
            "&{:02X} (port A {})",
            ppi.control_word(),
            if ppi.port_a_is_output() {
                "output"
            } else {
                "input"
            }
        ));
        ui.end_row();
        ui.label("Port A");
        ui.monospace(format!("&{:02X}", ppi.port_a_latch()));
        ui.end_row();
        ui.label("Port B");
        ui.monospace(format!(
            "&{b:02X}: VSYNC {}, brand {:?}, {} Hz, /EXP {}, printer busy {}, cassette in {}",
            yes(b & 1 != 0),
            ppi.brand(),
            if b & 0x10 != 0 { 50 } else { 60 },
            if b & 0x20 != 0 { "high" } else { "low" },
            yes(b & 0x40 != 0),
            u8::from(b & 0x80 != 0)
        ));
        ui.end_row();
        ui.label("Port C");
        ui.monospace(format!(
            "&{c:02X}: keyboard row {}, motor {}, cassette out {}, PSG {psg_function}",
            c & 0x0f,
            if c & 0x10 != 0 { "on" } else { "off" },
            u8::from(c & 0x20 != 0)
        ));
        ui.end_row();
    });
    ui.separator();
    ui.strong("Keyboard matrix (X: key down)");
    let lines = mb.keyboard().lines();
    egui::Grid::new("ppi-keyboard")
        .spacing([6.0, 0.0])
        .show(ui, |ui| {
            ui.weak("row");
            for bit in (0..8).rev() {
                ui.weak(bit.to_string());
            }
            ui.end_row();
            for (row, line) in lines.iter().enumerate() {
                ui.monospace(row.to_string());
                for bit in (0..8).rev() {
                    ui.monospace(if line & (1 << bit) == 0 { "X" } else { "·" });
                }
                ui.end_row();
            }
        });
}

/// The cassette deck and the recorder.
pub fn tape(ui: &mut egui::Ui, mb: &Motherboard, bus: &mut CommandBus) {
    let motor = mb.ppi().get_data_c_from_pins() & 0x10 != 0;
    match mb.tape_deck() {
        Some(deck) => {
            let duration = deck.tape().duration_seconds();
            let position = deck.position_seconds();
            ui.add(
                egui::ProgressBar::new((position / duration.max(1e-6)) as f32)
                    .desired_width(320.0)
                    .text(format!("{position:.1} s / {duration:.1} s")),
            );
            egui::Grid::new("tape").striped(true).show(ui, |ui| {
                ui.label("PLAY");
                ui.monospace(if deck.at_end() {
                    "at the end"
                } else if deck.playing() {
                    "down"
                } else {
                    "up"
                });
                ui.end_row();
                ui.label("Motor");
                ui.monospace(if motor { "on" } else { "off" });
                ui.end_row();
                ui.label("Head level");
                ui.monospace(u8::from(deck.level()).to_string());
                ui.end_row();
            });
            ui.horizontal(|ui| {
                for (label, id) in [
                    ("Play", "tape-play"),
                    ("Stop", "tape-stop"),
                    ("Rewind", "tape-rewind"),
                ] {
                    if ui.button(label).clicked() {
                        bus.push_id(id);
                    }
                }
            });
        }
        None => {
            ui.weak(format!(
                "no tape (motor {})",
                if motor { "on" } else { "off" }
            ));
        }
    }
    ui.separator();
    match mb.tape_recorder() {
        Some(recorder) => {
            ui.label(format!("Recording: {:.1} s of tape", recorder.seconds()));
            if ui.button("Stop and save…").clicked() {
                bus.push_id("tape-record-stop");
            }
        }
        None => {
            if ui.button("Record what the CPC saves").clicked() {
                bus.push_id("tape-record");
            }
        }
    }
}

/// The CPC+'s ASIC: lock, interrupts, split, and its 32 colours.
pub fn asic(ui: &mut egui::Ui, mb: &Motherboard) {
    let (Some(asic), Some(video)) = (mb.asic(), mb.ga().plus()) else {
        ui.weak("a CPC+ has one (Machine > Platform)");
        return;
    };
    egui::Grid::new("asic").striped(true).show(ui, |ui| {
        ui.label("Registers");
        ui.monospace(if asic.unlocked() {
            "unlocked"
        } else {
            "locked"
        });
        ui.end_row();
        ui.label("Raster interrupt (PRI)");
        ui.monospace(yes(asic.programmable_raster()));
        ui.end_row();
        ui.label("Split (SPLT, SSA)");
        ui.monospace(match asic.split() {
            Some((line, address)) => format!("after line {line}, from &{address:04X}"),
            None => "no".to_owned(),
        });
        ui.end_row();
        ui.label("DMA interrupt");
        ui.monospace(yes(asic.dma_interrupt()));
        ui.end_row();
    });
    ui.separator();
    ui.strong("Palette: pens, border, sprite colours");
    egui::Grid::new("asic-palette")
        .spacing([4.0, 6.0])
        .show(ui, |ui| {
            for index in 0..32 {
                ui.vertical_centered(|ui| {
                    ui.small(match index {
                        16 => "B".to_owned(),
                        17.. => format!("S{}", index - 16),
                        _ => index.to_string(),
                    });
                    swatch(ui, color(video.rgb(index)), false);
                });
                if index % 11 == 10 {
                    ui.end_row();
                }
            }
        });
}

/// The names of the bits of `value` that are set (`names[0]` is bit 7).
fn bits(value: u8, names: [&str; 8]) -> String {
    let set: Vec<&str> = names
        .iter()
        .enumerate()
        .filter(|(i, name)| !name.is_empty() && value & (0x80 >> i) != 0)
        .map(|(_, name)| *name)
        .collect();
    if set.is_empty() {
        "-".to_owned()
    } else {
        set.join(" ")
    }
}

/// The FDC: its status registers decoded, the drives, the last commands.
pub fn fdc(ui: &mut egui::Ui, mb: &Motherboard) {
    use crate::fdc::FdcDrive;
    let fdc = mb.fdc();
    let msr = fdc.read_status();
    let [st0, st1, st2] = fdc.status_registers();
    let interrupt = match st0 >> 6 {
        0 => "normal end",
        1 => "abnormal end",
        2 => "invalid command",
        _ => "drive not ready",
    };
    egui::Grid::new("fdc-status").striped(true).show(ui, |ui| {
        ui.label("Phase");
        ui.monospace(format!("{} ({})", fdc.phase_name(), fdc.command_name()));
        ui.end_row();
        ui.label("MSR");
        ui.monospace(format!(
            "&{msr:02X}: {}",
            bits(msr, ["RQM", "DIO", "EXM", "CB", "D3B", "D2B", "D1B", "D0B"])
        ));
        ui.end_row();
        ui.label("ST0");
        ui.monospace(format!(
            "&{st0:02X}: {interrupt}, {}, head {}, unit {}",
            bits(st0 & 0x38, ["", "", "SE", "EC", "NR", "", "", ""]),
            (st0 >> 2) & 1,
            st0 & 3
        ));
        ui.end_row();
        ui.label("ST1");
        ui.monospace(format!(
            "&{st1:02X}: {}",
            bits(st1, ["EN", "", "DE", "OR", "", "ND", "NW", "MA"])
        ));
        ui.end_row();
        ui.label("ST2");
        ui.monospace(format!(
            "&{st2:02X}: {}",
            bits(st2, ["", "CM", "DD", "WC", "SH", "SN", "BC", "MD"])
        ));
        ui.end_row();
        ui.label("Motor");
        ui.monospace(if fdc.motor() { "on" } else { "off" });
        ui.end_row();
    });
    ui.separator();
    egui::Grid::new("fdc-drives").striped(true).show(ui, |ui| {
        for heading in ["Drive", "Kind", "Disc", "Track", "Side up", "Modified"] {
            ui.strong(heading);
        }
        ui.end_row();
        for (unit, drive) in [FdcDrive::A, FdcDrive::B].into_iter().enumerate() {
            ui.monospace(["A", "B"][unit]);
            ui.monospace(fdc.drive_kind(drive).name());
            ui.monospace(yes(fdc.has_disk(drive)));
            ui.monospace(fdc.track(unit).to_string());
            ui.monospace(if fdc.disc_side(drive) == 0 { "A" } else { "B" });
            ui.monospace(yes(fdc.is_modified(drive)));
            ui.end_row();
        }
    });
    ui.separator();
    ui.strong("Last commands (the last first)");
    let hex = |bytes: &[u8]| {
        bytes
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    egui::Grid::new("fdc-commands")
        .striped(true)
        .show(ui, |ui| {
            let mut any = false;
            for record in fdc.recent_commands().rev() {
                any = true;
                ui.monospace(if record.repeats > 0 {
                    format!("{} ×{}", record.name(), record.repeats + 1)
                } else {
                    record.name()
                });
                ui.monospace(hex(&record.command));
                ui.weak(if record.result.is_empty() {
                    String::new()
                } else {
                    format!("→ {}", hex(&record.result))
                });
                ui.end_row();
            }
            if !any {
                ui.weak("none yet");
            }
        });
}

/// The Z80's state beyond its registers: HALT, WZ, the bytes the pointers point at.
pub fn z80(ui: &mut egui::Ui, mb: &Motherboard) {
    use crate::z80::{HasValue, Reg16};
    let cpu = mb.cpu();
    let r16 = |reg: Reg16| cpu.reg16(reg).value();
    let pc = r16(Reg16::Pc);
    egui::Grid::new("z80-more").striped(true).show(ui, |ui| {
        ui.label("State");
        // a HALT runs again until an interrupt: the PC stays on it
        ui.monospace(if mb.read_byte(pc) == 0x76 {
            "HALT, waiting for an interrupt"
        } else {
            "running"
        });
        ui.end_row();
        ui.label("WZ (MEMPTR)");
        ui.monospace(format!("&{:04X}", r16(Reg16::Wz)));
        ui.end_row();
        for (name, reg) in [
            ("(HL)", Reg16::Hl),
            ("(DE)", Reg16::De),
            ("(BC)", Reg16::Bc),
            ("(SP)", Reg16::Sp),
            ("(IX)", Reg16::Ix),
            ("(IY)", Reg16::Iy),
        ] {
            let at = r16(reg);
            let bytes: Vec<String> = (0..8u16)
                .map(|i| format!("{:02X}", mb.read_byte(at.wrapping_add(i))))
                .collect();
            ui.label(name);
            ui.monospace(format!("&{at:04X}: {}", bytes.join(" ")));
            ui.end_row();
        }
    });
}

/// The memory map: what each 16K of the CPU's view reads and writes, by name.
pub fn memory_map(ui: &mut egui::Ui, mb: &Motherboard) {
    use crate::memory::PhysicalAddress;
    let mmr = mb.mmr();
    // the Gate Array's RAM configurations (the PAL's, &7Fxx with bits 7-6 = 11)
    const CONFIGS: [&str; 8] = [
        "0 1 2 3 (C0)",
        "0 1 2 7 (C1)",
        "4 5 6 7 (C2)",
        "0 3 2 7 (C3)",
        "0 4 2 3 (C4)",
        "0 5 2 3 (C5)",
        "0 6 2 3 (C6)",
        "0 7 2 3 (C7)",
    ];
    ui.label(format!(
        "RAM configuration C{}, page {} (MMR &{mmr:02X}): banks {}",
        mmr & 7,
        (mmr >> 3) & 7,
        CONFIGS[usize::from(mmr & 7)]
    ));
    if let Some(banks) = mb.large_ram_expansion_bank_count() {
        ui.label(format!("4M card: {banks} banks of 64K"));
    }
    let name = |address: PhysicalAddress| match address {
        PhysicalAddress::LowerRom { .. } => "lower ROM".to_owned(),
        PhysicalAddress::UpperRom { rom, .. } => format!("upper ROM {rom}"),
        PhysicalAddress::Ram { bank, .. } => format!("RAM bank {bank}"),
    };
    egui::Grid::new("memmap-regions")
        .striped(true)
        .show(ui, |ui| {
            for heading in ["Region", "Read", "Write"] {
                ui.strong(heading);
            }
            ui.end_row();
            for region in 0..4u16 {
                let base = region * 0x4000;
                ui.monospace(format!("&{base:04X}-&{:04X}", base + 0x3fff));
                ui.monospace(name(mb.physical_address(base)));
                ui.monospace(name(mb.physical_address_for_write(base)));
                ui.end_row();
            }
        });
}

/// A jump, call or return: its target (none for a return or an indirect jump) and, when
/// conditional, whether it is taken with the flags `f`.
pub fn branch(address: u16, bytes: &[u8], f: u8) -> Option<(Option<u16>, Option<bool>)> {
    let op = *bytes.first()?;
    let word = || Some(u16::from_le_bytes([*bytes.get(1)?, *bytes.get(2)?]));
    let relative = || {
        let e = *bytes.get(1)? as i8;
        Some(address.wrapping_add(2).wrapping_add(e as u16))
    };
    // NZ Z NC C PO PE P M
    let holds = |cc: u8| {
        let (mask, set) = match cc {
            0 | 1 => (0x40, cc == 1),
            2 | 3 => (0x01, cc == 3),
            4 | 5 => (0x04, cc == 5),
            _ => (0x80, cc == 7),
        };
        (f & mask != 0) == set
    };
    let cc = (op >> 3) & 7;
    Some(match op {
        0xc3 => (word(), None),
        0xcd => (word(), None),
        0x18 => (relative(), None),
        0x10 => (relative(), None),
        0xc9 => (None, None),
        _ if op & 0xc7 == 0xc2 || op & 0xc7 == 0xc4 => (word(), Some(holds(cc))),
        0x20 | 0x28 | 0x30 | 0x38 => (relative(), Some(holds(cc - 4))),
        _ if op & 0xc7 == 0xc0 => (None, Some(holds(cc))),
        _ => return None,
    })
}

#[cfg(test)]
mod test {
    #[test]
    fn a_branch_is_taken_as_the_flags_say() {
        // JR NZ,+2 at &4000: Z clear, taken, to &4004
        assert_eq!(
            super::branch(0x4000, &[0x20, 0x02], 0x00),
            Some((Some(0x4004), Some(true)))
        );
        assert_eq!(
            super::branch(0x4000, &[0x20, 0x02], 0x40),
            Some((Some(0x4004), Some(false)))
        );
        // CALL C,&BB5A with carry
        assert_eq!(
            super::branch(0, &[0xdc, 0x5a, 0xbb], 0x01),
            Some((Some(0xbb5a), Some(true)))
        );
        // RET M, sign clear
        assert_eq!(super::branch(0, &[0xf8], 0x00), Some((None, Some(false))));
        assert_eq!(super::branch(0, &[0x3e, 0x01], 0), None);
    }
}

fn run(bus: &mut CommandBus, id: &str, arg: Arg) {
    if let Ok(command) = Command::with_arg(id, arg) {
        bus.push(command);
    }
}

/// The Z80's registers, each a field: dragged or typed, the register is set (Debug > Set
/// register).
pub fn registers(ui: &mut egui::Ui, mb: &Motherboard, bus: &mut CommandBus) {
    use crate::z80::{HasValue, Reg16, Reg8};
    let cpu = mb.cpu();
    let pairs = [
        ("PC", cpu.reg16(Reg16::Pc).value()),
        ("SP", cpu.reg16(Reg16::Sp).value()),
        ("AF", cpu.reg16(Reg16::Af).value()),
        ("BC", cpu.reg16(Reg16::Bc).value()),
        ("DE", cpu.reg16(Reg16::De).value()),
        ("HL", cpu.reg16(Reg16::Hl).value()),
        ("IX", cpu.reg16(Reg16::Ix).value()),
        ("IY", cpu.reg16(Reg16::Iy).value()),
    ];
    let shadows = [
        ("AF'", cpu.reg16(Reg16::Af2).value()),
        ("BC'", cpu.reg16(Reg16::Bc2).value()),
        ("DE'", cpu.reg16(Reg16::De2).value()),
        ("HL'", cpu.reg16(Reg16::Hl2).value()),
    ];
    egui::Grid::new("z80-registers")
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            for (row, (name, value)) in pairs.iter().enumerate() {
                ui.monospace(*name);
                let mut edited = u32::from(*value);
                let response = ui.add(
                    egui::DragValue::new(&mut edited)
                        .range(0..=0xffff)
                        .hexadecimal(4, false, true)
                        .prefix("&")
                        .speed(1.0),
                );
                if response.changed() && edited != u32::from(*value) {
                    run(
                        bus,
                        "z80-set-register",
                        Arg::Text(format!("{name}={edited}")),
                    );
                }
                ui.weak(format!("{value}"));
                if let Some((shadow, value)) = shadows.get(row) {
                    ui.monospace(*shadow);
                    ui.monospace(format!("&{value:04X}"));
                }
                ui.end_row();
            }
            for (name, reg) in [("I", Reg8::I), ("R", Reg8::R)] {
                let value = cpu.reg8(reg).value();
                ui.monospace(name);
                let mut edited = u32::from(value);
                if ui
                    .add(
                        egui::DragValue::new(&mut edited)
                            .range(0..=0xff)
                            .hexadecimal(2, false, true)
                            .prefix("&"),
                    )
                    .changed()
                    && edited != u32::from(value)
                {
                    run(
                        bus,
                        "z80-set-register",
                        Arg::Text(format!("{name}={edited}")),
                    );
                }
                ui.end_row();
            }
        });
    // the flags, each a check box: F set again with that bit changed
    let af = cpu.reg16(Reg16::Af).value();
    let f = af as u8;
    ui.horizontal(|ui| {
        ui.label("F");
        for (bit, name) in ["S", "Z", "Y", "H", "X", "P/V", "N", "C"]
            .iter()
            .enumerate()
        {
            let mask = 0x80u8 >> bit;
            let mut on = f & mask != 0;
            if ui.checkbox(&mut on, *name).changed() {
                let flags = if on { f | mask } else { f & !mask };
                run(
                    bus,
                    "z80-set-register",
                    Arg::Text(format!("AF={}", (af & 0xff00) | u16::from(flags))),
                );
            }
        }
    });
}

/// `text` (cpclib-dap's: `CALL 0xbb5a`, `LD HL, (0xbe00)`) with the addresses it names written
/// as their labels: the branch's target, and the 16-bit operand of the bytes.
pub fn with_labels(
    text: &str,
    address: u16,
    bytes: &[u8],
    name_at: impl Fn(u16) -> Option<String>,
) -> String {
    let mut candidates: Vec<u16> = Vec::new();
    if let Some((Some(target), _)) = branch(address, bytes, 0) {
        candidates.push(target);
    }
    // a 16-bit operand after the opcode, or after a prefix and the opcode
    for start in [1, 2] {
        if let (Some(&low), Some(&high)) = (bytes.get(start), bytes.get(start + 1)) {
            candidates.push(u16::from_le_bytes([low, high]));
        }
    }
    let mut text = text.to_owned();
    for value in candidates {
        let Some(name) = name_at(value) else { continue };
        let number = format!("0x{value:x}");
        // the number as a whole word only (0xb9 is not the start of 0xb978)
        let mut from = 0;
        while let Some(found) = text[from..].find(&number) {
            let start = from + found;
            let end = start + number.len();
            let alone = !text[end..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric());
            if alone {
                text.replace_range(start..end, &name);
                break;
            }
            from = end;
        }
    }
    text
}

/// A jump or call drawn in the disassembly's gutter, from row `from` to row `to` (`None`: out
/// of the rows shown, above when `up`), in lane `lane` (0 nearest the instructions).
#[derive(Clone, Debug, PartialEq)]
pub struct Arc {
    pub from: usize,
    pub to: Option<usize>,
    pub up: bool,
    pub lane: usize,
    pub call: bool,
}

/// The arcs of the jumps and calls among `rows` (their address and bytes): the shortest in the
/// lanes nearest the instructions, none sharing a lane where they overlap; at most `lanes`.
pub fn arcs(rows: &[(u16, &[u8])], lanes: usize) -> Vec<Arc> {
    let first = rows.first().map_or(0, |r| r.0);
    let mut found: Vec<(usize, Option<usize>, bool, bool)> = rows
        .iter()
        .enumerate()
        .filter_map(|(index, (address, bytes))| {
            let (target, _) = branch(*address, bytes, 0)?;
            let target = target?;
            let call = matches!(bytes.first(), Some(op) if *op == 0xcd || op & 0xc7 == 0xc4);
            let to = rows.iter().position(|(a, _)| *a == target);
            // a target between two rows (inside an instruction) or off the rows: an arrow out
            let up = target < first || (to.is_none() && target <= *address);
            Some((index, to, up, call))
        })
        .collect();
    // the shortest first: they nest inside the longer ones
    let span = |(from, to, up, _): &(usize, Option<usize>, bool, bool)| match to {
        Some(to) => from.abs_diff(*to),
        None if *up => *from,
        None => rows.len() - from,
    };
    found.sort_by_key(span);
    let mut used: Vec<Vec<(usize, usize)>> = vec![Vec::new(); lanes];
    let mut arcs = Vec::new();
    for (from, to, up, call) in found {
        let (low, high) = match to {
            Some(to) => (from.min(to), from.max(to)),
            None if up => (0, from),
            None => (from, rows.len()),
        };
        let Some(lane) = used
            .iter()
            .position(|taken| taken.iter().all(|&(l, h)| high < l || low > h))
        else {
            continue;
        };
        used[lane].push((low, high));
        arcs.push(Arc {
            from,
            to,
            up,
            lane,
            call,
        });
    }
    arcs
}

/// Draw `arcs` in the gutter `gutter` (its right side touches the instructions), the rows' middles
/// at `rows_y`; the one leaving the PC's row brighter.
pub fn paint_arcs(
    painter: &egui::Painter,
    gutter: egui::Rect,
    rows_y: &[f32],
    arcs: &[Arc],
    pc_row: Option<usize>,
) {
    const LANE: f32 = 6.0;
    for arc in arcs {
        let lit = Some(arc.from) == pc_row;
        let color = match (arc.call, lit) {
            (true, false) => egui::Color32::from_rgb(170, 120, 60),
            (true, true) => egui::Color32::from_rgb(255, 180, 80),
            (false, false) => egui::Color32::from_rgb(80, 130, 190),
            (false, true) => egui::Color32::from_rgb(120, 190, 255),
        };
        let stroke = egui::Stroke::new(if lit { 2.0_f32 } else { 1.0_f32 }, color);
        let right = gutter.right() - 2.0;
        let x = (right - 6.0 - LANE * arc.lane as f32).max(gutter.left() + 1.0);
        let y0 = rows_y[arc.from];
        let y1 = match arc.to {
            Some(to) => rows_y[to],
            None if arc.up => gutter.top(),
            None => gutter.bottom(),
        };
        painter.line_segment([egui::pos2(right, y0), egui::pos2(x, y0)], stroke);
        painter.line_segment([egui::pos2(x, y0), egui::pos2(x, y1)], stroke);
        if arc.to.is_some() {
            painter.line_segment([egui::pos2(x, y1), egui::pos2(right, y1)], stroke);
            // the arrow head on the target
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(right, y1),
                    egui::pos2(right - 4.0, y1 - 3.0),
                    egui::pos2(right - 4.0, y1 + 3.0),
                ],
                color,
                egui::Stroke::NONE,
            ));
        } else {
            // out of the rows shown: an arrow up or down at the edge
            let dy = if arc.up { 4.0 } else { -4.0 };
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(x, y1),
                    egui::pos2(x - 3.0, y1 + dy),
                    egui::pos2(x + 3.0, y1 + dy),
                ],
                color,
                egui::Stroke::NONE,
            ));
        }
    }
}

#[cfg(test)]
mod disassembly_test {
    use super::*;

    #[test]
    fn operands_are_written_as_labels() {
        let name = |a: u16| match a {
            0xbb5a => Some("TXT_OUTPUT".to_owned()),
            0xb9 => Some("short".to_owned()),
            _ => None,
        };
        assert_eq!(
            with_labels("CALL 0xbb5a", 0, &[0xcd, 0x5a, 0xbb], name),
            "CALL TXT_OUTPUT"
        );
        // a number that only begins like a label's is left
        assert_eq!(
            with_labels("LD HL, 0xb978", 0, &[0x21, 0x78, 0xb9], name),
            "LD HL, 0xb978"
        );
        assert_eq!(with_labels("LD A, C", 0, &[0x79], name), "LD A, C");
    }

    #[test]
    fn arcs_take_lanes_without_overlapping() {
        let jr = |e: u8| [0x18, e];
        // row 0: JR to row 2; row 1: JR to row 3 (overlaps the first); row 2: NOP; row 3: JP out
        let a = jr(2);
        let b = jr(1);
        let rows: Vec<(u16, &[u8])> = vec![
            (0x100, &a),
            (0x102, &b),
            (0x104, &[0x00]),
            (0x105, &[0xc3, 0x00, 0x80]),
        ];
        let arcs = arcs(&rows, 4);
        assert_eq!(arcs.len(), 3);
        let to = |from: usize| arcs.iter().find(|a| a.from == from).unwrap();
        assert_eq!(to(0).to, Some(2));
        assert_eq!(to(1).to, Some(3));
        assert_ne!(to(0).lane, to(1).lane);
        // the JP to &8000 leaves the rows downwards
        assert_eq!((to(3).to, to(3).up), (None, false));
    }
}

/// A LED: lit (green, or red for a recording) or off.
fn led(ui: &mut egui::Ui, lit: bool, color: egui::Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    let fill = if lit {
        color
    } else {
        ui.visuals().widgets.noninteractive.bg_fill
    };
    ui.painter().circle_filled(rect.center(), 4.0, fill);
    ui.painter().circle_stroke(
        rect.center(),
        4.0,
        egui::Stroke::new(1.0_f32, ui.visuals().weak_text_color()),
    );
    response
}

/// The file name of a path or a name given (`games.zip#side b.dsk` → `side b.dsk`).
fn short_name(name: &str) -> &str {
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    name.rsplit('#').next().unwrap_or(name)
}

/// The status bar of the main window: the machine's configuration, the drives with their LED,
/// the tape, and whether it runs.
pub fn status_bar(ui: &mut egui::Ui, target: &dyn crate::commands::CommandTarget) {
    use crate::fdc::FdcDrive;
    let mb = target.motherboard();
    let settings = target.settings();
    const GREEN: egui::Color32 = egui::Color32::from_rgb(60, 220, 90);
    const RED: egui::Color32 = egui::Color32::from_rgb(230, 60, 60);
    ui.horizontal(|ui| {
        // the drives: the LED lit while the motor turns for the drive addressed
        let fdc = mb.fdc();
        for (unit, drive) in [FdcDrive::A, FdcDrive::B].into_iter().enumerate() {
            let lit = fdc.motor() && fdc.selected_drive() == unit;
            let disc = settings
                .and_then(|s| s.media.discs[unit].as_deref())
                .map(short_name);
            let text = match (fdc.has_disk(drive), disc) {
                (true, Some(name)) => name.to_owned(),
                (true, None) => "disc".to_owned(),
                (false, _) => "empty".to_owned(),
            };
            led(ui, lit, GREEN).on_hover_text(if lit { "the motor turns" } else { "idle" });
            let label = format!("{}: {text}", ["A", "B"][unit]);
            let response = ui.label(if fdc.is_modified(drive) {
                egui::RichText::new(format!("{label} *")).strong()
            } else {
                egui::RichText::new(label)
            });
            response.on_hover_text(format!(
                "drive {}: {}, track {}{}",
                ["A", "B"][unit],
                fdc.drive_kind(drive).name(),
                fdc.track(unit),
                if fdc.is_modified(drive) {
                    ", written to (not saved)"
                } else {
                    ""
                }
            ));
            ui.separator();
        }
        // the tape and the recorder
        let motor = mb.ppi().get_data_c_from_pins() & 0x10 != 0;
        if let Some(deck) = mb.tape_deck() {
            led(ui, motor && deck.playing(), GREEN);
            let name = settings
                .and_then(|s| s.media.tape.as_deref())
                .map(short_name)
                .unwrap_or("tape");
            ui.label(format!(
                "{name} {:.0}/{:.0} s",
                deck.position_seconds(),
                deck.tape().duration_seconds()
            ));
            ui.separator();
        }
        if let Some(recorder) = mb.tape_recorder() {
            led(ui, motor, RED);
            ui.label(format!("REC {:.1} s", recorder.seconds()));
            ui.separator();
        }
        // the machine
        let crtc = mb.crtc();
        let ram = mb.total_ram_kb();
        let ga = match mb.ga().gate_array_type() {
            crate::ga::GateArrayType::Ga40010 => "GA 40010",
            _ => "GA 40007/8",
        };
        let mut machine = format!(
            "CPC {} · CRTC {} · {ga} · {ram}K",
            mb.model().name(),
            crtc.crtc_type() as u8
        );
        let mut cards: Vec<&str> = Vec::new();
        #[cfg(feature = "playcity")]
        if mb.has_expansion::<crate::sound_cards::PlayCity>() {
            cards.push("PlayCity");
        }
        #[cfg(feature = "m4")]
        if mb.has_expansion::<crate::m4::M4Board>() {
            cards.push("M4");
        }
        #[cfg(feature = "albireo")]
        if mb.has_expansion::<crate::ch376::Albireo>() {
            cards.push("Albireo");
        }
        #[cfg(feature = "nova")]
        if mb.has_expansion::<crate::nova::Nova>() {
            cards.push("Nova");
        }
        #[cfg(feature = "dandanator")]
        if mb.has_expansion::<crate::dandanator::Dandanator>() {
            cards.push("Dandanator");
        }
        #[cfg(feature = "booster")]
        if mb.has_expansion::<crate::booster::Booster>() {
            cards.push("Booster");
        }
        #[cfg(feature = "willy")]
        if mb.has_expansion::<crate::willy::Opl3Lpt>() {
            cards.push("Willy");
        }
        if mb
            .printer_port()
            .and_then(|port| port.device_as::<crate::dmp2000::Dmp2000>())
            .is_some()
        {
            cards.push("DMP2000");
        }
        #[cfg(feature = "digiblaster")]
        if mb
            .printer_port()
            .and_then(|port| port.device_as::<crate::sound_cards::Digiblaster>())
            .is_some()
        {
            cards.push("Digiblaster");
        }
        if !cards.is_empty() {
            machine.push_str(&format!(" · {}", cards.join(", ")));
        }
        ui.label(machine).on_hover_text(format!(
            "{} (CRTC type {}), {}K of RAM",
            crtc.chip_name(),
            crtc.crtc_type() as u8,
            ram
        ));
        // how it runs, on the right
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let paused = target
                .debugger()
                .map_or(settings.is_some_and(|s| s.paused), |d| d.paused());
            let state = if target.csl_running() {
                "CSL script".to_owned()
            } else if paused {
                format!("paused at &{:04X}", mb.pc())
            } else {
                match settings.map(|s| s.speed) {
                    Some(crate::commands::Speed::Max) => "full speed".to_owned(),
                    Some(crate::commands::Speed::Factor(f)) if (f - 1.0).abs() > 1e-3 => {
                        format!("{:.0} %", f * 100.0)
                    }
                    _ => "running".to_owned(),
                }
            };
            ui.label(state);
            if let Some(port) = target.debugger().and_then(|d| d.listening_port()) {
                ui.separator();
                ui.label(format!("debug server :{port}"));
            }
        });
    });
}

/// The CRTC's registers as boxes (cpclib's VS Code CRTC view): the name over the value, the
/// selected register outlined, and the registers of a known-bad setting (cpclib-dap's
/// `validate_crtc`) bordered red (the picture is lost) or yellow (not what is likely meant),
/// the reasons below.
pub fn crtc_registers(ui: &mut egui::Ui, mb: &Motherboard) {
    use cpclib_dap::inspect::CrtcSeverity;
    let crtc = mb.crtc();
    let registers = crtc.registers();
    let selected = usize::from(crtc.selected_register());
    let warnings = cpclib_dap::inspect::validate_crtc(&registers);
    const ERROR: egui::Color32 = egui::Color32::from_rgb(220, 60, 60);
    const WARNING: egui::Color32 = egui::Color32::from_rgb(220, 180, 40);
    egui::Grid::new("crtc-boxes")
        .spacing([4.0, 4.0])
        .show(ui, |ui| {
            for (r, value) in registers.iter().enumerate() {
                let name = format!("R{r}");
                let mine: Vec<_> = warnings
                    .iter()
                    .filter(|w| w.registers.contains(&name.as_str()))
                    .collect();
                let border = if mine.iter().any(|w| w.severity == CrtcSeverity::Error) {
                    Some(ERROR)
                } else if mine.is_empty() {
                    None
                } else {
                    Some(WARNING)
                };
                let stroke = match border {
                    Some(color) => egui::Stroke::new(2.0_f32, color),
                    None if r == selected => {
                        egui::Stroke::new(1.5_f32, ui.visuals().selection.stroke.color)
                    }
                    None => egui::Stroke::new(
                        1.0_f32,
                        ui.visuals().widgets.noninteractive.bg_stroke.color,
                    ),
                };
                let fill = match border {
                    Some(color) => color.gamma_multiply(0.18),
                    None => ui.visuals().extreme_bg_color,
                };
                let response = egui::Frame::new()
                    .stroke(stroke)
                    .fill(fill)
                    .corner_radius(3.0)
                    .inner_margin(egui::Margin::symmetric(6, 2))
                    .show(ui, |ui| {
                        ui.set_min_width(40.0);
                        ui.vertical_centered(|ui| {
                            ui.small(name.as_str());
                            ui.monospace(format!("&{value:02X}"));
                        });
                    })
                    .response;
                let mut tooltip = format!("{name} = {value} (&{value:02X})");
                if r == selected {
                    tooltip.push_str(", the register selected");
                }
                for warning in &mine {
                    tooltip.push_str(&format!("\n{}", warning.message));
                }
                response.on_hover_text(tooltip);
                if r % 6 == 5 {
                    ui.end_row();
                }
            }
        });
    for warning in &warnings {
        let color = match warning.severity {
            CrtcSeverity::Error => ERROR,
            CrtcSeverity::Warning => WARNING,
        };
        ui.colored_label(
            color,
            format!("{}: {}", warning.registers.join(", "), warning.message),
        );
    }
}

/// The memory around each register that points somewhere (cpclib's VS Code `-mv all,follow`):
/// two rows of 16 bytes, the byte pointed at outlined, the symbol of the address named.
pub fn follow_registers(
    ui: &mut egui::Ui,
    mb: &Motherboard,
    describe: impl Fn(u16) -> Option<String>,
    read: impl Fn(u16) -> u8,
) {
    let registers = [
        ("PC", mb.pc()),
        ("SP", mb.sp()),
        ("HL", mb.hl()),
        ("DE", mb.de()),
        ("BC", mb.bc()),
        ("IX", mb.ix()),
        ("IY", mb.iy()),
    ];
    let pointed = ui.visuals().selection.stroke.color;
    for (name, value) in registers {
        ui.horizontal(|ui| {
            ui.strong(name);
            ui.monospace(format!("&{value:04X}"));
            if let Some(symbol) = describe(value) {
                ui.weak(symbol);
            }
        });
        let start = value & 0xfff0;
        egui::Grid::new(("follow", name))
            .spacing([6.0, 0.0])
            .min_col_width(0.0)
            .show(ui, |ui| {
                for row in 0..2u16 {
                    let at = start.wrapping_add(row * 16);
                    ui.monospace(format!("{at:04X}"));
                    for column in 0..16u16 {
                        let address = at.wrapping_add(column);
                        let byte = read(address);
                        let mut text = egui::RichText::new(format!("{byte:02X}")).monospace();
                        if address == value {
                            text = text.color(pointed).strong().underline();
                        }
                        ui.label(text);
                    }
                    ui.end_row();
                }
            });
        ui.add_space(4.0);
    }
}
