//! The CRT model: parameters, profiles and the geometry of the phosphor mask.
//!
//! The approach (per-channel sub-pixel sampling of the beam, 480 triads
//! across the 768 monitor pixels i.e. an output 1.875 times wider, 4 output
//! rows per scanline, brightness-dependent spill of light into the
//! neighbouring slots, alternate triads staggered vertically like the slot
//! mask of a CTM) is inspired by the rendering source code of the ACE-DL
//! emulator (`CTM_Render.c`). This is an independent reimplementation: the
//! mask is analytic (a rounded rectangle's signed distance, shared with the
//! GPU shader) instead of hand-written pixel patterns and data-dependent
//! smoothing, and light is summed in linear space.
//!
//! Light is linear in the beam's current and the picture is stored as
//! `light^(1/2)` (see [`GAMMA`]): squares and square roots are what the SIMD
//! units do natively.
//!
//! One output pixel belongs to one phosphor slot, lit by one gun only:
//! `R G B` repeat every triad. What reaches the eye is
//! `light(x, y) = beam_c(u, v) * coverage(slot geometry, beam_c)`, optionally
//! blurred horizontally by the glass (halation).

use std::str::FromStr;

/// Source (monitor) size the model is built for.
pub const SRC_WIDTH: usize = 768;
pub const SRC_HEIGHT: usize = 272;
/// Triads across the source width: 480, as ACE-DL's 1.875 output ratio.
pub const TRIADS_ACROSS: usize = 480;

/// Exponent of the phosphors' response to the gun's level, the one of the
/// stored pictures: sRGB's is about 2.2, this is the cheap approximation.
pub const GAMMA: f32 = 2.0;

/// The tubes (and ACE-DL render modes) that can be approximated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrtProfile {
    /// Amstrad CTM644: slot mask, staggered triads.
    Ctm644,
    /// Aperture grille (vertical wires): no gap between the vertical slots.
    Trinitron,
    /// Amstrad GT64 / GT65, the green monitors: the CTM's slot mask and scanlines in one green
    /// phosphor. The colours are shown as their luminance.
    Gt65,
    /// Amstrad MM12, a grey monochrome monitor: as the GT65, in a grey phosphor.
    Mm12,
    /// Atari colour monitor: grille with strong scanlines.
    AtariColor,
}

impl CrtProfile {
    pub const NAMES: &'static str = "ctm|trinitron|gt65|mm12|atari";

    pub fn params(self) -> CrtParams {
        CrtParams::from_profile(self)
    }

    /// The bulge of the tube's glass, `(kx, ky)` of [`Warp`] at curvature 1: the 14" monitors
    /// of the CPC (CTM644, GT65) curve both ways, the Trinitron's cylinder only across, the TV the
    /// most. Eyeballed, not measured.
    pub fn curvature(self) -> (f32, f32) {
        match self {
            CrtProfile::Ctm644 | CrtProfile::Gt65 | CrtProfile::Mm12 => (0.025, 0.035),
            CrtProfile::Trinitron => (0.0, 0.025),
            CrtProfile::AtariColor => (0.035, 0.045),
        }
    }
}

/// The curved glass of the tube, as the viewer sees the flat picture through it: a barrel warp
/// of the whole picture (the CRT model's output), with the glass's rounded corners. The same
/// formula runs on the CPU ([`super::Curved`]) and in the GPU blit (`blit.wgsl`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Warp {
    /// Horizontal and vertical bulge: a point at (x, y) of [-1, 1]² shows the flat picture's
    /// `(x (1 + kx y²), y (1 + ky x²))`.
    pub k: (f32, f32),
    /// Radius of the glass's corners, in half-sizes of the picture.
    pub corner: f32,
}

impl Warp {
    pub const FLAT: Warp = Warp {
        k: (0.0, 0.0),
        corner: 0.0,
    };

