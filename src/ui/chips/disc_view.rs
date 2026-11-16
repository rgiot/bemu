//! A disc as the drive turns it: its tracks as rings (track 0 outside), each sector an arc
//! where it lies on its track (its ID field, then its data, coloured by what it is), the head
//! on its track, the sectors the last commands read or wrote glowing. The way C64 tools draw a
//! 1541 disc (Michael Steil's visualize_1541) and ACE its drives.
//!
//! The disc is drawn once into a picture (again when its sectors change), then turned each
//! frame: by the drive's clock with drive timing on a disc that says where its sectors are (an
//! HFE), else to bring the sector the head reads next under it.

use std::hash::{Hash, Hasher};

use super::*;
use crate::disc_image::{flag, DiscImage, Sector, Track};
use crate::fdc::FdcDrive;

/// The disc's diameter on screen (points); its picture is drawn twice as fine.
const DIAMETER: f32 = 260.0;
const PICTURE: usize = 520;
/// The rings between these fractions of the radius (the hub and the label inside).
const INNER: f32 = 0.34;
const OUTER: f32 = 0.97;
/// A 3" disc turns at 300 rpm: the drawing turns no faster to catch up.
const TURNS_PER_S: f32 = 5.0;
/// The bytes a track holds at 250 kbit/s MFM, 300 rpm; the microseconds a byte takes.
const TRACK_BYTES: f32 = 6250.0;
const BYTE_US: f32 = 32.0;
/// The head reads at this angle of the screen (to the right).
const HEAD_ANGLE: f32 = 0.0;

/// What a sector is, by its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Data,
    Deleted,
    CrcError,
    Weak,
    NoData,
    OtherTrack,
}

impl Kind {
    fn of(sector: &Sector, track: u8) -> Self {
        if sector.copies() > 1 {
            Self::Weak
        } else if sector.st1 & flag::ST1_MISSING_ADDRESS_MARK != 0
            || sector.st2 & flag::ST2_MISSING_DATA_MARK != 0
        {
            Self::NoData
        } else if sector.st1 & flag::ST1_DATA_ERROR != 0
            || sector.st2 & flag::ST2_DATA_ERROR_IN_DATA != 0
        {
            Self::CrcError
        } else if sector.is_deleted() {
            Self::Deleted
        } else if sector.c != track {
            Self::OtherTrack
        } else {
            Self::Data
        }
    }

    fn colour(self) -> egui::Color32 {
        use egui::Color32 as C;
        match self {
            Self::Data => C::from_rgb(70, 165, 85),
            Self::Deleted => C::from_rgb(150, 90, 200),
            Self::CrcError => C::from_rgb(235, 140, 40),
            Self::Weak => C::from_rgb(215, 60, 60),
            Self::NoData => C::from_rgb(120, 120, 120),
            Self::OtherTrack => C::from_rgb(70, 120, 220),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Data => "data",
            Self::Deleted => "deleted data",
            Self::CrcError => "CRC error",
            Self::Weak => "weak (several copies)",
            Self::NoData => "no data mark",
            Self::OtherTrack => "another track's ID",
        }
    }
}

const ID_COLOUR: egui::Color32 = egui::Color32::from_rgb(80, 200, 215);
const SURFACE: egui::Color32 = egui::Color32::from_rgb(38, 34, 30);
const UNFORMATTED: egui::Color32 = egui::Color32::from_rgb(24, 22, 20);

/// Where a sector lies on its track: fractions of a turn from the index hole, its ID field
/// `[id.0, id.1)`, its data `[data.0, data.1)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Arc {
    pub id: (f32, f32),
    pub data: (f32, f32),
    pub kind: Kind,
}

