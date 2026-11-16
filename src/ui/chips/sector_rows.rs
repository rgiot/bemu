//! The disc sector by sector: every track a group of rows, every row one sector, its raw bytes
//! from left to right - its ID field (C, H, R, N and the CRC), then its data - the way Michael
//! Steil's second 1541 view draws a disc to see its read errors. A weak sector has a row for
//! each of its copies, so the bytes that change from one to the next show as a column of
//! different colours; a missing part is pale.

use super::disc_view::{is_empty, scale, Kind, ID_COLOUR};
use super::*;
use crate::disc_image::{DiscImage, Sector};
use crate::fdc::FdcDrive;

/// The width of a sector's data on the picture (pixels): a byte a pixel up to this, shrunk
/// above (a 4K sector).
const DATA_WIDTH: usize = 520;
/// The ID field: C, H, R, N, then its two CRC bytes; a gap before the data.
const ID_BYTES: usize = 6;
const GAP: usize = 4;
const WIDTH: usize = ID_BYTES + GAP + DATA_WIDTH;
/// The room for the track number, left of the picture (points).
const LABEL: f32 = 30.0;
/// The picture's height is kept under this (pixels): rows are one pixel high above.
const MAX_HEIGHT: usize = 4000;

/// One row of the picture.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Row {
    /// Its first line on the picture.
    y: usize,
    track: u8,
    sector: usize,
    copy: usize,
}

/// A side's picture and where its rows are.
struct Picture {
    context: egui::Context,
    texture: egui::TextureHandle,
    signature: u64,
    rows: Vec<Row>,
    row_height: usize,
    /// The first line of each track's group.
    groups: Vec<(u8, usize)>,
    /// How many bytes a pixel is.
    bytes_per_pixel: usize,
    height: usize,
}

/// The pictures of the disc, one a side.
#[derive(Default)]
pub struct RowsView {
    pictures: [Option<Picture>; 2],
}

/// How a row's pixels are coloured: its bytes (ID then data), as the sector says.
fn row_pixels(
    sector: &Sector,
    cylinder: u8,
    copy: usize,
    bytes_per_pixel: usize,
) -> Vec<egui::Color32> {
    let kind = Kind::of(sector, cylinder);
    let empty = is_empty(sector);
    let mut pixels = vec![egui::Color32::TRANSPARENT; WIDTH];
    // the ID field
    let id = [sector.c, sector.h, sector.r, sector.n];
    for (i, byte) in id.iter().enumerate() {
        let shade = 0.55 + 0.45 * f32::from(byte.count_ones() as u8) / 8.0;
        pixels[i] = scale(ID_COLOUR, shade);
    }
    // its CRC: right, or the colour of the error
    let crc = if kind == Kind::IdCrcError {
        Kind::IdCrcError.colour()
    } else {
        scale(ID_COLOUR, 0.6)
    };
    pixels[4] = crc;
    pixels[5] = crc;
    // a sector of another track: its C is of that colour
    if kind == Kind::OtherTrack {
        pixels[0] = Kind::OtherTrack.colour();
    }
    // no data: nothing after the ID, a pale bar where it should be
    if kind == Kind::NoData {
        let length = (sector.nominal_len() / bytes_per_pixel).min(DATA_WIDTH);
        for pixel in &mut pixels[ID_BYTES + GAP..ID_BYTES + GAP + length] {
            *pixel = Kind::NoData.colour();
        }
        return pixels;
    }
    // the data (the copy of a weak sector), then its two CRC bytes
    let nominal = sector.nominal_len();
    let base = copy * nominal;
    let body = Kind::Data.colour();
    let tint = match kind {
        Kind::Deleted => Kind::Deleted.colour(),
        _ => body,
    };
    for i in 0..(nominal / bytes_per_pixel).min(DATA_WIDTH) {
        let offset = base + i * bytes_per_pixel;
        let byte = sector.data.get(offset).copied().unwrap_or(0);
        let shade = if empty {
            0.5
        } else {
            0.55 + 0.45 * f32::from(byte.count_ones() as u8) / 8.0
        };
        pixels[ID_BYTES + GAP + i] = scale(tint, shade);
    }
    // the data CRC: the colour of its error
    let crc_at = ID_BYTES + GAP + (nominal / bytes_per_pixel).min(DATA_WIDTH.saturating_sub(2));
    let crc_colour = match kind {
        Kind::CrcError => Kind::CrcError.colour(),
        _ => scale(body, 0.6),
    };
    for pixel in pixels.iter_mut().skip(crc_at).take(2) {
        *pixel = crc_colour;
    }
    // a weak sector: the bytes its copies differ in stand out
    if kind == Kind::Weak && sector.copies() > 1 {
        for i in 0..(nominal / bytes_per_pixel).min(DATA_WIDTH) {
            let offset = i * bytes_per_pixel;
            let first = sector.data.get(offset);
            if (1..sector.copies()).any(|c| sector.data.get(c * nominal + offset) != first) {
                pixels[ID_BYTES + GAP + i] = Kind::Weak.colour();
            }
        }
    }
    pixels
}

