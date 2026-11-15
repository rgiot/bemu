//! The chips' windows: what the registers mean, decoded (as WinAPE, ACE and MAME show them),
//! over the raw values `DebugServer::inspect` serves the API.

use crate::commands::{Arg, Command, CommandBus};
use crate::ga::RgbLevels;
use crate::motherboard::Motherboard;

/// A colour of the CPC's levels (0-100 each).
pub fn rgb_color(levels: RgbLevels) -> egui::Color32 {
    color(levels)
}

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

/// The Gate Array: its mode (changed with a command), its pens (each opens the 27 inks), its
/// registers. A CPC+ has the ASIC's instead: [`asic`].
pub fn gate_array(ui: &mut egui::Ui, mb: &Motherboard, bus: &mut CommandBus) {
    ga_mode(ui, mb, bus);
    ga_inks(ui, mb, bus);
    ga_state(ui, mb);
}

/// The chip and the screen mode, a button per mode (`ga-set-mode`, from the next line).
fn ga_mode(ui: &mut egui::Ui, mb: &Motherboard, bus: &mut CommandBus) {
    let ga = mb.ga();
    let mode = |mode: crate::ga::VideoMode| mode as u8;
    ui.horizontal(|ui| {
        ui.label(format!(
            "{}: mode",
            match (ga.plus(), ga.gate_array_type()) {
                (Some(_), _) => "ASIC",
                (None, crate::ga::GateArrayType::Ga40010) => "40010",
                _ => "40007/40008",
            }
        ));
        let active = mode(ga.active_video_mode());
        for wanted in 0..4u8 {
            if ui
                .selectable_label(active == wanted, wanted.to_string())
                .on_hover_text(format!("Mode {wanted} from the next line"))
                .clicked()
                && active != wanted
            {
                if let Ok(command) = Command::with_arg("ga-set-mode", Arg::Text(wanted.to_string()))
                {
                    bus.push(command);
                }
            }
        }
        if mode(ga.next_video_mode()) != active {
            ui.weak(format!("(next line: mode {})", mode(ga.next_video_mode())));
        }
    });
}

/// The 16 pens and the border, each a swatch that opens the 27 inks (`ga-set-ink`).
fn ga_inks(ui: &mut egui::Ui, mb: &Motherboard, bus: &mut CommandBus) {
    use super::widgets::{ink_picker, Picked};
    let ga = mb.ga();
    let palette = ga.palette();
    let pen = ga.selected_pen();
    ui.weak("A click on a pen gives it another ink, until the program writes it again")
        .on_hover_text("The firmware sends its own inks again at each flash period: a change lasts under BASIC only while the firmware is stopped");
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
                    let tooltip = format!(
                        "pen {index}: ink {} (hardware &{:02X})",
                        ink.number(),
                        ink.hardware_number()
                    );
                    let id = egui::Id::new(("ga-pen", index));
                    if let Some(Picked::Ink(new)) = ink_picker(
                        ui,
                        id,
                        ink.number(),
                        usize::from(pen) == index,
                        false,
                        &tooltip,
                    ) {
                        let pen = if index == 16 {
                            "B".to_owned()
                        } else {
                            index.to_string()
                        };
                        if let Ok(command) =
                            Command::with_arg("ga-set-ink", Arg::Text(format!("{pen} {new}")))
                        {
                            bus.push(command);
                        }
                    }
                });
                if index == 8 {
                    ui.end_row();
                }
            }
        });
}