/// The arcs of the sectors of `track` (`head`, its number `cylinder`), in the order they pass:
/// from the image's timings when it has them (a bitstream), else from MFM's layout - the
/// gaps, the ID field, the data and their CRC, a turn holding 6250 bytes (a track holding more,
/// overformatted, is squeezed into the turn).
pub(super) fn layout(disc: &DiscImage, head: u8, cylinder: u8, track: &Track) -> Vec<Arc> {
    if let Some(timing) = disc
        .track_timing(head, cylinder)
        .filter(|t| t.period > 0 && t.id_start.len() == track.sectors.len())
    {
        let period = timing.period as f32;
        return track
            .sectors
            .iter()
            .enumerate()
            .map(|(i, sector)| {
                let data = timing.data_start[i] as f32 / period;
                let length = (sector.nominal_len() as f32 + 2.0) * BYTE_US / period;
                Arc {
                    id: (
                        timing.id_start[i] as f32 / period,
                        timing.id_end[i] as f32 / period,
                    ),
                    data: (data, data + length),
                    kind: Kind::of(sector, cylinder),
                }
            })
            .collect();
    }
    // gap 4a, sync, index mark, gap 1; before an ID: sync and its mark; ID and CRC; gap 2, sync
    // and the data mark; after the data: its CRC, then gap 3
    const START: f32 = 80.0 + 12.0 + 4.0 + 50.0;
    const BEFORE_ID: f32 = 12.0 + 4.0;
    const ID: f32 = 6.0;
    const BEFORE_DATA: f32 = 22.0 + 12.0 + 4.0;
    let gap3 = f32::from(track.gap3.max(1));
    let total = START
        + track
            .sectors
            .iter()
            .map(|s| BEFORE_ID + ID + BEFORE_DATA + s.nominal_len() as f32 + 2.0 + gap3)
            .sum::<f32>();
    let turn = total.max(TRACK_BYTES);
    let mut at = START;
    track
        .sectors
        .iter()
        .map(|sector| {
            let id = at + BEFORE_ID;
            let data = id + ID + BEFORE_DATA;
            let end = data + sector.nominal_len() as f32 + 2.0;
            at = end + gap3;
            Arc {
                id: (id / turn, (id + ID) / turn),
                data: (data / turn, end / turn),
                kind: Kind::of(sector, cylinder),
            }
        })
        .collect()
}

/// The arc at `position` (a fraction of a turn) of `arcs`, if any: its index, and whether it is
/// its ID field.
fn arc_at(arcs: &[Arc], position: f32) -> Option<(usize, bool)> {
    arcs.iter().enumerate().find_map(|(i, arc)| {
        let within = |(from, to): (f32, f32)| position >= from && position < to;
        if within(arc.id) {
            Some((i, true))
        } else if within(arc.data) {
            Some((i, false))
        } else {
            None
        }
    })
}

/// The ring of `radius` (a fraction of the disc's radius): its track, of `tracks`.
fn track_at(radius: f32, tracks: u8) -> Option<u8> {
    if !(INNER..OUTER).contains(&radius) || tracks == 0 {
        return None;
    }
    let ring = (OUTER - radius) / (OUTER - INNER) * f32::from(tracks);
    Some((ring as u8).min(tracks - 1))
}

/// The radius (fraction) of the middle of track `track` of `tracks`.
fn track_radius(track: u8, tracks: u8) -> f32 {
    OUTER - (f32::from(track) + 0.5) / f32::from(tracks.max(1)) * (OUTER - INNER)
}

/// The disc's sides and sectors, as far as the colours go: another one, another picture.
fn signature(disc: &DiscImage, side: u8) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (side, disc.nb_tracks(), disc.is_bitstream()).hash(&mut hasher);
    for cylinder in 0..disc.nb_tracks() {
        match disc.track(side, cylinder) {
            None => 0u8.hash(&mut hasher),
            Some(track) => {
                (track.gap3, track.sectors.len()).hash(&mut hasher);
                for s in &track.sectors {
                    (s.c, s.h, s.r, s.n, s.st1, s.st2, s.copies(), s.data.len()).hash(&mut hasher);
                }
            }
        }
    }
    hasher.finish()
}