/// The picture of the side `side` of `disc`.
fn build(
    disc: &DiscImage,
    side: u8,
) -> (egui::ColorImage, Vec<Row>, Vec<(u8, usize)>, usize, usize) {
    // the rows first: a row a sector, a row a copy of a weak one
    let mut layout: Vec<(u8, usize, usize)> = Vec::new();
    for cylinder in 0..disc.nb_tracks() {
        if let Some(track) = disc.track(side, cylinder) {
            for (index, sector) in track.sectors.iter().enumerate() {
                for copy in 0..sector.copies().max(1) {
                    layout.push((cylinder, index, copy));
                }
            }
        }
    }
    let tracks_with_rows = layout
        .iter()
        .map(|(t, ..)| *t)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let row_height = if (layout.len() * 2 + tracks_with_rows * 2) > MAX_HEIGHT {
        1
    } else {
        2
    };
    let widest = (0..disc.nb_tracks())
        .filter_map(|c| disc.track(side, c))
        .flat_map(|t| t.sectors.iter().map(Sector::nominal_len))
        .max()
        .unwrap_or(512);
    let bytes_per_pixel = widest.div_ceil(DATA_WIDTH).max(1);

    let mut rows = Vec::with_capacity(layout.len());
    let mut groups = Vec::new();
    let mut y = 0;
    let mut last_track = None;
    let mut lines: Vec<Vec<egui::Color32>> = Vec::new();
    for (cylinder, index, copy) in layout {
        if last_track != Some(cylinder) {
            if last_track.is_some() {
                // a gap between two tracks
                for _ in 0..2 {
                    lines.push(vec![egui::Color32::TRANSPARENT; WIDTH]);
                    y += 1;
                }
            }
            groups.push((cylinder, y));
            last_track = Some(cylinder);
        }
        let sector = &disc.track(side, cylinder).expect("a row's track").sectors[index];
        let pixels = row_pixels(sector, cylinder, copy, bytes_per_pixel);
        rows.push(Row {
            y,
            track: cylinder,
            sector: index,
            copy,
        });
        for _ in 0..row_height {
            lines.push(pixels.clone());
            y += 1;
        }
    }
    let height = y.max(1);
    let mut image = egui::ColorImage::new(
        [WIDTH, height],
        vec![egui::Color32::TRANSPARENT; WIDTH * height],
    );
    for (line, pixels) in lines.iter().enumerate() {
        image.pixels[line * WIDTH..(line + 1) * WIDTH].copy_from_slice(pixels);
    }
    (image, rows, groups, bytes_per_pixel, row_height)
}

/// The disc in drive `unit`, sector by sector, the side the head reads (the side up of a 3"
/// drive); a byte hovered says what it is.
pub fn show(ui: &mut egui::Ui, mb: &Motherboard, unit: usize, view: &mut RowsView, signature: u64) {
    let drive = if unit == 0 { FdcDrive::A } else { FdcDrive::B };
    let fdc = mb.fdc();
    let Some(disc) = fdc.image(drive) else {
        return;
    };
    let side = fdc.side_read(unit).min(disc.nb_heads().saturating_sub(1));
    let kept = &mut view.pictures[usize::from(side & 1)];
    let fresh = !matches!(kept, Some(p) if p.context == *ui.ctx() && p.signature == signature);
    if fresh {
        let (image, rows, groups, bytes_per_pixel, row_height) = build(disc, side);
        let height = image.size[1];
        let texture = ui.ctx().load_texture(
            format!("disc-rows-{unit}-{side}"),
            image,
            egui::TextureOptions::NEAREST,
        );
        *kept = Some(Picture {
            context: ui.ctx().clone(),
            texture,
            signature,
            rows,
            row_height,
            groups,
            bytes_per_pixel,
            height,
        });
    }
    let picture = kept.as_ref().expect("drawn");
    ui.weak(format!(
        "Side {} read: a row a sector (a row a copy of a weak one), its ID field then its data; \
         {} byte(s) a pixel.",
        ["A", "B"][usize::from(side & 1)],
        picture.bytes_per_pixel
    ));
    ui.weak(
        "The bytes the FDC decodes, not the raw MFM bits: a dropout or a protection that lives \
         in the flux does not show here.",
    );
    egui::ScrollArea::vertical()
        .id_salt(("disc-rows", unit))
        .max_height(380.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            let size = egui::vec2(LABEL + WIDTH as f32, picture.height as f32);
            let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
            let origin = rect.min + egui::vec2(LABEL, 0.0);
            let painter = ui.painter_at(rect);
            painter.image(
                picture.texture.id(),
                egui::Rect::from_min_size(origin, egui::vec2(WIDTH as f32, picture.height as f32)),
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            // the track of each group, where it starts (those in view)
            let visible = ui.clip_rect();
            for (track, y) in &picture.groups {
                let at = rect.min.y + *y as f32;
                if at >= visible.top() - 12.0 && at <= visible.bottom() {
                    painter.text(
                        egui::pos2(rect.min.x + LABEL - 4.0, at),
                        egui::Align2::RIGHT_TOP,
                        track.to_string(),
                        egui::FontId::monospace(9.0),
                        ui.visuals().weak_text_color(),
                    );
                }
            }
            // a byte hovered: its track, sector, copy, offset and value
            if let Some(pointer) = response.hover_pos() {
                let (x, y) = (pointer.x - origin.x, pointer.y - rect.min.y);
                if x >= 0.0 && y >= 0.0 {
                    let (x, y) = (x as usize, y as usize);
                    let row = picture
                        .rows
                        .iter()
                        .rev()
                        .find(|r| r.y <= y && y < r.y + picture.row_height);
                    if let Some(row) = row {
                        if let Some(sector) = disc
                            .track(side, row.track)
                            .and_then(|t| t.sectors.get(row.sector))
                        {
                            response.on_hover_text(hover(sector, *row, x, picture.bytes_per_pixel));
                        }
                    }
                }
            }
        });
}

