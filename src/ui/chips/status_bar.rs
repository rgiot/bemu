//! The status bar: the machine, its speed, the drives' LEDs.

use super::*;

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
            crate::ui::widgets::rule(ui);
        }
        if let Some(recorder) = mb.tape_recorder() {
            led(ui, motor, RED);
            ui.label(format!("REC {:.1} s", recorder.seconds()));
            crate::ui::widgets::rule(ui);
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
            crate::ui::widgets::rule(ui);
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
                crate::ui::widgets::rule(ui);
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
