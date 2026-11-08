//! The chips' windows: what the registers mean, decoded (as WinAPE, ACE and MAME show them),
//! over the raw values `DebugServer::inspect` serves the API.

use crate::commands::CommandBus;
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
