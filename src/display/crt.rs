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
    /// Amstrad GT65: no mask structure, visible scanlines only.
    Gt65,
    /// Atari colour monitor: grille with strong scanlines.
    AtariColor,
}

impl CrtProfile {
    pub const NAMES: &'static str = "ctm|trinitron|gt65|atari";

    pub fn params(self) -> CrtParams {
        CrtParams::from_profile(self)
    }

    /// The bulge of the tube's glass, `(kx, ky)` of [`Warp`] at curvature 1: the 14" monitors
    /// of the CPC (CTM644, GT65) curve both ways, the Trinitron's cylinder only across, the TV the
    /// most. Eyeballed, not measured.
    pub fn curvature(self) -> (f32, f32) {
        match self {
            CrtProfile::Ctm644 | CrtProfile::Gt65 => (0.025, 0.035),
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
            "gt65" => Ok(Self::Gt65),
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
}

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
            CrtProfile::Gt65 => Self {
                slot_fill: (1.0, 1.2),
                stagger: 0.0,
                spill: 0.0,
                halation: 0.0,
                beam_sigma_v: 0.42,
                gain: 3.0,
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

    /// The phosphor (0 = red, 1 = green, 2 = blue) of output column `x`.
    pub fn channel(&self, x: usize) -> usize {
        (x / self.slot_px()) % 3
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
        let grille = CrtProfile::Gt65.params().white_norm();
        assert!(ctm > grille, "{ctm} {grille}");
    }

    #[test]
    fn coverage_grows_with_level() {
        assert!(coverage(0.5, 0.8, 1.0) > coverage(0.5, 0.8, 0.0));
        assert_eq!(coverage(-3.0, 0.0, 0.0), 1.0);
        assert_eq!(coverage(3.0, 0.0, 0.0), 0.0);
    }
}