/// The disc's picture, not turned: `PICTURE` pixels square, the position on a track counting
/// anticlockwise from the index hole (at the right): the disc turns clockwise, so its sectors
/// pass under the head (at the right) in their order.
fn picture(disc: &DiscImage, side: u8) -> egui::ColorImage {
    let tracks = disc.nb_tracks();
    let layouts: Vec<Option<Vec<Arc>>> = (0..tracks)
        .map(|c| disc.track(side, c).map(|t| layout(disc, side, c, t)))
        .collect();
    let ring_px = (OUTER - INNER) * PICTURE as f32 / 2.0 / f32::from(tracks.max(1));
    let mut image = egui::ColorImage::new(
        [PICTURE, PICTURE],
        vec![egui::Color32::TRANSPARENT; PICTURE * PICTURE],
    );
    let centre = PICTURE as f32 / 2.0;
    for y in 0..PICTURE {
        for x in 0..PICTURE {
            let (dx, dy) = (x as f32 + 0.5 - centre, y as f32 + 0.5 - centre);
            let radius = (dx * dx + dy * dy).sqrt() / centre;
            let colour = if radius > 1.0 {
                continue;
            } else if radius < 0.1 {
                // the hub's hole
                continue;
            } else if radius < INNER || radius >= OUTER {
                // the label and the rim
                if radius < INNER - 0.04 {
                    egui::Color32::from_rgb(58, 54, 50)
                } else {
                    SURFACE
                }
            } else {
                let track = track_at(radius, tracks).unwrap_or(0);
                // the rings told apart: a fine line between wide ones, narrow ones (a moire
                // with lines) every other one a little darker
                let in_ring = ((OUTER - radius) / (OUTER - INNER) * f32::from(tracks)).fract();
                let darker = ring_px < 5.0 && track % 2 == 1;
                if ring_px >= 5.0 && in_ring * ring_px < 0.8 {
                    UNFORMATTED
                } else {
                    // the angle, anticlockwise (y goes down on the screen)
                    let position = ((-dy).atan2(dx) / std::f32::consts::TAU).rem_euclid(1.0);
                    let colour = match &layouts[usize::from(track)] {
                        None => UNFORMATTED,
                        Some(arcs) => match arc_at(arcs, position) {
                            Some((_, true)) => ID_COLOUR,
                            Some((i, false)) => arcs[i].kind.colour(),
                            None => SURFACE,
                        },
                    };
                    if darker {
                        egui::Color32::from_rgb(
                            (f32::from(colour.r()) * 0.72) as u8,
                            (f32::from(colour.g()) * 0.72) as u8,
                            (f32::from(colour.b()) * 0.72) as u8,
                        )
                    } else {
                        colour
                    }
                }
            };
            image.pixels[y * PICTURE + x] = colour;
        }
    }
    // the index hole, by the hub at the start of the tracks
    let hole = (centre + 0.24 * centre, centre);
    for y in 0..PICTURE {
        for x in 0..PICTURE {
            let (dx, dy) = (x as f32 - hole.0, y as f32 - hole.1);
            if dx * dx + dy * dy < (0.035 * centre).powi(2) {
                image.pixels[y * PICTURE + x] = egui::Color32::TRANSPARENT;
            }
        }
    }
    image
}

/// A drive's disc drawing: its picture and how far it has turned.
#[derive(Default)]
pub struct DiscView {
    /// The picture, the context it was given to (a window opened again has another) and the
    /// disc it shows.
    texture: Option<(egui::Context, egui::TextureHandle, u64)>,
    /// The turn drawn (fraction), and when.
    turned: f32,
    at: Option<f64>,
}

