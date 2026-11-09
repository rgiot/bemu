//! Debug > View > Screen memory: the video RAM as the CRTC and the Gate Array show it, from the
//! CRTC's start address, width and character height and the Gate Array's mode and inks - or from
//! the address, width, height and mode given (a sprite sheet, a screen not shown yet).

use crate::ga::RgbLevels;
use crate::motherboard::Motherboard;

/// The window's fields (empty: the CRTC's / Gate Array's).
#[derive(Default)]
pub struct ScreenState {
    address: String,
    width: String,
    row_height: String,
    rows: String,
    /// `None`: the Gate Array's mode.
    mode: Option<u8>,
    texture: Option<egui::TextureHandle>,
}

/// The pens of the pixels of `byte` in `mode`, left to right (2 in mode 0, 4 in mode 1, 8 in
/// mode 2; mode 3 is mode 0's layout with 4 pens).
pub fn pens(byte: u8, mode: u8) -> Vec<u8> {
    let bit = |n: u8| (byte >> n) & 1;
    match mode & 3 {
        0 | 3 => {
            let pixel = |b0: u8, b1: u8, b2: u8, b3: u8| {
                let pen = bit(b0) | bit(b1) << 1 | bit(b2) << 2 | bit(b3) << 3;
                if mode & 3 == 3 {
                    pen & 3
                } else {
                    pen
                }
            };
            vec![pixel(7, 3, 5, 1), pixel(6, 2, 4, 0)]
        }
        1 => (0..4).map(|i| bit(7 - i) | bit(3 - i) << 1).collect(),
        _ => (0..8).map(|i| bit(7 - i)).collect(),
    }
}

/// Where the byte `column` of picture line `line` is: the CRTC's addressing (each character row
/// `width` bytes on, each of its lines &800 further, within the 16K page of `start`).
pub fn byte_address(start: u16, width: u16, row_height: u16, line: u16, column: u16) -> u16 {
    let base = start & 0xc000;
    let row = line / row_height.max(1);
    let raster = line % row_height.max(1) % 8;
    base + ((start & 0x3fff)
        .wrapping_add(row.wrapping_mul(width))
        .wrapping_add(raster * 0x800)
        .wrapping_add(column)
        & 0x3fff)
}

fn color(levels: RgbLevels) -> egui::Color32 {
    let byte = |level: f32| (level * 255.0 / 100.0).round().clamp(0.0, 255.0) as u8;
    egui::Color32::from_rgb(byte(levels.r()), byte(levels.g()), byte(levels.b()))
}

fn field(ui: &mut egui::Ui, label: &str, text: &mut String, hint: String) {
    ui.label(label);
    ui.add(
        egui::TextEdit::singleline(text)
            .hint_text(hint)
            .desired_width(56.0),
    );
}