    /// The glass of `profile`, its bulge times `scale` (0: flat).
    pub fn of(profile: CrtProfile, scale: f32) -> Self {
        if scale <= 0.0 {
            return Self::FLAT;
        }
        let (kx, ky) = profile.curvature();
        Warp {
            k: (kx * scale, ky * scale),
            corner: 0.08,
        }
    }

    pub fn is_flat(&self) -> bool {
        *self == Self::FLAT
    }

    /// For the point `(u, v)` of the output, in [0, 1]²: the point of the flat picture seen there
    /// and how much of it, 1 inside the glass, 0 outside and a ramp across its edge, `pixel`
    /// being an output pixel's size in the same units.
    pub fn source(&self, u: f32, v: f32, pixel: (f32, f32)) -> (f32, f32, f32) {
        let (x, y) = (2.0 * u - 1.0, 2.0 * v - 1.0);
        let wx = x * (1.0 + self.k.0 * y * y);
        let wy = y * (1.0 + self.k.1 * x * x);
        let alpha = if self.is_flat() {
            1.0
        } else {
            // distance to the rounded rectangle |w| <= 1 (negative inside), in half-sizes
            let r = self.corner;
            let qx = wx.abs() - (1.0 - r);
            let qy = wy.abs() - (1.0 - r);
            let outside = qx.max(0.0).hypot(qy.max(0.0));
            let distance = outside + qx.max(qy).min(0.0) - r;
            // a pixel is 2 * pixel half-sizes
            (0.5 - distance / (2.0 * pixel.0.min(pixel.1))).clamp(0.0, 1.0)
        };
        ((wx + 1.0) / 2.0, (wy + 1.0) / 2.0, alpha)
    }
}

impl FromStr for CrtProfile {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "ctm" => Ok(Self::Ctm644),
            "trinitron" => Ok(Self::Trinitron),
            "gt65" | "gt64" => Ok(Self::Gt65),
            "mm12" => Ok(Self::Mm12),
            "atari" => Ok(Self::AtariColor),
            other => Err(format!("unknown CRT {other:?} (expected {})", Self::NAMES)),
        }
    }
}

/// Everything that distinguishes a tube from another. Lengths in `px` are
/// output pixels; `lines` are monitor scanlines.
#[derive(Clone, Debug, PartialEq)]
pub struct CrtParams {
    /// Output pixels per triad: a multiple of 3 (3 = one pixel per slot,
    /// as ACE-DL; 6 or 9 give slots their shape, at 4 times the cost).
    pub triad_px: usize,
    /// Output rows per scanline.
    pub rows_per_line: usize,
    /// Vertical period of the slots, in scanlines.
    pub slot_pitch_lines: f32,
    /// Size of a slot relative to its cell (width, height).
    pub slot_fill: (f32, f32),
    /// Corner radius, relative to the smaller half-size of the slot.
    pub corner_radius: f32,
    /// Vertical shift of odd triads, relative to the slot pitch.
    pub stagger: f32,
    /// Standard deviation of the beam spot across and along a scanline:
    /// horizontally in monitor pixels, vertically in scanlines.
    pub beam_sigma_h: f32,
    pub beam_sigma_v: f32,
    /// Growth in px of a slot at full beam intensity: bright pixels bleed
    /// over the slot's edges.
    pub spill: f32,
    /// Light of a slot reaching its two horizontal neighbours (glass).
    pub halation: f32,
    /// Brightness relative to [`Self::white_norm`]'s calibration, where a
    /// white field has the same mean light as without a mask. Each gun lights
    /// only a third of the surface, so 3 keeps the mean light of a picture
    /// (ACE-DL's 2.6): the brightest slots clip.
    pub gain: f32,
    /// A monochrome tube: the colour of its phosphor, as the light of each of the three primaries
    /// for a full white (the green of a GT65, the grey of an MM12). Every slot is lit by the
    /// luminance of the picture instead of the gun of its colour. `None`: a colour tube.
    pub phosphor: Option<[f32; 3]>,
    /// The light a black reaches, as a share of the light of a white field (as a camera or an eye
    /// takes it, the mask and the scanlines averaged): a tube never is black. The dark areas of the
    /// photographs of real CPCs on a CTM (the Shaker site's, 89 screens of the five CRTC types) are
    /// at 3.5 % of the light of the white of the same picture (half of the screens between 2.8 and
    /// 5.1 %), where bemu's black ink gave 0.1 %: the beam at the level of a black ink and the
    /// glass's glare. See [`Self::veil_light`].
    pub veil: f32,
    /// How much darker the edge of the picture is than its centre, the light falling as
    /// `1 - vignette * r²` with `r` 1 at the middle of an edge. Measured on the photographs: the
    /// white of a CTM's screen is 0.72 of the centre's at the middle of the edge, 0.62 at the
    /// corners' distance (a camera's lens takes its share of it). It is more than the photographs'
    /// 0.30 a unit of r² in light because the white of the model is clipped: the centre is above 1.
    pub vignette: f32,
    /// How strong the glow is, 1 as TuxAPE's. See [`GlowField`].
    pub glow: f32,
}