/// What the pointer at column `x` of `row` is over.
fn hover(sector: &Sector, row: Row, x: usize, bytes_per_pixel: usize) -> String {
    let kind = Kind::of(sector, row.track);
    let head = format!(
        "track {}, sector {} (C &{:02X} H &{:02X} R &{:02X} N {}){}",
        row.track,
        row.sector + 1,
        sector.c,
        sector.h,
        sector.r,
        sector.n,
        if sector.copies() > 1 {
            format!(", copy {} of {}", row.copy + 1, sector.copies())
        } else {
            String::new()
        }
    );
    if x < ID_BYTES {
        let part = ["C", "H", "R", "N", "CRC", "CRC"][x];
        return format!(
            "{head}\nID field: {part}{}",
            if kind == Kind::IdCrcError && x >= 4 {
                " (wrong)"
            } else {
                ""
            }
        );
    }
    let Some(i) = x.checked_sub(ID_BYTES + GAP) else {
        return format!("{head}\nbetween the ID and the data");
    };
    let nominal = sector.nominal_len();
    let offset = i * bytes_per_pixel;
    if offset >= nominal {
        return format!(
            "{head}\n{}",
            if offset < nominal + 2 * bytes_per_pixel {
                "data CRC"
            } else {
                "(past the end of the sector)"
            }
        );
    }
    if kind == Kind::NoData {
        return format!("{head}\nno data mark: the data is not there");
    }
    let byte = sector
        .data
        .get(row.copy * nominal + offset)
        .copied()
        .unwrap_or(0);
    format!(
        "{head}\nbyte {offset} (&{offset:03X}) = &{byte:02X} ({byte:#010b}){}",
        if kind == Kind::CrcError {
            "\nthe data has a CRC error"
        } else {
            ""
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disc_image::Track;

    fn disc(tracks: u8, sectors: u8, n: u8) -> DiscImage {
        let mut disc = DiscImage::new(tracks, 1);
        for cylinder in 0..tracks {
            let track = Track {
                track: cylinder,
                head: 0,
                gap3: 0x4e,
                filler: 0xe5,
                sectors: (0..sectors)
                    .map(|r| Sector::new(cylinder, 0, 0xc1 + r, n, 0xe5))
                    .collect(),
            };
            disc.set_track(0, cylinder, track).unwrap();
        }
        disc
    }

    /// A row a sector, a gap between tracks, every row where it says it is.
    #[test]
    fn a_row_a_sector_in_a_group_a_track() {
        let (image, rows, groups, bytes_per_pixel, row_height) = build(&disc(3, 9, 2), 0);
        assert_eq!(rows.len(), 27);
        assert_eq!(groups.len(), 3);
        assert_eq!((bytes_per_pixel, row_height), (1, 2));
        // 3 groups of 9 rows of 2 lines, 2 gaps of 2 lines
        assert_eq!(image.size, [WIDTH, 3 * 9 * 2 + 2 * 2]);
        assert_eq!(groups[1].1, 9 * 2 + 2);
        assert!(rows.windows(2).all(|w| w[0].y < w[1].y));
        // the ID field is coloured, the data is
        let first = &image.pixels[..WIDTH];
        assert_ne!(first[0], egui::Color32::TRANSPARENT);
        assert_ne!(first[ID_BYTES + GAP + 100], egui::Color32::TRANSPARENT);
    }

    /// A weak sector has a row a copy, and the bytes its copies differ in are marked.
    #[test]
    fn a_weak_sector_has_a_row_a_copy() {
        let mut image = disc(1, 2, 1);
        let mut track = image.track(0, 0).unwrap().clone();
        let sector = &mut track.sectors[0];
        sector.data = [vec![1; 256], vec![2; 256]].concat();
        image.set_track(0, 0, track).unwrap();
        let (image, rows, ..) = build(&image, 0);
        assert_eq!(rows.iter().filter(|r| r.sector == 0).count(), 2);
        let pixel = image.pixels[ID_BYTES + GAP + 10];
        assert_eq!(pixel, Kind::Weak.colour(), "the differing bytes");
        // a long sector shrinks into the width
        let (_, _, _, per_pixel, _) = build(&disc(1, 1, 5), 0);
        assert_eq!(per_pixel, 4096_usize.div_ceil(DATA_WIDTH));
    }
}