fn number(text: &str) -> Option<u16> {
    let text = text.trim();
    match text
        .strip_prefix('&')
        .or_else(|| text.strip_prefix('#'))
        .or_else(|| text.strip_prefix("0x"))
    {
        Some(hex) => u16::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

pub fn show(ui: &mut egui::Ui, mb: &Motherboard, state: &mut ScreenState) {
    let crtc = mb.crtc().registers();
    let crtc_start = cpclib_dap::inspect::crtc_screen_start_address(crtc[12], crtc[13]) as u16;
    let (crtc_width, crtc_height) = cpclib_dap::inspect::crtc_screen_defaults(&crtc);
    let ga_mode = mb.ga().active_video_mode() as u8;
    ui.horizontal(|ui| {
        field(
            ui,
            "Address",
            &mut state.address,
            format!("&{crtc_start:04X}"),
        );
        field(ui, "Width", &mut state.width, format!("{crtc_width}"));
        field(
            ui,
            "Lines/row",
            &mut state.row_height,
            format!("{crtc_height}"),
        );
        field(ui, "Rows", &mut state.rows, format!("{}", crtc[6]));
    });
    ui.horizontal(|ui| {
        ui.label("Mode");
        let names = ["Gate Array's", "0", "1", "2", "3"];
        let mut index = state.mode.map_or(0, |m| usize::from(m) + 1);
        egui::ComboBox::from_id_salt("screen-mode")
            .show_index(ui, &mut index, 5, |i| names[i].to_owned());
        state.mode = (index > 0).then(|| index as u8 - 1);
        if ui.button("The CRTC's").clicked() {
            *state = ScreenState {
                texture: state.texture.take(),
                ..ScreenState::default()
            };
        }
    });
    let start = number(&state.address).unwrap_or(crtc_start);
    let width = number(&state.width)
        .unwrap_or(crtc_width as u16)
        .clamp(1, 128);
    let row_height = number(&state.row_height)
        .unwrap_or(crtc_height as u16)
        .clamp(1, 32);
    let rows = number(&state.rows)
        .unwrap_or(u16::from(crtc[6]))
        .clamp(1, 64);
    let mode = state.mode.unwrap_or(ga_mode);
    let lines = rows * row_height;
    // 8 pixels a byte in every mode (a mode 0 pixel 4 wide), each line twice: the CPC's shape
    let (w, h) = (usize::from(width) * 8, usize::from(lines) * 2);
    let inks: Vec<egui::Color32> = mb.ga().palette()[..16]
        .iter()
        .map(|ink| color(RgbLevels::from(*ink)))
        .collect();
    let mut pixels = vec![egui::Color32::BLACK; w * h];
    for line in 0..lines {
        for column in 0..width {
            let address = byte_address(start, width, row_height, line, column);
            let byte = mb
                .read_bank_byte(address / 0x4000, address & 0x3fff)
                .unwrap_or(0);
            let pens = pens(byte, mode);
            let stretch = 8 / pens.len();
            for (i, pen) in pens.iter().enumerate() {
                let ink = inks[usize::from(*pen)];
                for dx in 0..stretch {
                    let x = usize::from(column) * 8 + i * stretch + dx;
                    for dy in 0..2 {
                        pixels[(usize::from(line) * 2 + dy) * w + x] = ink;
                    }
                }
            }
        }
    }
    let image = egui::ColorImage::new([w, h], pixels);
    let texture = match &mut state.texture {
        Some(texture) => {
            texture.set(image, egui::TextureOptions::NEAREST);
            texture.clone()
        }
        None => {
            let texture =
                ui.ctx()
                    .load_texture("screen-memory", image, egui::TextureOptions::NEAREST);
            state.texture = Some(texture.clone());
            texture
        }
    };
    let response = ui.add(egui::Image::new(&texture).sense(egui::Sense::hover()));
    // the byte under the pointer
    if let Some(pointer) = response.hover_pos() {
        let rect = response.rect;
        let x = ((pointer.x - rect.left()) / rect.width() * w as f32) as usize;
        let y = ((pointer.y - rect.top()) / rect.height() * h as f32) as usize;
        let (column, line) = ((x / 8) as u16, (y / 2) as u16);
        if column < width && line < lines {
            let address = byte_address(start, width, row_height, line, column);
            let byte = mb
                .read_bank_byte(address / 0x4000, address & 0x3fff)
                .unwrap_or(0);
            response.on_hover_text(format!(
                "&{address:04X}: &{byte:02X} (line {line}, byte {column}), pens {:?}",
                pens(byte, mode)
            ));
        }
    }
    ui.weak(format!(
        "{} bytes × {lines} lines from &{start:04X}, mode {mode}",
        width
    ));
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn a_byte_is_the_pens_of_its_mode() {
        // mode 1: &88 is pen 3 at the left; mode 2: bits from 7 down
        assert_eq!(pens(0x88, 1), vec![3, 0, 0, 0]);
        assert_eq!(pens(0x80, 2), vec![1, 0, 0, 0, 0, 0, 0, 0]);
        // mode 0: bits 7,3,5,1 make the first pixel's pen bits 0-3
        assert_eq!(pens(0x80, 0), vec![1, 0]);
        assert_eq!(pens(0x02, 0), vec![8, 0]);
        assert_eq!(pens(0x01, 0), vec![0, 8]);
    }

    #[test]
    fn the_screen_is_addressed_as_the_crtc_does() {
        // the standard screen: line 1 is &800 on, character row 1 is 80 bytes on
        assert_eq!(byte_address(0xc000, 80, 8, 0, 0), 0xc000);
        assert_eq!(byte_address(0xc000, 80, 8, 1, 0), 0xc800);
        assert_eq!(byte_address(0xc000, 80, 8, 8, 3), 0xc053);
        // within its 16K page
        assert_eq!(
            byte_address(0xc000, 80, 8, 7, 0x7ff),
            0xc000 | (0x3800 + 0x7ff)
        );
    }
}