/// The floor under the glow: none, a black picture is black.
pub const VEIL: f32 = 0.0;

/// The glow's grid: a cell is 4 pixels of the source.
pub const GLOW_W: usize = SRC_WIDTH / 4;
pub const GLOW_H: usize = SRC_HEIGHT / 4;

/// The glow of what is bright on what is around it, as TuxAPE's view of a CTM has it (what it
/// measured on photographs: right beside a large bright area the dark is lit to a few hundredths
/// of it, still to a hundredth a third of the screen away): the picture blurred a little and a
/// great deal, each added with its weight. Sigmas in pixels of the source (3 and 80 mode 1
/// pixels of the 384 of the frame: the wide one is what the photographs ask for, see [`GLOW`]).
pub const GLOW_CLOSE: (f32, f32) = (6.0, 0.12);
pub const GLOW_WIDE: (f32, f32) = (160.0, 0.085);
/// How strong it is, for 1 as TuxAPE's weights. Checked on 89 photographs of real CTMs: with
/// these weights and a wide blur of 80 mode 1 pixels the dark areas of the photographs come out
/// at 3.46 % of the white's light, where they measure 3.50 %; fitting the two weights freely
/// gives 0.085 for the wide one, TuxAPE's own value (the data cannot tell the close one, whose
/// light is under a pixel of the edge of the bright).
pub const GLOW: f32 = 1.0;

/// The glow of a frame, in units of a white's light: the light of the source blurred two ways,
/// weighted, on a grid of [`GLOW_W`] × [`GLOW_H`] cells.
pub struct GlowField {
    cells: Vec<[f32; 3]>,
}