/// An arc of `from..to` (fractions of a turn, on the disc) between the radii `r0..r1` (points),
/// turned by `turned`, as a mesh.
fn arc_mesh(
    mesh: &mut egui::Mesh,
    centre: egui::Pos2,
    (r0, r1): (f32, f32),
    (from, to): (f32, f32),
    turned: f32,
    colour: egui::Color32,
) {
    let steps = ((to - from) * 180.0).ceil().max(1.0) as usize;
    let base = mesh.vertices.len() as u32;
    for step in 0..=steps {
        let position = from + (to - from) * step as f32 / steps as f32;
        let angle = screen_angle(position, turned);
        let (sin, cos) = angle.sin_cos();
        for r in [r0, r1] {
            mesh.colored_vertex(centre + egui::vec2(cos, -sin) * r, colour);
        }
    }
    for step in 0..steps as u32 {
        let i = base + step * 2;
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i + 1, i + 3, i + 2);
    }
}

/// The angle on the screen (anticlockwise from the right, radians) of `position` on the disc
/// turned by `turned` (clockwise): the head, on the right, reads the position `turned`.
fn screen_angle(position: f32, turned: f32) -> f32 {
    HEAD_ANGLE + (position - turned) * std::f32::consts::TAU
}

/// The sectors the last read and write commands of drive `unit` addressed, the last first:
/// (cylinder, side, sector ID).
fn recent_accesses(mb: &Motherboard, unit: usize) -> Vec<(u8, u8, u8)> {
    mb.fdc()
        .recent_commands()
        .rev()
        .filter(|record| {
            // Read Data, Read Deleted, Write Data, Write Deleted
            let op = record.command.first().map_or(0, |c| c & 0x1f);
            matches!(op, 0x05 | 0x06 | 0x09 | 0x0c) && record.command.len() >= 7
        })
        .filter(|record| usize::from(record.command[1] & 3) == unit)
        .flat_map(|record| {
            let (c, side, first, last) = (
                record.command[2],
                (record.command[1] >> 2) & 1,
                record.command[4],
                record.command[6],
            );
            // the sectors up to EOT (a multi-sector read)
            (first..=last.max(first))
                .take(32)
                .map(move |r| (c, side, r))
        })
        .take(16)
        .collect()
}

