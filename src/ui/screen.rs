//! Debug > View > Screen memory: the video RAM as the CRTC and the Gate Array show it, from the
//! CRTC's start address, width and character height and the Gate Array's mode and inks - or from
//! the address, width, height and mode given (a sprite sheet, a screen not shown yet). The picture
//! is cpclib-dap's (`screen_view_matrix`, what its VS Code screen view draws).

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
    /// The bytes one after the other (a sprite sheet) rather than the CRTC's interleaving.
    linear: bool,
    texture: Option<egui::TextureHandle>,
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
        ui.checkbox(&mut state.linear, "Linear").on_hover_text(
            "The bytes one after the other (a sprite sheet), not the CRTC's interleaving",
        );
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
    // the RAM the CRTC reads: the base 64K, whatever the CPU has paged in
    let memory: Vec<u8> = (0..=0xffffu16)
        .map(|address| {
            mb.read_bank_byte(address / 0x4000, address & 0x3fff)
                .unwrap_or(0)
        })
        .collect();
    let pens: Vec<u8> = mb
        .ga()
        .palette()
        .iter()
        .map(|ink| ink.hardware_number() & 0x1f)
        .collect();
    let palette =
        cpclib_dap::inspect::palette_from_raw_ga(&pens.try_into().expect("16 pens and the border"));
    let encoding = if state.linear {
        cpclib_dap::inspect::ScreenEncoding::Cpc
    } else {
        cpclib_dap::inspect::ScreenEncoding::Screen
    };
    let matrix = match cpclib_dap::inspect::screen_view_matrix(
        usize::from(start),
        usize::from(width),
        usize::from(lines),
        mode,
        &palette,
        &memory,
        usize::from(row_height),
        &[],
        encoding,
    ) {
        Ok(matrix) => matrix,
        Err(error) => {
            ui.colored_label(ui.visuals().error_fg_color, error);
            return;
        }
    };
    let picture = matrix.as_image();
    let (w, h) = (picture.width() as usize, picture.height() as usize);
    let image = egui::ColorImage::from_rgb([w, h], picture.as_raw());
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
            let address = if state.linear {
                start
                    .wrapping_add(line.wrapping_mul(width))
                    .wrapping_add(column)
            } else {
                byte_address(start, width, row_height, line, column)
            };
            let byte = mb
                .read_bank_byte(address / 0x4000, address & 0x3fff)
                .unwrap_or(0);
            response.on_hover_text(format!(
                "&{address:04X}: &{byte:02X} {byte:#010b} (line {line}, byte {column})"
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