impl GlowField {
    pub fn new(src: &image::RgbImage) -> Self {
        let mut level = [0.0f32; 256];
        for (i, l) in level.iter_mut().enumerate() {
            *l = (i as f32 / 255.0).powi(2);
        }
        // the source's light, by cells
        let mut light = vec![[0.0f32; 3]; GLOW_W * GLOW_H];
        let (w, h) = (src.width() as usize, src.height() as usize);
        let raw = src.as_raw();
        for y in 0..h.min(SRC_HEIGHT) {
            for x in 0..w.min(SRC_WIDTH) {
                let cell = &mut light[(y / 4) * GLOW_W + x / 4];
                for c in 0..3 {
                    cell[c] += level[raw[(y * w + x) * 3 + c] as usize] / 16.0;
                }
            }
        }
        let close = Self::blur(&light, GLOW_W, GLOW_H, GLOW_CLOSE.0 / 4.0);
        // the wide one on a grid four times coarser, then back (it is smooth)
        let (cw, ch) = (GLOW_W / 4, GLOW_H / 4 + 1);
        let mut coarse = vec![[0.0f32; 3]; cw * ch];
        for y in 0..GLOW_H {
            for x in 0..GLOW_W {
                for c in 0..3 {
                    coarse[(y / 4) * cw + x / 4][c] += light[y * GLOW_W + x][c] / 16.0;
                }
            }
        }
        let wide = Self::blur(&coarse, cw, ch, GLOW_WIDE.0 / 16.0);
        let mut cells = vec![[0.0f32; 3]; GLOW_W * GLOW_H];
        for y in 0..GLOW_H {
            for x in 0..GLOW_W {
                let at = |gx: f32, gy: f32| {
                    let (gx, gy) = (gx.clamp(0.0, (cw - 1) as f32), gy.clamp(0.0, (ch - 1) as f32));
                    let (x0, y0) = (gx.floor() as usize, gy.floor() as usize);
                    let (x1, y1) = ((x0 + 1).min(cw - 1), (y0 + 1).min(ch - 1));
                    let (fx, fy) = (gx - x0 as f32, gy - y0 as f32);
                    let mut o = [0.0; 3];
                    for c in 0..3 {
                        let top = wide[y0 * cw + x0][c] * (1.0 - fx) + wide[y0 * cw + x1][c] * fx;
                        let bottom = wide[y1 * cw + x0][c] * (1.0 - fx) + wide[y1 * cw + x1][c] * fx;
                        o[c] = top * (1.0 - fy) + bottom * fy;
                    }
                    o
                };
                let wide = at((x as f32 + 0.5) / 4.0 - 0.5, (y as f32 + 0.5) / 4.0 - 0.5);
                for c in 0..3 {
                    cells[y * GLOW_W + x][c] =
                        GLOW_CLOSE.1 * close[y * GLOW_W + x][c] + GLOW_WIDE.1 * wide[c];
                }
            }
        }
        Self { cells }
    }

    /// Gaussian blur (zero outside the picture) with `sigma` cells.
    fn blur(cells: &[[f32; 3]], w: usize, h: usize, sigma: f32) -> Vec<[f32; 3]> {
        let radius = (sigma * 4.0).ceil() as isize;
        let mut kernel: Vec<f32> = (-radius..=radius)
            .map(|d| (-((d * d) as f32) / (2.0 * sigma * sigma)).exp())
            .collect();
        let sum: f32 = kernel.iter().sum();
        kernel.iter_mut().for_each(|k| *k /= sum);
        let mut tmp = vec![[0.0f32; 3]; w * h];
        let mut out = vec![[0.0f32; 3]; w * h];
        for y in 0..h {
            for x in 0..w {
                for (k, d) in kernel.iter().zip(-radius..=radius) {
                    let sx = x as isize + d;
                    if (0..w as isize).contains(&sx) {
                        for c in 0..3 {
                            tmp[y * w + x][c] += k * cells[y * w + sx as usize][c];
                        }
                    }
                }
            }
        }
        for y in 0..h {
            for x in 0..w {
                for (k, d) in kernel.iter().zip(-radius..=radius) {
                    let sy = y as isize + d;
                    if (0..h as isize).contains(&sy) {
                        for c in 0..3 {
                            out[y * w + x][c] += k * tmp[sy as usize * w + x][c];
                        }
                    }
                }
            }
        }
        out
    }

    /// The glow at the place `(u, v)` of the picture (0 to 1 each), bilinear between the cells.
    pub fn at(&self, u: f32, v: f32) -> [f32; 3] {
        let gx = (u * GLOW_W as f32 - 0.5).clamp(0.0, (GLOW_W - 1) as f32);
        let gy = (v * GLOW_H as f32 - 0.5).clamp(0.0, (GLOW_H - 1) as f32);
        let (x0, y0) = (gx.floor() as usize, gy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(GLOW_W - 1), (y0 + 1).min(GLOW_H - 1));
        let (fx, fy) = (gx - x0 as f32, gy - y0 as f32);
        let mut o = [0.0; 3];
        for c in 0..3 {
            let top = self.cells[y0 * GLOW_W + x0][c] * (1.0 - fx) + self.cells[y0 * GLOW_W + x1][c] * fx;
            let bottom = self.cells[y1 * GLOW_W + x0][c] * (1.0 - fx) + self.cells[y1 * GLOW_W + x1][c] * fx;
            o[c] = top * (1.0 - fy) + bottom * fy;
        }
        o
    }