/// The disc in drive `unit`, turning, its head on its track; a sector hovered says what it is.
pub fn show(ui: &mut egui::Ui, mb: &Motherboard, unit: usize, view: &mut DiscView) {
    let drive = if unit == 0 { FdcDrive::A } else { FdcDrive::B };
    let fdc = mb.fdc();
    let Some(disc) = fdc.image(drive) else {
        return;
    };
    let tracks = disc.nb_tracks();
    let side = fdc.disc_side(drive).min(disc.nb_heads().saturating_sub(1));
    let head_track = fdc.track(unit).min(tracks.saturating_sub(1));
    let under = disc.track(side, head_track);
    let arcs = under
        .map(|t| layout(disc, side, head_track, t))
        .unwrap_or_default();

    // the picture, drawn again for another disc (or another context)
    let signature = signature(disc, side);
    let fresh = !matches!(&view.texture, Some((ctx, _, s)) if *ctx == *ui.ctx() && *s == signature);
    if fresh {
        let texture = ui.ctx().load_texture(
            format!("disc-{unit}"),
            picture(disc, side),
            egui::TextureOptions::LINEAR,
        );
        view.texture = Some((ui.ctx().clone(), texture, signature));
    }
    let texture = view
        .texture
        .as_ref()
        .map(|(_, t, _)| t.clone())
        .expect("drawn");

    // how far it has turned: the drive's clock, or the next sector brought under the head
    let now = ui.input(|i| i.time);
    let timing = disc
        .track_timing(side, head_track)
        .filter(|t| t.period > 0 && fdc.time_us() > 0);
    let target = match timing {
        Some(timing) => (fdc.time_us() % timing.period) as f32 / timing.period as f32,
        None => arcs
            .get(fdc.sector_under_head(unit) % arcs.len().max(1))
            .map_or(view.turned, |arc| arc.id.0),
    };
    let elapsed = view.at.map_or(1.0, |at| (now - at) as f32);
    view.at = Some(now);
    let behind = (target - view.turned).rem_euclid(1.0);
    let step = if timing.is_some() {
        behind
    } else {
        behind.min(elapsed * TURNS_PER_S)
    };
    view.turned = (view.turned + step).rem_euclid(1.0);
    if timing.is_some() || behind > step {
        ui.ctx().request_repaint();
    }

    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(DIAMETER, DIAMETER), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let centre = rect.center();
    let radius = DIAMETER / 2.0;
    // the picture, turned: a square of four corners turned about the centre
    let mut mesh = egui::Mesh::with_texture(texture.id());
    let angle = view.turned * std::f32::consts::TAU;
    let (sin, cos) = angle.sin_cos();
    for (corner, uv) in [
        (egui::vec2(-1.0, -1.0), egui::pos2(0.0, 0.0)),
        (egui::vec2(1.0, -1.0), egui::pos2(1.0, 0.0)),
        (egui::vec2(1.0, 1.0), egui::pos2(1.0, 1.0)),
        (egui::vec2(-1.0, 1.0), egui::pos2(0.0, 1.0)),
    ] {
        // clockwise on the screen (y down)
        let turned = egui::vec2(
            corner.x * cos - corner.y * sin,
            corner.x * sin + corner.y * cos,
        );
        mesh.vertices.push(egui::epaint::Vertex {
            pos: centre + turned * radius,
            uv,
            color: egui::Color32::WHITE,
        });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);

    // what the last commands read or wrote, glowing (the last brightest)
    let ring = |track: u8| {
        let middle = track_radius(track, tracks) * radius;
        let half = (OUTER - INNER) * radius / f32::from(tracks.max(1)) / 2.0;
        (middle - half - 0.5, middle + half + 0.5)
    };
    let mut glow = egui::Mesh::default();
    for (age, (c, h, r)) in recent_accesses(mb, unit).into_iter().enumerate() {
        if h != side {
            continue;
        }
        // the track under the head when it has that ID, else the one of its number
        let cylinder = if under.is_some_and(|t| t.sectors.iter().any(|s| s.c == c && s.r == r)) {
            head_track
        } else {
            c
        };
        let Some(track) = disc.track(side, cylinder) else {
            continue;
        };
        let Some(index) = track.sectors.iter().position(|s| s.c == c && s.r == r) else {
            continue;
        };
        let arc = layout(disc, side, cylinder, track)[index];
        let alpha = (240.0 * (1.0 - age as f32 / 16.0)) as u8;
        // a little wider than its ring: seen on narrow rings
        let (r0, r1) = ring(cylinder);
        arc_mesh(
            &mut glow,
            centre,
            (r0 - 1.5, r1 + 1.5),
            (arc.id.0, arc.data.1),
            view.turned,
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, alpha),
        );
    }
    painter.add(glow);

    // the head, on the right, over its track; the arm from outside
    // as wide as its ring, a head's size at most
    let middle = track_radius(head_track, tracks) * radius;
    let (r0, r1) = ring(head_track);
    let (r0, r1) = (r0.max(middle - 6.0), r1.min(middle + 6.0));
    let head = egui::Rect::from_min_max(
        centre + egui::vec2(r0 - 2.0, -5.0),
        centre + egui::vec2(r1 + 2.0, 5.0),
    );
    let lit = fdc.motor() && fdc.selected_drive() == unit;
    let head_colour = if lit {
        egui::Color32::from_rgb(255, 220, 90)
    } else {
        egui::Color32::from_rgb(200, 200, 200)
    };
    painter.line_segment(
        [
            centre + egui::vec2(r1 + 2.0, 0.0),
            centre + egui::vec2(radius + 6.0, 0.0),
        ],
        egui::Stroke::new(3.0_f32, egui::Color32::from_gray(150)),
    );
    painter.rect_filled(head, 2.0, head_colour);

    // a sector hovered: what it is
    if let Some(pointer) = response.hover_pos() {
        let offset = pointer - centre;
        let at = offset.length() / radius;
        if let Some(track) = track_at(at, tracks) {
            let angle = (-offset.y).atan2(offset.x);
            let position =
                (view.turned + (angle - HEAD_ANGLE) / std::f32::consts::TAU).rem_euclid(1.0);
            let text = match disc.track(side, track) {
                None => format!("track {track}: unformatted"),
                Some(t) => {
                    let arcs = layout(disc, side, track, t);
                    match arc_at(&arcs, position) {
                        Some((i, id)) => {
                            let s = &t.sectors[i];
                            format!(
                                "track {track}, sector {} of {}: C &{:02X} H &{:02X} R &{:02X} N {} ({} bytes){}\n{}",
                                i + 1,
                                t.sectors.len(),
                                s.c,
                                s.h,
                                s.r,
                                s.n,
                                s.nominal_len(),
                                if id { ", its ID field" } else { "" },
                                arcs[i].kind.label()
                            )
                        }
                        None => format!("track {track}: a gap"),
                    }
                }
            };
            response.on_hover_text(text);
        }
    }
}