/// The selected pen, RMR decoded, the upper ROM, the interrupt counter.
fn ga_state(ui: &mut egui::Ui, mb: &Motherboard) {
    let ga = mb.ga();
    let pen = ga.selected_pen();
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
    super::widgets::rule(ui);
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
    super::widgets::rule(ui);
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

/// The ASIC of a CPC+ (the Gate Array's part, then its own): the mode and the Gate Array's
/// registers, the 32 palette entries (12 bits: pens, border, sprite colours), the raster
/// interrupt, the split, the soft scroll, RMR2, the DMA channels, the hardware sprites.
pub fn asic(ui: &mut egui::Ui, mb: &Motherboard, bus: &mut CommandBus) {
    let (Some(asic), Some(video)) = (mb.asic(), mb.ga().plus()) else {
        ui.weak("a CPC+ has one (Machine > Platform)");
        return;
    };
    ga_mode(ui, mb, bus);
    // the palette: pens 0-15, the border, the sprite colours 1-15
    ui.strong("Palette (12 bits: &GRB)");
    let pen = usize::from(mb.ga().selected_pen());
    egui::Grid::new("asic-palette")
        .spacing([4.0, 6.0])
        .show(ui, |ui| {
            for index in 0..32 {
                let entry = video.palette_entry(index);
                ui.vertical_centered(|ui| {
                    ui.small(match index {
                        16 => "B".to_owned(),
                        17.. => format!("S{}", index - 16),
                        _ => index.to_string(),
                    });
                    swatch(ui, color(video.rgb(index)), index == pen).on_hover_text(format!(
                        "{}: &{entry:03X} (green {}, red {}, blue {})",
                        match index {
                            16 => "border".to_owned(),
                            17.. => format!("sprite colour {}", index - 16),
                            _ => format!("pen {index}"),
                        },
                        (entry >> 8) & 15,
                        (entry >> 4) & 15,
                        entry & 15
                    ));
                });
                // the pens, then the border and the sprite colours
                if index == 15 {
                    ui.end_row();
                }
            }
        });
    ga_state(ui, mb);
    super::widgets::rule(ui);
    let rmr2 = video.rmr2();
    let sscr = video.sscr();
    egui::Grid::new("asic").striped(true).show(ui, |ui| {
        ui.label("Registers");
        ui.monospace(if asic.unlocked() {
            "unlocked"
        } else {
            "locked"
        });
        ui.end_row();
        ui.label("Raster interrupt (PRI)");
        ui.monospace(match asic.pri() {
            0 => "off (the Gate Array's interrupts)".to_owned(),
            line => format!("line {line}"),
        });
        ui.end_row();
        ui.label("Split (SPLT, SSA)");
        ui.monospace(match asic.split() {
            Some((line, address)) => format!("after line {line}, from &{address:04X}"),
            None => "no".to_owned(),
        });
        ui.end_row();
        ui.label("Soft scroll (SSCR)");
        ui.monospace(format!(
            "&{sscr:02X}: {} pixels right, {} lines down{}",
            sscr & 15,
            (sscr >> 4) & 7,
            if sscr & 0x80 != 0 {
                ", border over the first byte"
            } else {
                ""
            }
        ));
        ui.end_row();
        ui.label("RMR2");
        ui.monospace(format!(
            "&{rmr2:02X}: cartridge page {} {}",
            rmr2 & 7,
            match (rmr2 >> 3) & 3 {
                0 => "at &0000",
                1 => "at &4000",
                2 => "at &8000",
                _ => "at &0000, the registers at &4000",
            }
        ));
        ui.end_row();
        ui.label("Interrupt vector (IVR)");
        ui.monospace(format!(
            "&{:02X}{}",
            asic.ivr() & 0xf8,
            if asic.ivr() & 1 != 0 {
                ", DMA interrupts cleared by the acknowledge"
            } else {
                ""
            }
        ));
        ui.end_row();
    });
    super::widgets::rule(ui);
    ui.strong("DMA (sound)");
    egui::Grid::new("asic-dma").striped(true).show(ui, |ui| {
        for heading in [
            "",
            "On",
            "Address",
            "Prescaler",
            "Pause",
            "Repeat",
            "Interrupt",
        ] {
            ui.weak(heading);
        }
        ui.end_row();
        for (n, channel) in asic.dma_channels().iter().enumerate() {
            ui.monospace(n.to_string());
            ui.monospace(yes(channel.enabled));
            ui.monospace(format!("&{:04X}", channel.address));
            ui.monospace(channel.prescaler.to_string());
            ui.monospace(channel.pause.to_string());
            ui.monospace(channel.loop_count.to_string());
            ui.monospace(yes(channel.interrupt));
            ui.end_row();
        }
    });
    super::widgets::rule(ui);
    ui.strong("Hardware sprites");
    asic_sprites(ui, video);
}

/// The 16 hardware sprites: each drawn in its colours (a transparent pixel checked), its
/// position and magnification; a hidden one (magnification 0) greyed.
fn asic_sprites(ui: &mut egui::Ui, video: &crate::asic::AsicVideo) {
    const PIXEL: f32 = 3.0;
    let zoom = |z: u8| match z {
        0 => "-".to_owned(),
        z => format!("x{}", 1 << (z - 1)),
    };
    egui::Grid::new("asic-sprites")
        .spacing([10.0, 8.0])
        .show(ui, |ui| {
            for n in 0..16 {
                let sprite = video.sprite(n);
                ui.vertical(|ui| {
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(16.0 * PIXEL, 16.0 * PIXEL),
                        egui::Sense::hover(),
                    );
                    let painter = ui.painter_at(rect);
                    for (i, &pen) in sprite.pixels.iter().enumerate() {
                        let (x, y) = ((i % 16) as f32, (i / 16) as f32);
                        let cell = egui::Rect::from_min_size(
                            rect.min + egui::vec2(x * PIXEL, y * PIXEL),
                            egui::vec2(PIXEL, PIXEL),
                        );
                        let fill = if pen == 0 {
                            // transparent: a checker
                            if (i % 16 + i / 16) % 2 == 0 {
                                egui::Color32::from_gray(40)
                            } else {
                                egui::Color32::from_gray(60)
                            }
                        } else {
                            color(video.rgb(16 + usize::from(pen & 15)))
                        };
                        painter.rect_filled(
                            cell,
                            0.0,
                            if sprite.shown() {
                                fill
                            } else {
                                fill.gamma_multiply(0.35)
                            },
                        );
                    }
                    response.on_hover_text(format!(
                        "sprite {n}: x {}, y {}, magnification {} x {}{}",
                        sprite.x,
                        sprite.y,
                        zoom(sprite.zoom_x),
                        zoom(sprite.zoom_y),
                        if sprite.shown() { "" } else { " (hidden)" }
                    ));
                    ui.small(format!(
                        "{n}: {},{} {}",
                        sprite.x,
                        sprite.y,
                        if sprite.shown() {
                            format!("{}{}", zoom(sprite.zoom_x), zoom(sprite.zoom_y))
                        } else {
                            "off".to_owned()
                        }
                    ));
                });
                if n % 8 == 7 {
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

/// The format a track's sector IDs name: AMSDOS's Data (&C1-&C9), System (&41-&49) or IBM
/// (&01-&08), else its sectors and their IDs ("10 sectors &C1-&CA").
fn format_name(track: &crate::disc_image::Track) -> String {
    let ids: Vec<u8> = track.sectors.iter().map(|s| s.r).collect();
    let all = |range: std::ops::RangeInclusive<u8>| {
        ids.len() == range.clone().count() && ids.iter().all(|r| range.contains(r))
    };
    if all(0xc1..=0xc9) {
        "Data".to_owned()
    } else if all(0x41..=0x49) {
        "System".to_owned()
    } else if all(0x01..=0x08) {
        "IBM".to_owned()
    } else if let (Some(first), Some(last)) = (ids.iter().min(), ids.iter().max()) {
        format!("{} sectors &{first:02X}-&{last:02X}", ids.len())
    } else {
        "no sector".to_owned()
    }
}

/// A track in a word: unformatted, the standard 9 sectors of 512 bytes, or another layout (a
/// protection, a more packed format), weak sectors (several copies of their data) above all.
fn track_kind(track: Option<&crate::disc_image::Track>) -> (&'static str, egui::Color32) {
    match track {
        None => ("unformatted", egui::Color32::from_gray(45)),
        Some(track) if track.sectors.iter().any(|s| s.copies() > 1) => {
            ("weak sectors", egui::Color32::from_rgb(200, 60, 60))
        }
        Some(track) if track.sectors.len() == 9 && track.sectors.iter().all(|s| s.n == 2) => {
            ("9 x 512", egui::Color32::from_rgb(60, 150, 70))
        }
        Some(track) if track.sectors.is_empty() => ("no sector", egui::Color32::from_gray(80)),
        Some(_) => ("another layout", egui::Color32::from_rgb(210, 150, 40)),
    }
}

/// The disc in a drive as the head sees it: its format, a map of the tracks of the side read
/// (the head on it), the sectors of the track under the head (the next one marked), its
/// catalogue.
fn disc_details(ui: &mut egui::Ui, mb: &Motherboard, unit: usize) {
    use crate::fdc::FdcDrive;
    let drive = if unit == 0 { FdcDrive::A } else { FdcDrive::B };
    let fdc = mb.fdc();
    let Some(disc) = fdc.image(drive) else {
        return;
    };
    let side = fdc.disc_side(drive).min(disc.nb_heads().saturating_sub(1));
    let head_track = fdc.track(unit);
    let under = disc.track(side, head_track);
    ui.strong(format!(
        "Drive {}: {} {}, {} tracks, {}, side {} read",
        ["A", "B"][unit],
        if disc.is_bitstream() { "HFE" } else { "DSK" },
        under.map_or("unformatted".to_owned(), format_name),
        disc.nb_tracks(),
        if disc.is_double_sided() {
            "double sided"
        } else {
            "single sided"
        },
        ["A", "B"][usize::from(side & 1)]
    ));
    // the tracks of the side read, the head's outlined
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        for track in 0..disc.nb_tracks() {
            let image = disc.track(side, track);
            let (kind, fill) = track_kind(image);
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(7.0, 18.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 1.0, fill);
            if track == head_track {
                ui.painter().rect_stroke(
                    rect.expand(1.0),
                    1.0,
                    egui::Stroke::new(2.0_f32, egui::Color32::WHITE),
                    egui::StrokeKind::Outside,
                );
            }
            response.on_hover_text(match image {
                Some(t) => format!(
                    "track {track}: {kind}, {} sector(s): {}",
                    t.sectors.len(),
                    t.sectors
                        .iter()
                        .map(|s| format!("&{:02X}", s.r))
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
                None => format!("track {track}: unformatted"),
            });
        }
    });
    ui.horizontal(|ui| {
        for (kind, fill) in [
            track_kind(None),
            ("9 x 512", egui::Color32::from_rgb(60, 150, 70)),
            ("another layout", egui::Color32::from_rgb(210, 150, 40)),
            ("weak sectors", egui::Color32::from_rgb(200, 60, 60)),
        ] {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 1.0, fill);
            ui.weak(kind);
        }
        ui.weak(format!("· head over track {head_track}"));
    });
    // the sectors under the head, in their order on the track
    match under {
        Some(track) if !track.sectors.is_empty() => {
            let next = fdc.sector_under_head(unit) % track.sectors.len();
            egui::Grid::new(("fdc-sectors", unit))
                .striped(true)
                .show(ui, |ui| {
                    for heading in ["", "C", "H", "R", "N", "Size", "Data", "Notes"] {
                        ui.weak(heading);
                    }
                    ui.end_row();
                    for (index, sector) in track.sectors.iter().enumerate() {
                        ui.monospace(if index == next { "▶" } else { " " })
                            .on_hover_text(if index == next {
                                "The next sector under the head"
                            } else {
                                ""
                            });
                        ui.monospace(format!("&{:02X}", sector.c));
                        ui.monospace(format!("&{:02X}", sector.h));
                        ui.monospace(format!("&{:02X}", sector.r));
                        ui.monospace(format!("{}", sector.n));
                        ui.monospace(format!("{}", sector.nominal_len()));
                        ui.monospace(format!("{}", sector.data.len()));
                        let mut notes = Vec::new();
                        if sector.copies() > 1 {
                            notes.push(format!("weak: {} copies", sector.copies()));
                        }
                        if sector.is_deleted() {
                            notes.push("deleted".to_owned());
                        }
                        if sector.c != head_track {
                            notes.push(format!("ID of track {}", sector.c));
                        }
                        if sector.data.len() != sector.nominal_len() && sector.copies() <= 1 {
                            notes.push("odd size".to_owned());
                        }
                        ui.weak(notes.join(", "));
                        ui.end_row();
                    }
                });
        }
        _ => {
            ui.weak(format!("track {head_track}: unformatted"));
        }
    }
    let catalogue = disc.amsdos_catalog();
    if !catalogue.is_empty() {
        ui.collapsing(format!("Catalogue ({} entries)", catalogue.len()), |ui| {
            egui::Grid::new(("fdc-catalogue", unit))
                .striped(true)
                .show(ui, |ui| {
                    for entry in catalogue.iter().take(64) {
                        ui.monospace(format!("{}.{}", entry.name.trim(), entry.extension.trim()));
                        ui.weak(format!(
                            "user {}{}, {}K",
                            entry.user,
                            if entry.hidden { ", hidden" } else { "" },
                            (u32::from(entry.records) * 128).div_ceil(1024)
                        ));
                        ui.end_row();
                    }
                });
        });
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
    super::widgets::rule(ui);
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
    // each disc as the head sees it
    for unit in 0..2 {
        if fdc.has_disk(if unit == 0 { FdcDrive::A } else { FdcDrive::B }) {
            super::widgets::rule(ui);
            disc_details(ui, mb, unit);
        }
    }
    super::widgets::rule(ui);
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
                    format!("= {}", hex(&record.result))
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

/// The memory map: the CPU's paging (the paging form's map), every 16K block of RAM and where
/// it is paged now, the ROMs in their slots (named from their header).
pub fn memory_map(ui: &mut egui::Ui, mb: &Motherboard) {
    use crate::memory::PhysicalAddress;
    super::paging::map(ui, mb, &super::paging::Paging::default());
    if let Some(banks) = mb.large_ram_expansion_bank_count() {
        ui.label(format!("4M card: {banks} pages of 64K"));
    }
    ui.add_space(6.0);
    // each block of RAM: where the CPU reads or writes it now
    let mut read_at: Vec<Vec<u16>> = vec![Vec::new(); usize::from(mb.physical_bank_count())];
    let mut written_at: Vec<Vec<u16>> = read_at.clone();
    for block in 0..4u16 {
        let base = block * 0x4000;
        if let PhysicalAddress::Ram { bank, .. } = mb.physical_address(base) {
            if let Some(at) = read_at.get_mut(usize::from(bank)) {
                at.push(base);
            }
        }
        if let PhysicalAddress::Ram { bank, .. } = mb.physical_address_for_write(base) {
            if let Some(at) = written_at.get_mut(usize::from(bank)) {
                at.push(base);
            }
        }
    }
    ui.strong(format!(
        "RAM: {}K",
        16 * u32::from(mb.physical_bank_count())
    ));
    egui::Grid::new("memmap-banks")
        .spacing([4.0, 4.0])
        .show(ui, |ui| {
            for (bank, (read, written)) in read_at.iter().zip(&written_at).enumerate() {
                let bank = bank as u16;
                let paged = !read.is_empty() || !written.is_empty();
                let fill = if !read.is_empty() {
                    egui::Color32::from_rgb(60, 150, 70)
                } else if !written.is_empty() {
                    egui::Color32::from_rgb(40, 90, 50)
                } else {
                    ui.visuals().extreme_bg_color
                };
                let at = |addresses: &[u16]| {
                    addresses
                        .iter()
                        .map(|a| format!("&{a:04X}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                };
                let tooltip = if !read.is_empty() {
                    format!("read and written at {}", at(read))
                } else if !written.is_empty() {
                    format!("written at {} (a ROM is read there)", at(written))
                } else {
                    "not paged".to_owned()
                };
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(64.0, 30.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 2.0, fill);
                ui.painter().text(
                    rect.center_top() + egui::vec2(0.0, 2.0),
                    egui::Align2::CENTER_TOP,
                    super::paging::bank_name(bank),
                    egui::FontId::proportional(12.0),
                    if paged {
                        egui::Color32::WHITE
                    } else {
                        ui.visuals().weak_text_color()
                    },
                );
                if paged {
                    ui.painter().text(
                        rect.center_bottom() - egui::vec2(0.0, 2.0),
                        egui::Align2::CENTER_BOTTOM,
                        at(if read.is_empty() { written } else { read }),
                        egui::FontId::monospace(10.0),
                        egui::Color32::WHITE,
                    );
                }
                response.on_hover_text(format!("{}: {tooltip}", super::paging::bank_name(bank)));
                // a 64K page a row: the main 64K, then each expansion page
                if bank % 4 == 3 {
                    ui.end_row();
                }
            }
        });
    ui.add_space(6.0);
    // a CPC+: its cartridge (the system one, or one inserted), each page by its ROM name
    if let Some(cartridge) = mb.any_cartridge() {
        let system = mb.cartridge().is_none();
        ui.strong(format!(
            "Cartridge: {}, {} pages of 16K",
            if system { "the system one" } else { "inserted" },
            cartridge.page_count()
        ));
        let lower = mb.ga().plus().map(|plus| plus.rmr2() & 7);
        let upper = mb.cartridge_page_of_upper_rom(mb.upper_rom_number());
        // the pages 8 a row, the lower and upper ROMs' lit; a page's name on hover
        egui::Grid::new("memmap-cartridge")
            .spacing([4.0, 4.0])
            .show(ui, |ui| {
                for page in 0..cartridge.page_count().min(32) as u8 {
                    let role = match (lower == Some(page), upper == Some(page)) {
                        (true, true) => "lower and upper ROM",
                        (true, false) => "lower ROM",
                        (false, true) => "upper ROM",
                        _ => "",
                    };
                    let fill = if role.is_empty() {
                        ui.visuals().extreme_bg_color
                    } else {
                        egui::Color32::from_rgb(200, 70, 50)
                    };
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(34.0, 20.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 2.0, fill);
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        page.to_string(),
                        egui::FontId::monospace(12.0),
                        if role.is_empty() {
                            ui.visuals().weak_text_color()
                        } else {
                            egui::Color32::WHITE
                        },
                    );
                    let bytes: Vec<u8> = (0..0x400u16)
                        .filter_map(|offset| mb.read_cartridge_byte(page, offset))
                        .collect();
                    let name = rom_name(&bytes)
                        .unwrap_or_else(|| super::paging::cartridge_page_name(mb, page));
                    response.on_hover_text(if role.is_empty() {
                        format!("page {page}: {name}")
                    } else {
                        format!("page {page}: {name} ({role})")
                    });
                    if page % 8 == 7 {
                        ui.end_row();
                    }
                }
            });
    }
    ui.strong("ROMs");
    egui::Grid::new("memmap-roms").striped(true).show(ui, |ui| {
        let selected = mb.upper_rom_number();
        for slot in 0..=255u8 {
            let Some(rom) = mb.upper_rom(slot) else {
                continue;
            };
            ui.monospace(format!("{slot:3}"));
            ui.monospace(rom_name(rom).unwrap_or_else(|| "(no name)".to_owned()));
            if slot == selected {
                ui.weak("selected");
            }
            ui.end_row();
        }
    });
}

/// A ROM's name: the first name of its command table (its header: type, mark, version,
/// revision, then the table's address; each name ends with bit 7 set on its last letter).
pub fn rom_name(rom: &[u8]) -> Option<String> {
    let table = usize::from(u16::from_le_bytes([*rom.get(4)?, *rom.get(5)?]));
    let offset = table.checked_sub(0xc000)?;
    let mut name = String::new();
    for &byte in rom.get(offset..)?.iter().take(16) {
        let c = byte & 0x7f;
        if !(0x20..0x7f).contains(&c) {
            return None;
        }
        name.push(c as char);
        if byte & 0x80 != 0 {
            return Some(format!(
                "{} {}.{}.{}",
                name.trim(),
                rom.get(1)?,
                rom.get(2)?,
                rom.get(3)?
            ));
        }
    }
    None
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

/// The Z80's registers, editable (each change a `z80-set-register` command): the flags first,
/// each a check box; then each pair beside its shadow (AF / AF'...), the index registers, SP and
/// PC, I and R.
pub fn registers(ui: &mut egui::Ui, mb: &Motherboard, bus: &mut CommandBus) {
    use crate::z80::{HasValue, Reg16, Reg8};
    let cpu = mb.cpu();
    // the flags, each a check box: F set again with that bit changed
    let af = cpu.reg16(Reg16::Af).value();
    let f = af as u8;
    ui.horizontal(|ui| {
        ui.strong("Flags");
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
    ui.add_space(4.0);
    // a 16-bit register to edit: its name, its field, its value in decimal
    let word = |ui: &mut egui::Ui, bus: &mut CommandBus, name: &str, value: u16| {
        ui.monospace(name);
        let mut edited = u32::from(value);
        let response = ui.add(
            egui::DragValue::new(&mut edited)
                .range(0..=0xffff)
                .hexadecimal(4, false, true)
                .prefix("&")
                .speed(1.0),
        );
        if response.changed() && edited != u32::from(value) {
            run(
                bus,
                "z80-set-register",
                Arg::Text(format!("{name}={edited}")),
            );
        }
        ui.weak(format!("{value:5}"));
    };
    egui::Grid::new("z80-registers")
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            // each pair on the row of its shadow
            for (name, reg, shadow, shadow_reg) in [
                ("AF", Reg16::Af, "AF'", Reg16::Af2),
                ("BC", Reg16::Bc, "BC'", Reg16::Bc2),
                ("DE", Reg16::De, "DE'", Reg16::De2),
                ("HL", Reg16::Hl, "HL'", Reg16::Hl2),
            ] {
                word(ui, bus, name, cpu.reg16(reg).value());
                word(ui, bus, shadow, cpu.reg16(shadow_reg).value());
                ui.end_row();
            }
            word(ui, bus, "IX", cpu.reg16(Reg16::Ix).value());
            word(ui, bus, "IY", cpu.reg16(Reg16::Iy).value());
            ui.end_row();
            word(ui, bus, "SP", cpu.reg16(Reg16::Sp).value());
            word(ui, bus, "PC", cpu.reg16(Reg16::Pc).value());
            ui.end_row();
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
                ui.weak(format!("{value:5}"));
            }
            ui.end_row();
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
    fn a_track_is_named_by_its_sectors() {
        use crate::disc_image::{Sector, Track};
        let track = |ids: &[u8], n: u8| Track {
            sectors: ids.iter().map(|&r| Sector::new(0, 0, r, n, 0xe5)).collect(),
            track: 0,
            head: 0,
            gap3: 0x4e,
            filler: 0xe5,
        };
        let data = track(&[0xc1, 0xc6, 0xc2, 0xc7, 0xc3, 0xc8, 0xc4, 0xc9, 0xc5], 2);
        assert_eq!(format_name(&data), "Data");
        assert_eq!(track_kind(Some(&data)).0, "9 x 512");
        let packed = track(
            &[0xc1, 0xc6, 0xc2, 0xc7, 0xc3, 0xc8, 0xc4, 0xc9, 0xc5, 0xca],
            2,
        );
        assert_eq!(format_name(&packed), "10 sectors &C1-&CA");
        assert_eq!(track_kind(Some(&packed)).0, "another layout");
        assert_eq!(track_kind(None).0, "unformatted");
    }

    #[test]
    fn a_rom_is_named_from_its_header() {
        // a background ROM 1.2.3, its table at &C010, its first name "MYROM"
        let mut rom = vec![0u8; 0x40];
        rom[..6].copy_from_slice(&[1, 1, 2, 3, 0x10, 0xc0]);
        rom[0x10..0x15].copy_from_slice(b"MYRO\xcd");
        assert_eq!(rom_name(&rom).as_deref(), Some("MYROM 1.2.3"));
        // no table: no name
        assert_eq!(rom_name(&[0; 8]), None);
    }

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
    let mb = target.motherboard();
    let settings = target.settings();
    const GREEN: egui::Color32 = egui::Color32::from_rgb(60, 220, 90);
    const RED: egui::Color32 = egui::Color32::from_rgb(230, 60, 60);
    ui.horizontal(|ui| {
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
            super::widgets::rule(ui);
        }
        if let Some(recorder) = mb.tape_recorder() {
            led(ui, motor, RED);
            ui.label(format!("REC {:.1} s", recorder.seconds()));
            super::widgets::rule(ui);
        }
        // the machine
        let crtc = mb.crtc();
        let ram = mb.total_ram_kb();
        let ga = match (mb.ga().plus(), mb.ga().gate_array_type()) {
            (Some(_), _) => "ASIC",
            (None, crate::ga::GateArrayType::Ga40010) => "GA 40010",
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
        // on the right, from the right edge: the drives, then how it runs
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            drive_leds(ui, target);
            super::widgets::rule(ui);
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
                super::widgets::rule(ui);
                ui.label(format!("debug server :{port}"));
            }
        });
    });
}

/// The drives, right to left (B at the edge): the LED lit while the motor turns for the drive
/// addressed, blinking a moment when its head changes track; the disc's name (bold and `*` once
/// written to).
fn drive_leds(ui: &mut egui::Ui, target: &dyn crate::commands::CommandTarget) {
    use crate::fdc::FdcDrive;
    const GREEN: egui::Color32 = egui::Color32::from_rgb(60, 220, 90);
    /// How long the LED blinks after a step.
    const BLINK_S: f64 = 0.25;
    let mb = target.motherboard();
    let settings = target.settings();
    let fdc = mb.fdc();
    let now = ui.input(|i| i.time);
    // each drive's track and when it last changed
    let id = egui::Id::new("status-drive-tracks");
    let mut tracks: [(u8, f64); 2] = ui.data(|d| d.get_temp(id)).unwrap_or([
        (fdc.track(0), f64::NEG_INFINITY),
        (fdc.track(1), f64::NEG_INFINITY),
    ]);
    for (unit, drive) in [(1usize, FdcDrive::B), (0, FdcDrive::A)] {
        let track = fdc.track(unit);
        if track != tracks[unit].0 {
            tracks[unit] = (track, now);
        }
        let stepping = now - tracks[unit].1 < BLINK_S;
        let motor = fdc.motor() && fdc.selected_drive() == unit;
        // blinking: 20 half-periods a second
        let lit = if stepping {
            (now * 20.0) as i64 % 2 == 0
        } else {
            motor
        };
        if stepping {
            ui.ctx().request_repaint();
        }
        let disc = settings
            .and_then(|s| s.media.discs[unit].as_deref())
            .map(short_name);
        let text = match (fdc.has_disk(drive), disc) {
            (true, Some(name)) => name.to_owned(),
            (true, None) => "disc".to_owned(),
            (false, _) => "empty".to_owned(),
        };
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
            track,
            if fdc.is_modified(drive) {
                ", written to (not saved)"
            } else {
                ""
            }
        ));
        led(ui, lit, GREEN).on_hover_text(if stepping {
            "the head changes track"
        } else if motor {
            "the motor turns"
        } else {
            "idle"
        });
        if unit == 1 {
            ui.add_space(6.0);
        }
    }
    ui.data_mut(|d| d.insert_temp(id, tracks));
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

/// The beam in the frame as the CRTC counts it: the frame's lines and microseconds, the
/// displayed area (R1 × R6 characters), the HSYNC column and the VSYNC band, the beam's
/// crosshair (the paused picture's magenta), and where it is in numbers.
pub fn beam_map(ui: &mut egui::Ui, mb: &Motherboard) {
    const X: f32 = 4.0; // pixels per µs
    const Y: f32 = 1.0; // pixels per line
    let crtc = mb.crtc();
    let r = crtc.registers();
    let c = crtc.counters();
    let char_lines = u32::from(r[9] & 0x1f) + 1;
    let width = u32::from(r[0]) + 1;
    let lines = ((u32::from(r[4] & 0x7f) + 1) * char_lines + u32::from(r[5] & 0x1f)).max(1);
    let line = if c.in_vertical_adjust {
        (u32::from(r[4] & 0x7f) + 1) * char_lines + u32::from(c.vtac)
    } else {
        u32::from(c.vcc) * char_lines + u32::from(c.vlc)
    };
    let x = u32::from(c.hcc);
    ui.monospace(format!(
        "line {line} / {lines} (row C4 {}, raster C9 {}{}), µs {x} / {width} (C0)",
        c.vcc,
        c.vlc,
        if c.in_vertical_adjust {
            format!(", adjust {}", c.vtac)
        } else {
            String::new()
        }
    ));
    ui.monospace(format!(
        "{} NOPs since the frame started, {} to its end",
        line * width + x,
        (lines * width).saturating_sub(line * width + x)
    ));
    // the drawing: the whole frame at most 128 µs × 512 lines
    let shown_w = width.min(128) as f32 * X;
    let shown_h = lines.min(512) as f32 * Y;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(shown_w, shown_h), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let at = |us: u32, line: u32| {
        rect.left_top() + egui::vec2(us.min(128) as f32 * X, line.min(512) as f32 * Y)
    };
    painter.rect_filled(rect, 0.0, egui::Color32::from_gray(30));
    // the displayed characters
    let shown = egui::Rect::from_min_max(
        at(0, 0),
        at(
            u32::from(r[1]).min(width),
            (u32::from(r[6] & 0x7f) * char_lines).min(lines),
        ),
    );
    painter.rect_filled(shown, 0.0, egui::Color32::from_rgb(30, 50, 110));
    // HSYNC: from R2, R3's low nibble µs (0 none)
    let hsync_w = u32::from(r[3] & 0x0f);
    if hsync_w > 0 {
        painter.rect_filled(
            egui::Rect::from_min_max(at(u32::from(r[2]), 0), at(u32::from(r[2]) + hsync_w, lines)),
            0.0,
            egui::Color32::from_rgb(90, 70, 20),
        );
    }
    // VSYNC: from row R7, R3's high nibble lines (0: 16)
    let vsync_lines = match u32::from(r[3] >> 4) {
        0 => 16,
        n => n,
    };
    let vsync_top = u32::from(r[7] & 0x7f) * char_lines;
    painter.rect_filled(
        egui::Rect::from_min_max(at(0, vsync_top), at(width, vsync_top + vsync_lines)),
        0.0,
        egui::Color32::from_rgb(110, 40, 40),
    );
    // the beam
    let magenta = egui::Color32::from_rgba_unmultiplied(255, 0, 255, 204);
    let p = at(x, line) + egui::vec2(X / 2.0, 0.0);
    painter.line_segment(
        [egui::pos2(p.x, rect.top()), egui::pos2(p.x, rect.bottom())],
        egui::Stroke::new(1.0_f32, magenta),
    );
    painter.line_segment(
        [egui::pos2(rect.left(), p.y), egui::pos2(rect.right(), p.y)],
        egui::Stroke::new(1.0_f32, magenta),
    );
    painter.rect_filled(
        egui::Rect::from_center_size(p, egui::vec2(6.0, 5.0)),
        0.0,
        magenta,
    );
    // a point of the frame under the pointer
    if let Some(pointer) = response.hover_pos() {
        let us = ((pointer.x - rect.left()) / X) as u32;
        let at_line = ((pointer.y - rect.top()) / Y) as u32;
        response.on_hover_text(format!(
            "line {at_line} (row {}, raster {}), µs {us}: NOP {} of the frame",
            at_line / char_lines,
            at_line % char_lines,
            at_line * width + us
        ));
    }
    ui.horizontal(|ui| {
        for (colour, name) in [
            (egui::Color32::from_rgb(30, 50, 110), "displayed (R1 × R6)"),
            (egui::Color32::from_rgb(90, 70, 20), "HSYNC (R2, R3)"),
            (egui::Color32::from_rgb(110, 40, 40), "VSYNC (R7, R3)"),
        ] {
            let (swatch, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
            ui.painter().rect_filled(swatch, 2.0, colour);
            ui.weak(name);
        }
    });
}