    /// The cells as texels of four floats (the fourth is 0), row by row.
    pub fn texels(&self) -> Vec<f32> {
        self.cells.iter().flat_map(|c| [c[0], c[1], c[2], 0.0]).collect()
    }
}

/// What the real CTMs' photographs show of the edges, see [`CrtParams::vignette`].
pub const VIGNETTE: f32 = 0.55;

/// The luminance of a pixel's light: the green screen of the CPC shows the colours as the
/// luminance they would have on a black and white television.
pub const LUMA: [f32; 3] = [0.299, 0.587, 0.114];

impl CrtParams {
    pub fn from_profile(profile: CrtProfile) -> Self {
        let base = Self {
            triad_px: 3,
            rows_per_line: 4,
            slot_pitch_lines: 1.0,
            slot_fill: (0.82, 0.72),
            corner_radius: 0.45,
            stagger: 0.5,
            beam_sigma_h: 0.5,
            beam_sigma_v: 0.38,
            spill: 0.5,
            halation: 0.10,
            gain: 3.0,
            phosphor: None,
            veil: VEIL,
            vignette: VIGNETTE,
            glow: GLOW,
        };
        match profile {
            CrtProfile::Ctm644 => base,
            CrtProfile::Trinitron => Self {
                slot_fill: (0.9, 1.2),
                stagger: 0.0,
                spill: 0.4,
                gain: 3.0,
                ..base
            },
            // the CTM's slot mask and grid in one phosphor: every slot is lit, so a third of the
            // colour tubes' gain
            CrtProfile::Gt65 => Self {
                gain: 1.0,
                // a P31 green phosphor: green, a little blue and less red
                phosphor: Some([0.12, 1.0, 0.16]),
                ..base
            },
            CrtProfile::Mm12 => Self {
                gain: 1.0,
                phosphor: Some([0.78, 0.80, 0.84]),
                ..base
            },
            CrtProfile::AtariColor => Self {
                slot_fill: (1.0, 1.2),
                stagger: 0.0,
                spill: 0.4,
                beam_sigma_v: 0.30,
                gain: 3.0,
                ..base
            },
        }
    }

    /// The same tube drawn `level` times larger in both directions (`level` >= 1): a picture to
    /// shrink to a large window, the slots and the scanlines finer relative to it.
    pub fn scaled(&self, level: usize) -> Self {
        let level = level.max(1);
        Self {
            triad_px: self.triad_px * level,
            rows_per_line: self.rows_per_line * level,
            ..self.clone()
        }
    }

    /// The mean light of a white field, worked out as the renderers do: each slot's light, the
    /// halation of its neighbours, the clipping of what is too bright, averaged over a period of
    /// the pattern and over the three guns (the one phosphor of a mono tube).
    /// The light, added to every slot whatever its gun, that makes a black the [`Self::veil`] of a
    /// white field's mean light.
    pub fn veil_light(&self) -> f32 {
        if self.veil <= 0.0 {
            return 0.0;
        }
        self.veil * self.white_light()
    }

    /// The light of the glow where the picture is white (see [`GlowField`]).
    pub fn glow_light(&self) -> f32 {
        self.glow * self.white_light()
    }