/// The colours of the disc, explained.
pub fn legend(ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        let swatch = |ui: &mut egui::Ui, colour: egui::Color32, text: &str| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 1.0, colour);
            ui.weak(text);
        };
        swatch(ui, ID_COLOUR, "ID field");
        for kind in [
            Kind::Data,
            Kind::Deleted,
            Kind::CrcError,
            Kind::Weak,
            Kind::NoData,
            Kind::OtherTrack,
        ] {
            swatch(ui, kind.colour(), kind.label());
        }
        swatch(ui, UNFORMATTED, "unformatted");
        swatch(ui, egui::Color32::WHITE, "read / written lately");
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disc_image::Sector;

    fn track(n: u8, count: u8) -> Track {
        Track {
            track: 0,
            head: 0,
            gap3: 0x4e,
            filler: 0xe5,
            sectors: (0..count)
                .map(|r| Sector::new(0, 0, 0xc1 + r, n, 0xe5))
                .collect(),
        }
    }

    /// A data track's 9 sectors follow each other round the turn, in order, without overlapping,
    /// most of the turn used (gap 4 at the end).
    #[test]
    fn a_data_track_fills_its_turn_in_order() {
        let disc = DiscImage::new(40, 1);
        let arcs = layout(&disc, 0, 0, &track(2, 9));
        assert_eq!(arcs.len(), 9);
        for pair in arcs.windows(2) {
            assert!(pair[0].id.0 < pair[0].id.1 && pair[0].id.1 < pair[0].data.0);
            assert!(pair[0].data.1 < pair[1].id.0, "{pair:?}");
        }
        let end = arcs.last().unwrap().data.1;
        assert!(end > 0.85 && end < 1.0, "{end}");
    }

    /// An overformatted track (10 sectors of 512 bytes) is squeezed into the turn.
    #[test]
    fn an_overformatted_track_stays_in_its_turn() {
        let disc = DiscImage::new(40, 1);
        let arcs = layout(&disc, 0, 0, &track(2, 11));
        assert!(arcs.last().unwrap().data.1 <= 1.0);
    }

    /// Track 0 is the outer ring, the last the inner one; a point is found on its arc.
    #[test]
    fn the_rings_and_arcs_are_found_back() {
        assert_eq!(track_at(OUTER - 0.001, 40), Some(0));
        assert_eq!(track_at(INNER + 0.001, 40), Some(39));
        assert_eq!(track_at(0.2, 40), None);
        let disc = DiscImage::new(40, 1);
        let arcs = layout(&disc, 0, 0, &track(2, 9));
        let middle = (arcs[3].data.0 + arcs[3].data.1) / 2.0;
        assert_eq!(arc_at(&arcs, middle), Some((3, false)));
        assert_eq!(arc_at(&arcs, arcs[3].id.0), Some((3, true)));
        // the head reads what is at its angle
        assert!((screen_angle(0.25, 0.25) - HEAD_ANGLE).abs() < 1e-6);
    }
}
