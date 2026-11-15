//! The FDC window: its status, its drives and the discs as the head sees them.

use super::*;

/// A track kind's colour in the map.
fn kind_colour(kind: crate::state::TrackKind) -> egui::Color32 {
    use crate::state::TrackKind::*;
    match kind {
        Unformatted => egui::Color32::from_gray(45),
        WeakSectors => egui::Color32::from_rgb(200, 60, 60),
        Standard => egui::Color32::from_rgb(60, 150, 70),
        NoSector => egui::Color32::from_gray(80),
        AnotherLayout => egui::Color32::from_rgb(210, 150, 40),
    }
}

/// The disc in a drive as the head sees it (the same as /api/fdc gives): its format, a map of
/// the tracks of the side read (the head on it), the sectors of the track under the head (the
/// next one marked), its catalogue.
fn disc_details(ui: &mut egui::Ui, mb: &Motherboard, unit: usize) {
    use crate::state::TrackKind;
    let Some(disc) = crate::state::DiscState::of(mb, unit) else {
        return;
    };
    let head_track = disc.head_track;
    ui.strong(format!(
        "Drive {}: {} {}, {} tracks, {}, side {} read",
        ["A", "B"][unit],
        disc.image,
        disc.format,
        disc.tracks,
        if disc.double_sided {
            "double sided"
        } else {
            "single sided"
        },
        disc.side
    ));
    // the tracks of the side read, the head's outlined
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        for (track, (kind, ids)) in disc.track_kinds.iter().zip(&disc.track_sectors).enumerate() {
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(7.0, 18.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 1.0, kind_colour(*kind));
            if track == usize::from(head_track) {
                ui.painter().rect_stroke(
                    rect.expand(1.0),
                    1.0,
                    egui::Stroke::new(2.0_f32, egui::Color32::WHITE),
                    egui::StrokeKind::Outside,
                );
            }
            response.on_hover_text(if *kind == TrackKind::Unformatted {
                format!("track {track}: unformatted")
            } else {
                format!(
                    "track {track}: {}, {} sector(s): {}",
                    kind.label(),
                    ids.len(),
                    ids.iter()
                        .map(|r| format!("&{r:02X}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                )
            });
        }
    });
    ui.horizontal(|ui| {
        for kind in [
            TrackKind::Unformatted,
            TrackKind::Standard,
            TrackKind::AnotherLayout,
            TrackKind::WeakSectors,
        ] {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 1.0, kind_colour(kind));
            ui.weak(kind.label());
        }
        ui.weak(format!("· head over track {head_track}"));
    });
    // the sectors under the head, in their order on the track
    if disc.sectors.is_empty() {
        ui.weak(format!("track {head_track}: unformatted"));
    } else {
        egui::Grid::new(("fdc-sectors", unit))
            .striped(true)
            .show(ui, |ui| {
                for heading in ["", "C", "H", "R", "N", "Size", "Data", "Notes"] {
                    ui.weak(heading);
                }
                ui.end_row();
                for (index, sector) in disc.sectors.iter().enumerate() {
                    let next = disc.next_sector == Some(index);
                    ui.monospace(if next { "▶" } else { " " })
                        .on_hover_text(if next {
                            "The next sector under the head"
                        } else {
                            ""
                        });
                    ui.monospace(format!("&{:02X}", sector.c));
                    ui.monospace(format!("&{:02X}", sector.h));
                    ui.monospace(format!("&{:02X}", sector.r));
                    ui.monospace(format!("{}", sector.n));
                    ui.monospace(format!("{}", sector.size));
                    ui.monospace(format!("{}", sector.data_len));
                    ui.weak(sector.notes.join(", "));
                    ui.end_row();
                }
            });
    }
    if !disc.catalogue.is_empty() {
        ui.collapsing(
            format!("Catalogue ({} entries)", disc.catalogue.len()),
            |ui| {
                egui::Grid::new(("fdc-catalogue", unit))
                    .striped(true)
                    .show(ui, |ui| {
                        for entry in disc.catalogue.iter().take(64) {
                            ui.monospace(&entry.name);
                            ui.weak(format!(
                                "user {}{}, {}K",
                                entry.user,
                                if entry.hidden { ", hidden" } else { "" },
                                entry.kilobytes
                            ));
                            ui.end_row();
                        }
                    });
            },
        );
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
    crate::ui::widgets::rule(ui);
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
            crate::ui::widgets::rule(ui);
            disc_details(ui, mb, unit);
        }
    }
    crate::ui::widgets::rule(ui);
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