    /// The mean light of a white field, worked out as the renderers do: each slot's light, the
    /// halation of its neighbours, the clipping of what is too bright, averaged over a period of
    /// the pattern and over the three guns (the one phosphor of a mono tube).
    pub fn white_light(&self) -> f32 {
        let scale = self.gain * self.white_norm();
        let (slot, pitch) = (self.slot_px() as isize, self.pitch_rows());
        let period = self.rows_per_line * pitch;
        let width = 2 * self.triad_px;
        let (h, centre) = (self.halation, 1.0 - 2.0 * self.halation);
        let gauss = |d: f32| (-d * d / (2.0 * self.beam_sigma_v * self.beam_sigma_v)).exp();
        let mut total = 0.0;
        let guns: &[usize] = if self.phosphor.is_some() { &[1] } else { &[0, 1, 2] };
        for y in 0..period {
            let f = (y as f32 + 0.5) / self.rows_per_line as f32 - 0.5;
            let line = f.floor();
            let level = gauss(f - line) + gauss(line + 1.0 - f);
            let light: Vec<f32> = (0..width)
                .map(|x| level * coverage(self.slot_distance(x, y), self.spill, level) * scale)
                .collect();
            for &gun in guns {
                // the light of the gun at output column `x`: lit slots only
                let own = |x: isize| {
                    let x = x.rem_euclid(width as isize) as usize;
                    if self.channel(x) == gun {
                        light[x]
                    } else {
                        0.0
                    }
                };
                for x in 0..width as isize {
                    total += (centre * own(x) + h * (own(x - slot) + own(x + slot))).min(1.0);
                }
            }
        }
        total / (period * width * guns.len()) as f32
    }

    /// Width of a slot, in output pixels.
    pub fn slot_px(&self) -> usize {
        (self.triad_px / 3).max(1)
    }

    /// Vertical period of the slots, in output rows.
    pub fn pitch_rows(&self) -> usize {
        ((self.rows_per_line as f32 * self.slot_pitch_lines).round() as usize).max(1)
    }

    pub fn output_size(&self) -> (usize, usize) {
        (
            TRIADS_ACROSS * self.triad_px,
            SRC_HEIGHT * self.rows_per_line,
        )
    }

    /// Output pixels per monitor pixel.
    pub fn x_ratio(&self) -> f32 {
        (TRIADS_ACROSS * self.triad_px) as f32 / SRC_WIDTH as f32
    }

    /// The phosphor (0 = red, 1 = green, 2 = blue) of output column `x`; a monochrome tube has the
    /// one (called green here, its colour is [`Self::phosphor`]).
    pub fn channel(&self, x: usize) -> usize {
        if self.phosphor.is_some() {
            1
        } else {
            (x / self.slot_px()) % 3
        }
    }

    /// Factor making the mean light of a full-white field equal to what the
    /// monitor sends: the mask and the dark gaps between scanlines hide a
    /// part of it. Computed over one period of the pattern.
    pub fn white_norm(&self) -> f32 {
        let pitch = self.pitch_rows();
        let period = self.rows_per_line * pitch;
        let gauss = |d: f32| (-d * d / (2.0 * self.beam_sigma_v * self.beam_sigma_v)).exp();
        let mut total = 0.0;
        for y in 0..period {
            let f = (y as f32 + 0.5) / self.rows_per_line as f32 - 0.5;
            let line = f.floor();
            let level = gauss(f - line) + gauss(line + 1.0 - f);
            for x in 0..2 * self.triad_px {
                total += level * coverage(self.slot_distance(x, y), self.spill, level);
            }
        }
        (period * 2 * self.triad_px) as f32 / total
    }

    /// Signed distance, in px, from the centre of output pixel (`x`, `y`) to
    /// the edge of its slot: negative inside, 0 on the edge. Periodic in `y`
    /// with [`Self::pitch_rows`], and in `x` with two triads.
    pub fn slot_distance(&self, x: usize, y: usize) -> f32 {
        let slot_w = self.slot_px() as f32;
        let pitch = self.pitch_rows() as f32;
        let slot = x / self.slot_px();
        let triad = slot / 3;
        let shift = if triad % 2 == 1 {
            self.stagger * pitch
        } else {
            0.0
        };
        // position in the slot's cell, the cell's centre being the origin
        let px = (x as f32 + 0.5) - (slot as f32 * slot_w) - slot_w / 2.0;
        let py = ((y as f32 + 0.5 - shift).rem_euclid(pitch)) - pitch / 2.0;
        let half = (
            self.slot_fill.0 * slot_w / 2.0,
            self.slot_fill.1 * pitch / 2.0,
        );
        let radius = (self.corner_radius * half.0.min(half.1)).max(0.0);
        rounded_box_distance(px, py, half.0, half.1, radius)
    }
}

