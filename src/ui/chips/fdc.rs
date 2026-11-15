//! The FDC window: its status, its drives and the discs as the head sees them.

use super::*;

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

#[cfg(test)]
mod tests {
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
}