/// Signed distance to a rounded rectangle of half-sizes (`hx`, `hy`),
/// corners rounded by `r`, centred on the origin.
pub fn rounded_box_distance(px: f32, py: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let qx = px.abs() - (hx - r);
    let qy = py.abs() - (hy - r);
    let outside = (qx.max(0.0) * qx.max(0.0) + qy.max(0.0) * qy.max(0.0)).sqrt();
    outside + qx.max(qy).min(0.0) - r
}

/// Fraction of output pixel covered by its slot, grown by `spill * level`.
#[inline]
pub fn coverage(distance: f32, spill: f32, level: f32) -> f32 {
    (0.5 - (distance - spill * level)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn profiles_parse() {
        for name in CrtProfile::NAMES.split('|') {
            assert!(name.parse::<CrtProfile>().is_ok(), "{name}");
        }
        assert!("crt".parse::<CrtProfile>().is_err());
    }

    #[test]
    fn ctm_output_is_ace_dl_s() {
        // 768 * 1.875 wide, 4 rows per scanline
        assert_eq!(CrtProfile::Ctm644.params().output_size(), (1440, 1088));
    }

    #[test]
    fn channels_repeat_red_green_blue() {
        let p = CrtProfile::Ctm644.params();
        assert_eq!(
            (0..7).map(|x| p.channel(x)).collect::<Vec<_>>(),
            [0, 1, 2, 0, 1, 2, 0]
        );
        let wide = CrtParams { triad_px: 6, ..p };
        assert_eq!(
            (0..6).map(|x| wide.channel(x)).collect::<Vec<_>>(),
            [0, 0, 1, 1, 2, 2]
        );
    }

    #[test]
    fn mask_is_periodic_and_staggered() {
        let p = CrtParams {
            triad_px: 9,
            ..CrtProfile::Ctm644.params()
        };
        let pitch = p.pitch_rows();
        for x in 0..30 {
            for y in 0..pitch {
                assert_eq!(p.slot_distance(x, y), p.slot_distance(x, y + 3 * pitch));
                // two triads further, the same
                assert_eq!(
                    p.slot_distance(x, y),
                    p.slot_distance(x + 2 * p.triad_px, y)
                );
            }
        }
        // odd triads are shifted by half a pitch (slot 1 of triad 0 is at x=4,
        // slot 1 of triad 1 at x=4 + triad_px)
        for y in 0..pitch {
            let odd = p.slot_distance(4 + p.triad_px, y);
            let even_shifted = p.slot_distance(4, y + pitch / 2);
            assert!(
                (odd - even_shifted).abs() < 1e-4,
                "row {y}: {odd} vs {even_shifted}"
            );
        }
    }

    #[test]
    fn grille_has_no_vertical_gap() {
        let p = CrtProfile::Trinitron.params();
        for y in 0..p.pitch_rows() {
            assert!(p.slot_distance(1, y) < 0.0);
        }
    }

    #[test]
    fn calibrated_gain_makes_up_for_the_mask() {
        let ctm = CrtProfile::Ctm644.params().white_norm();
        let grille = CrtProfile::Trinitron.params().white_norm();
        assert!(ctm > grille, "{ctm} {grille}");
    }

    #[test]
    fn coverage_grows_with_level() {
        assert!(coverage(0.5, 0.8, 1.0) > coverage(0.5, 0.8, 0.0));
        assert_eq!(coverage(-3.0, 0.0, 0.0), 1.0);
        assert_eq!(coverage(3.0, 0.0, 0.0), 0.0);
    }
}
