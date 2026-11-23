//! The CRT model on the GPU, written once as a CubeCL kernel (native Vulkan / Metal / DX12 and the
//! browser's WebGPU): one thread per output pixel, the model evaluated per pixel (the
//! beam's Gaussian taps at the slot's centre, through the glass; the slot mask's coverage; the
//! halation of the neighbouring slots; the veil, the glow, the vignette; the glass's edge; the
//! gamma).
//!
//! The arguments are plain buffers: the monitor's frame (its RGB bytes as they are, `src_w` x `src_h`
//! pixels), the glow cells (four floats a cell; worked out on the GPU too, by [`Pipeline`], as one
//! pipeline: see [`Pipeline`]), the parameters of [`pack`], and the
//! picture (`0xRRGGBB` words, row after row).
use super::crt::{
    gauss_taps, CrtParams, Warp, GLOW_CH, GLOW_CLOSE, GLOW_CW, GLOW_H, GLOW_W, GLOW_WIDE, SRC_HEIGHT,
    SRC_WIDTH,
};
use cubecl::prelude::*;
use image::RgbImage;
use std::borrow::Cow;

// parameter slots
pub const OUT_W: usize = 0;
pub const OUT_H: usize = 1;
pub const SLOT_PX: usize = 2;
pub const ROWS: usize = 3;
pub const PITCH: usize = 4;
pub const FILL_X: usize = 5;
pub const FILL_Y: usize = 6;
pub const CORNER: usize = 7;
pub const STAGGER: usize = 8;
pub const SIGMA_H: usize = 9;
pub const SIGMA_V: usize = 10;
pub const SPILL: usize = 11;
pub const HALATION: usize = 12;
pub const SCALE: usize = 13;
pub const X_RATIO: usize = 14;
pub const MONO: usize = 15;
pub const TINT: usize = 16; // 3 values
pub const VEIL: usize = 19;
pub const VIGNETTE: usize = 20;
pub const GLOW: usize = 21;
pub const WARP_KX: usize = 22;
pub const WARP_KY: usize = 23;
pub const WARP_CORNER: usize = 24;
pub const EDGED: usize = 25;
pub const BLUE_SHIFT: usize = 26;
pub const WHITE: usize = 27; // 3 values

// the tables of `luts` (see `Pipeline::new`): the Gaussian weights of 1024 phases, and the slot mask
pub const PHASES: usize = 1024;
/// The scanline's weights are tabulated for this many levels of light.
pub const V_LEVELS: usize = 16;
/// The horizontal weights of the red and green guns, then of the blue (wider), the scanline's
/// weights by level, then the slot mask.
pub const HB_OFF: usize = PHASES * 4;
pub const V_OFF: usize = 2 * PHASES * 4;
pub const M_OFF: usize = V_OFF + V_LEVELS * PHASES;
/// The parameters' count.
pub const PARAMS: usize = 30;
/// What `crt` looks up per output column (floats): the phosphor of the slot and of its two
/// neighbours, their halation weights (0 beyond the picture), the glow's cells and weight, the
/// column's place on the picture (`dx` and `dx^2`); and per output row: the glow's cells and
/// weight, `dy` and `dy^2`. Built once (`axis_tables`): no division, no `floor`, no clamp per pixel.
pub const COL: usize = 12;
/// The source lines `hfilter` covers: from the one above the picture to two below it.
pub const HF_LINES: usize = SRC_HEIGHT + 4;
pub const ROW: usize = 8;
/// The frame as linear light, padded with zeros: `PAD` pixels around the monitor's 768 x 272, so
/// that a beam's taps beyond the picture read black without any test (a read outside is clamped
/// into the border).
pub const PAD: usize = 4;
pub const LIN_W: usize = SRC_WIDTH + 2 * PAD;
pub const LIN_H: usize = SRC_HEIGHT + 2 * PAD;

#[cube]
fn gauss(d: f32, sigma: f32) -> f32 {
    f32::exp(-d * d / (2.0 * sigma * sigma))
}

/// Byte `at` of the frame (its RGB bytes as they are, four to a word).
#[cube]
fn byte(frame: &[u32], at: u32) -> f32 {
    ((frame[(at >> 2u32) as usize] >> ((at & 3u32) * 8u32)) & 255u32) as f32
}

/// Linear light of gun `c` of the source pixel (i, j), from the padded linear frame (`to_linear`):
/// black beyond the picture, where the index is clamped into the zeros of the border. A mono tube
/// reads the luminance (the fourth float of a pixel) whatever the gun.
#[cube]
fn gun(lin: &[f32], pr: &[f32], i: i32, j: i32, c: u32) -> f32 {
    let pi = i32::clamp(i + 4, 0, 775) as u32;
    let pj = i32::clamp(j + 4, 0, 279) as u32;
    let mut ch = c;
    if pr[MONO] > 0.5 {
        ch = 3u32;
    }
    lin[((pj * 776u32 + pi) * 4u32 + ch) as usize]
}

#[cube]
fn slot_channel(pr: &[f32], x: i32) -> u32 {
    let mut c = ((x / pr[SLOT_PX] as i32) % 3) as u32;
    if pr[MONO] > 0.5 {
        c = 1u32;
    }
    c
}

/// The beam's horizontal filter on source line `j` for the slot whose centre reads at `wx` (0..1 of
/// the picture), in the phosphor `c`: the four taps of the tables at the beam's fractional place.
#[cube]
fn hrow(lin: &[f32], luts: &[f32], pr: &[f32], wx: f32, c: u32, j: i32) -> f32 {
    // the blue gun lands a little to the right of the others (the beam is read to its left)
    let mut v = wx * pr[OUT_W] / pr[X_RATIO] - 0.5;
    let mut table = 0usize;
    if c == 2u32 && pr[MONO] < 0.5 {
        v = v - pr[BLUE_SHIFT];
        table = HB_OFF;
    }
    let fl = f32::floor(v);
    let first = fl as i32 - 1;
    let hp = table + u32::min(((v - fl) * 1024.0) as u32, 1023u32) as usize * 4;
    luts[hp] * gun(lin, pr, first, j, c)
        + luts[hp + 1] * gun(lin, pr, first + 1, j, c)
        + luts[hp + 2] * gun(lin, pr, first + 2, j, c)
        + luts[hp + 3] * gun(lin, pr, first + 3, j, c)
}

/// [`hrow`] for the lines `j` and `j + 1` (the beam's weights worked out once).
#[cube]
fn hpair(lin: &[f32], luts: &[f32], pr: &[f32], wx: f32, c: u32, j: i32) -> (f32, f32) {
    let mut v = wx * pr[OUT_W] / pr[X_RATIO] - 0.5;
    let mut table = 0usize;
    if c == 2u32 && pr[MONO] < 0.5 {
        v = v - pr[BLUE_SHIFT];
        table = HB_OFF;
    }
    let fl = f32::floor(v);
    let first = fl as i32 - 1;
    let hp = table + u32::min(((v - fl) * 1024.0) as u32, 1023u32) as usize * 4;
    let (w0, w1, w2, w3) = (luts[hp], luts[hp + 1], luts[hp + 2], luts[hp + 3]);
    let la = w0 * gun(lin, pr, first, j, c)
        + w1 * gun(lin, pr, first + 1, j, c)
        + w2 * gun(lin, pr, first + 2, j, c)
        + w3 * gun(lin, pr, first + 3, j, c);
    let lb = w0 * gun(lin, pr, first, j + 1, c)
        + w1 * gun(lin, pr, first + 1, j + 1, c)
        + w2 * gun(lin, pr, first + 2, j + 1, c)
        + w3 * gun(lin, pr, first + 3, j + 1, c);
    (la, lb)
}

/// The light of a slot from the two source lines' filtered values `la`, `lb` around the beam
/// (`phase` of 1024 between them): the scanline's width by level, the slot mask's coverage.
#[cube]
fn shade(luts: &[f32], pr: &[f32], la: f32, lb: f32, phase: usize, x: usize, y: usize) -> f32 {
    // a scanline is wider the brighter it is: weights by the level of each line
    let lv0 = (f32::min(la, 1.0) * 15.0 + 0.5) as usize;
    let lv1 = (f32::min(lb, 1.0) * 15.0 + 0.5) as usize;
    let a = luts[V_OFF + lv0 * 1024 + phase];
    let b = luts[V_OFF + lv1 * 1024 + 1023 - phase];
    let level = a * la + b * lb;
    let period = 6 * pr[SLOT_PX] as usize;
    let distance = luts[M_OFF + (y % pr[PITCH] as usize) * period + x % period];
    let cov = f32::clamp(0.5 - (distance - pr[SPILL] * level), 0.0, 1.0);
    level * cov * pr[SCALE]
}

/// The light of the slot at output column `x`, row `y`, in its own phosphor; none outside the
/// picture. The Gaussian weights and the slot mask come from the tables `luts` (a phase of 1/1024
/// of a pixel: below the rounding of the picture). The general glass: the raster bends under the
/// fixed mask, so the slot reads where its centre's warped place is.
#[cube]
fn slot_light(lin: &[f32], luts: &[f32], pr: &[f32], x: usize, y: usize) -> f32 {
    let sw = pr[SLOT_PX] as usize;
    let slot = x / sw;
    let c = slot_channel(pr, x as i32);
    let ow = pr[OUT_W];
    let oh = pr[OUT_H];
    let qx = ((slot as f32 + 0.5) * sw as f32 / ow) * 2.0 - 1.0;
    let qy = ((y as f32 + 0.5) / oh) * 2.0 - 1.0;
    let wx = (qx * (1.0 + pr[WARP_KX] * qy * qy) + 1.0) / 2.0;
    let wy = (qy * (1.0 + pr[WARP_KY] * qx * qx) + 1.0) / 2.0;
    let f = wy * oh / pr[ROWS] - 0.5;
    let line = f32::floor(f) as i32;
    let (la, lb) = hpair(lin, luts, pr, wx, c, line);
    let phase = u32::min(((f - line as f32) * 1024.0) as u32, 1023u32) as usize;
    shade(luts, pr, la, lb, phase, x, y)
}

/// The horizontally filtered value of every slot on every source line (`HF_LINES` from the line
/// above the picture), for the flat glass: where the beam reads depends on the slot's column alone,
/// so the filter is done once per slot and line, not per pixel.
#[cube(launch_unchecked)]
pub fn hfilter(lin: &[f32], luts: &[f32], pr: &[f32], hf: &mut [f32]) {
    let s = ABSOLUTE_POS_X as usize;
    let r = ABSOLUTE_POS_Y as usize;
    let sw = pr[SLOT_PX] as usize;
    let ns = (pr[OUT_W] as usize + sw - 1) / sw;
    if s < ns && r < 276 && pr[WARP_KX] == 0.0 && pr[WARP_KY] == 0.0 {
        let c = slot_channel(pr, (s * sw) as i32);
        let qx = ((s as f32 + 0.5) * sw as f32 / pr[OUT_W]) * 2.0 - 1.0;
        hf[r * ns + s] = hrow(lin, luts, pr, (qx + 1.0) / 2.0, c, r as i32 - 1);
    }
}

/// The light of the slot of every pixel (its own phosphor), once: the halation of the neighbours is
/// then a sum of three of them. A flat glass reads the filtered lines (`hfilter`) at the slot and
/// line the column and row tables name; a curved one works the beam out per pixel.
#[cube(launch_unchecked)]
pub fn slots(lin: &[f32], luts: &[f32], pr: &[f32], hf: &[f32], cols: &[f32], rows: &[f32], lights: &mut [f32]) {
    let x = ABSOLUTE_POS_X as usize;
    let y = ABSOLUTE_POS_Y as usize;
    let w = pr[OUT_W] as usize;
    if x < w && y < pr[OUT_H] as usize {
        if pr[WARP_KX] == 0.0 && pr[WARP_KY] == 0.0 {
            let at = rows[y * 8 + 5] as usize + cols[x * 12 + 11] as usize;
            let next = rows[y * 8 + 7] as usize + cols[x * 12 + 11] as usize;
            lights[y * w + x] = shade(luts, pr, hf[at], hf[next], rows[y * 8 + 6] as usize, x, y);
        } else {
            lights[y * w + x] = slot_light(lin, luts, pr, x, y);
        }
    }
}

/// `l` if the slot's phosphor is `want`.
#[cube]
fn only(c: f32, want: f32, l: f32) -> f32 {
    let mut out = 0.0f32;
    if c == want {
        out = l;
    }
    out
}

/// One channel of the picture, encoded: the light `v` and what is added to it (veil, glow), dimmed
/// at the picture's edges (`fall`) and by the glass's edge (`alpha`), through the gamma.
#[cube]
fn encode(v: f32, add: f32, fall: f32, alpha: f32) -> u32 {
    (f32::sqrt(f32::clamp((v + add) * fall, 0.0, 1.0)) * alpha * 255.0 + 0.5) as u32
}

#[cube(launch_unchecked)]
pub fn crt(lights: &[f32], glow: &[f32], pr: &[f32], cols: &[f32], rows: &[f32], out: &mut [u32]) {
    let x = ABSOLUTE_POS_X as usize;
    let y = ABSOLUTE_POS_Y as usize;
    let w = pr[OUT_W] as usize;
    if x < w && y < pr[OUT_H] as usize {
        let c = x * 12;
        let q = y * 8;
        // the slot of the pixel and its two neighbours (halation): the weight of a neighbour beyond
        // the picture is 0, so the read is just kept in the row
        let sw = pr[SLOT_PX] as usize;
        let row = y * w;
        let xl = if x >= sw { x - sw } else { 0usize };
        let xr = if x + sw < w { x + sw } else { w - 1 };
        let ll = cols[c + 3] * lights[row + xl];
        let lc = cols[c + 4] * lights[row + x];
        let lr = cols[c + 5] * lights[row + xr];
        let (cl, cc, cr) = (cols[c], cols[c + 1], cols[c + 2]);
        let mut r = only(cl, 0.0, ll) + only(cc, 0.0, lc) + only(cr, 0.0, lr);
        let mut g = only(cl, 1.0, ll) + only(cc, 1.0, lc) + only(cr, 1.0, lr);
        let mut b = only(cl, 2.0, ll) + only(cc, 2.0, lc) + only(cr, 2.0, lr);
        // the veil and the glow of the picture around (a mono tube: of its luminance, in its colour):
        // the glow's four cells and weights from the tables
        let (fx, fy) = (cols[c + 8], rows[q + 2]);
        let x0 = cols[c + 6] as usize;
        let x1 = cols[c + 7] as usize;
        let y0 = rows[q] as usize * 192;
        let y1 = rows[q + 1] as usize * 192;
        let (a00, a01, a10, a11) = ((y0 + x0) * 4, (y0 + x1) * 4, (y1 + x0) * 4, (y1 + x1) * 4);
        let gr = (glow[a00] * (1.0 - fx) + glow[a01] * fx) * (1.0 - fy) + (glow[a10] * (1.0 - fx) + glow[a11] * fx) * fy;
        let gg = (glow[a00 + 1] * (1.0 - fx) + glow[a01 + 1] * fx) * (1.0 - fy) + (glow[a10 + 1] * (1.0 - fx) + glow[a11 + 1] * fx) * fy;
        let gb = (glow[a00 + 2] * (1.0 - fx) + glow[a01 + 2] * fx) * (1.0 - fy) + (glow[a10 + 2] * (1.0 - fx) + glow[a11 + 2] * fx) * fy;
        let mut ar = pr[VEIL] + pr[GLOW] * gr;
        let mut ag = pr[VEIL] + pr[GLOW] * gg;
        let mut ab = pr[VEIL] + pr[GLOW] * gb;
        if pr[MONO] > 0.5 {
            let s = ll + lc + lr;
            let luma = 0.299 * (ar - pr[VEIL]) + 0.587 * (ag - pr[VEIL]) + 0.114 * (ab - pr[VEIL]);
            r = pr[TINT] * s;
            g = pr[TINT + 1] * s;
            b = pr[TINT + 2] * s;
            ar = pr[TINT] * (pr[VEIL] + luma);
            ag = pr[TINT + 1] * (pr[VEIL] + luma);
            ab = pr[TINT + 2] * (pr[VEIL] + luma);
        }
        let (dx, dx2, dy, dy2) = (cols[c + 9], cols[c + 10], rows[q + 3], rows[q + 4]);
        let fall = f32::max(1.0 - pr[VIGNETTE] * (dx2 + dy2), 0.0);
        // the glass's rounded edge, smoothed over about a pixel: none for a flat glass, and a pixel
        // well inside it (the common case) needs no square root
        let mut alpha = 1.0f32;
        if pr[EDGED] > 0.5 {
            let wx = dx * (1.0 + pr[WARP_KX] * dy2);
            let wy = dy * (1.0 + pr[WARP_KY] * dx2);
            let ex = f32::abs(wx) - (1.0 - pr[WARP_CORNER]);
            let ey = f32::abs(wy) - (1.0 - pr[WARP_CORNER]);
            let pixel = 2.0 / f32::max(pr[OUT_W], pr[OUT_H]);
            if ex >= 0.0 - pixel || ey >= 0.0 - pixel {
                let ox = f32::max(ex, 0.0);
                let oy = f32::max(ey, 0.0);
                let distance = f32::sqrt(ox * ox + oy * oy) + f32::min(f32::max(ex, ey), 0.0) - pr[WARP_CORNER];
                alpha = f32::clamp(0.5 - distance / pixel, 0.0, 1.0);
            }
        }
        out[row + x] = (encode(r, ar, fall * pr[WHITE], alpha) << 16u32)
            | (encode(g, ag, fall * pr[WHITE + 1], alpha) << 8u32)
            | encode(b, ab, fall * pr[WHITE + 2], alpha);
    }
}

// ---- the glow of the frame, on the GPU (`GlowField::new` of crt.rs, step for step) ----

/// The cells of the glow's two grids, in one buffer (`FINE` fine cells, then `COARSE` coarse ones,
/// four floats each): the mean light of the 4 x 4 source pixels of a fine cell, and of the 16 x 16
/// of a coarse one (the frame's pixels beyond its size are black).
#[cube(launch_unchecked)]
pub fn glow_cells(frame: &[u32], grid: &mut [f32], lin: &mut [f32], sw: u32, sh: u32) {
    let i = ABSOLUTE_POS;
    if i >= 13920 && i < 231200 {
        // the same launch also makes the padded linear frame (nothing here reads the other half)
        let k = (i - 13920) as u32;
        let px = (k % 776u32) as i32 - 4;
        let py = (k / 776u32) as i32 - 4;
        let mut r = 0.0f32;
        let mut g = 0.0f32;
        let mut b = 0.0f32;
        if px >= 0 && py >= 0 && (px as u32) < sw && (py as u32) < sh {
            let at = (py as u32 * sw + px as u32) * 3u32;
            let lr = byte(frame, at) / 255.0;
            let lg = byte(frame, at + 1u32) / 255.0;
            let lb = byte(frame, at + 2u32) / 255.0;
            r = lr * lr;
            g = lg * lg;
            b = lb * lb;
        }
        let at = (k * 4u32) as usize;
        lin[at] = r;
        lin[at + 1] = g;
        lin[at + 2] = b;
        lin[at + 3] = 0.299 * r + 0.587 * g + 0.114 * b;
    }
    if i < 13920 {
        let mut cx = (i % 192) as u32;
        let mut cy = (i / 192) as u32;
        let mut size = 4u32;
        if i >= 13056 {
            let j = (i - 13056) as u32;
            cx = j % 48u32;
            cy = j / 48u32;
            size = 16u32;
        }
        let mut r = 0.0f32;
        let mut g = 0.0f32;
        let mut b = 0.0f32;
        for dy in 0..size {
            for dx in 0..size {
                let x = cx * size + dx;
                let y = cy * size + dy;
                if x < sw && y < sh {
                    let at = (y * sw + x) * 3u32;
                    let lr = byte(frame, at) / 255.0;
                    let lg = byte(frame, at + 1u32) / 255.0;
                    let lb = byte(frame, at + 2u32) / 255.0;
                    r += lr * lr;
                    g += lg * lg;
                    b += lb * lb;
                }
            }
        }
        let n = (size * size) as f32;
        grid[i * 4] = r / n;
        grid[i * 4 + 1] = g / n;
        grid[i * 4 + 2] = b / n;
        grid[i * 4 + 3] = 0.0;
    }
}

/// One pass of the separable Gaussian blurs (zero outside the grid) of both grids at once: the fine
/// grid (192 x 68) with the close taps, the coarse one (48 x 18) with the wide ones, along the rows
/// (`horizontal` 1) or the columns. The taps are `taps[..]` (close first, then wide, from `offset`).
#[cube(launch_unchecked)]
pub fn glow_blur(
    src: &[f32],
    taps: &[f32],
    dst: &mut [f32],
    horizontal: u32,
    close_radius: u32,
    wide_radius: u32,
    offset: u32,
) {
    let i = ABSOLUTE_POS;
    if i < 13920 {
        let mut base = 0u32;
        let mut w = 192u32;
        let mut h = 68u32;
        let mut radius = close_radius;
        let mut first = 0u32;
        let mut j = i as u32;
        if i >= 13056 {
            base = 13056u32;
            w = 48u32;
            h = 18u32;
            radius = wide_radius;
            first = offset;
            j = (i - 13056) as u32;
        }
        let x = (j % w) as i32;
        let y = (j / w) as i32;
        let mut r = 0.0f32;
        let mut g = 0.0f32;
        let mut b = 0.0f32;
        for t in 0..(2u32 * radius + 1u32) {
            let d = t as i32 - radius as i32;
            let mut sx = x;
            let mut sy = y;
            if horizontal == 1u32 {
                sx = x + d;
            } else {
                sy = y + d;
            }
            if sx >= 0 && sx < w as i32 && sy >= 0 && sy < h as i32 {
                let at = ((base + sy as u32 * w + sx as u32) * 4u32) as usize;
                let k = taps[(first + t) as usize];
                r += k * src[at];
                g += k * src[at + 1];
                b += k * src[at + 2];
            }
        }
        dst[i * 4] = r;
        dst[i * 4 + 1] = g;
        dst[i * 4 + 2] = b;
        dst[i * 4 + 3] = 0.0;
    }
}

#[cube]
fn bilinear(wide: &[f32], x0: u32, x1: u32, y0: u32, y1: u32, fx: f32, fy: f32, c: u32) -> f32 {
    let top = wide[((13056u32 + y0 * 48u32 + x0) * 4u32 + c) as usize] * (1.0 - fx)
        + wide[((13056u32 + y0 * 48u32 + x1) * 4u32 + c) as usize] * fx;
    let bottom = wide[((13056u32 + y1 * 48u32 + x0) * 4u32 + c) as usize] * (1.0 - fx)
        + wide[((13056u32 + y1 * 48u32 + x1) * 4u32 + c) as usize] * fx;
    top * (1.0 - fy) + bottom * fy
}

/// The glow cells the `crt` kernel reads: the close blur plus the (bilinear) wide one, weighted.
#[cube(launch_unchecked)]
pub fn glow_merge(blurred: &[f32], glow: &mut [f32], kc: f32, kw: f32) {
    let i = ABSOLUTE_POS;
    if i < 13056 {
        let x = (i % 192) as u32;
        let y = (i / 192) as u32;
        let gx = f32::clamp((x as f32 + 0.5) / 4.0 - 0.5, 0.0, 47.0);
        let gy = f32::clamp((y as f32 + 0.5) / 4.0 - 0.5, 0.0, 17.0);
        let x0 = f32::floor(gx) as u32;
        let y0 = f32::floor(gy) as u32;
        let x1 = u32::min(x0 + 1u32, 47u32);
        let y1 = u32::min(y0 + 1u32, 17u32);
        let fx = gx - x0 as f32;
        let fy = gy - y0 as f32;
        glow[i * 4] = kc * blurred[i * 4] + kw * bilinear(blurred, x0, x1, y0, y1, fx, fy, 0u32);
        glow[i * 4 + 1] = kc * blurred[i * 4 + 1] + kw * bilinear(blurred, x0, x1, y0, y1, fx, fy, 1u32);
        glow[i * 4 + 2] = kc * blurred[i * 4 + 2] + kw * bilinear(blurred, x0, x1, y0, y1, fx, fy, 2u32);
        glow[i * 4 + 3] = 0.0;
    }
}

/// Every buffer and launch of a frame's picture: the glow of the frame, then the `crt` kernel.
pub struct Pipeline {
    /// The cells of both grids, then the same blurred along the rows, then along the columns.
    grid: cubecl::server::Handle,
    tmp: cubecl::server::Handle,
    blurred: cubecl::server::Handle,
    taps: cubecl::server::Handle,
    glow: cubecl::server::Handle,
    params: cubecl::server::Handle,
    luts: cubecl::server::Handle,
    /// The frame as padded linear light (`to_linear`, made by `glow_cells`).
    lin: cubecl::server::Handle,
    cols: cubecl::server::Handle,
    rows: cubecl::server::Handle,
    out_size: (usize, usize),
    hf: cubecl::server::Handle,
    slot_count: usize,
    lights: cubecl::server::Handle,
    luts_len: usize,
    out: cubecl::server::Handle,
    close_radius: u32,
    wide_radius: u32,
    close_taps: usize,
    kernel_weights: (f32, f32),
    out_len: usize,
    /// The units of a cube (the CPU backend runs a cube on one thread: a smaller one spreads
    /// the picture over more of them).
    pub cube_dim: u32,
}

const FINE: usize = GLOW_W * GLOW_H;
const COARSE: usize = GLOW_CW * GLOW_CH;
// the glow kernels are `launch_unchecked` and bake these sizes in as literals (192, 68, 48, 18,
// 13056 = 192 * 68, 13920 = 13056 + 864): a change of the grids must change them together
const _: () = assert!(
    GLOW_W == 192 && GLOW_H == 68 && GLOW_CW == 48 && GLOW_CH == 18 && FINE == 13056 && COARSE == 864
);

const _: () = assert!(LIN_W == 776 && LIN_H == 280 && LIN_W * LIN_H == 217_280 && PAD == 4 && 13920 + 217_280 == 231_200);

/// The most cubes a dimension of a launch may have (wgpu's default limit).
const MAX_CUBES: u32 = 65535;

/// The cubes for a `w` x `h` picture, `dim` x 1 threads each: a row of cubes per row of pixels
/// (the kernels read their pixel from `ABSOLUTE_POS_X` / `_Y`, no division).
pub fn grid_count(w: usize, h: usize, dim: u32) -> CubeCount {
    CubeCount::Static((w as u32).div_ceil(dim), h as u32, 1)
}

/// The cubes to run `n` threads in cubes of `dim`: in one dimension, or spread over two when there
/// would be more than the GPU's limit (a picture drawn at 4x or more); the kernels' `i < len`
/// guards and the linear `ABSOLUTE_POS` make the split invisible.
pub fn cube_count(n: usize, dim: u32) -> CubeCount {
    let cubes = (n as u32).div_ceil(dim);
    if cubes <= MAX_CUBES {
        CubeCount::Static(cubes.max(1), 1, 1)
    } else {
        CubeCount::Static(MAX_CUBES, cubes.div_ceil(MAX_CUBES), 1)
    }
}

impl Pipeline {
    pub fn new(client: &cubecl::client::Client, params: &CrtParams, warp: Warp) -> Self {
        let (w, h) = params.output_size();
        let close = gauss_taps(GLOW_CLOSE.0 / 4.0);
        let wide = gauss_taps(GLOW_WIDE.0 / 16.0);
        let taps: Vec<f32> = close.iter().chain(wide.iter()).copied().collect();
        let luts = luts(params);
        let cells = |n: usize| client.create_from_slice(&vec![0u8; n * 16]);
        Self {
            grid: cells(FINE + COARSE),
            tmp: cells(FINE + COARSE),
            blurred: cells(FINE + COARSE),
            taps: client.create_from_slice(f32::as_bytes(&taps)),
            glow: cells(FINE),
            params: client.create_from_slice(f32::as_bytes(&pack(params, warp))),
            lin: client.create_from_slice(&vec![0u8; LIN_W * LIN_H * 16]),
            cols: client.create_from_slice(f32::as_bytes(&axis_tables(params).0)),
            rows: client.create_from_slice(f32::as_bytes(&axis_tables(params).1)),
            out_size: (w, h),
            hf: client.empty(HF_LINES * w.div_ceil(params.slot_px()) * 4),
            slot_count: w.div_ceil(params.slot_px()),
            luts_len: luts.len(),
            luts: client.create_from_slice(f32::as_bytes(&luts)),
            lights: client.empty(w * h * 4),
            out: client.empty(w * h * 4),
            close_radius: (close.len() / 2) as u32,
            wide_radius: (wide.len() / 2) as u32,
            close_taps: close.len(),
            kernel_weights: (GLOW_CLOSE.1 * params.glow_halo, GLOW_WIDE.1),
            out_len: w * h,
            cube_dim: 256,
        }
    }

    /// The parameters' buffer (`pack`: the glass is in it).
    pub fn params_handle(&self) -> &cubecl::server::Handle {
        &self.params
    }

    /// The light of every slot (the `slots` kernel's output), for the tests.
    #[cfg(test)]
    pub fn lights(&self) -> &cubecl::server::Handle {
        &self.lights
    }

    /// The picture's buffer.
    pub fn out(&self) -> &cubecl::server::Handle {
        &self.out
    }

    /// The glow cells of the last [`Self::run`] (four floats a cell).
    pub fn glow(&self) -> &cubecl::server::Handle {
        &self.glow
    }

    /// Work out the glow of `frame` (`sw` x `sh` pixels of RGB bytes, `sw` <= 768, `sh` <= 272) and
    /// the picture, in the buffers of `self`.
    pub fn run(&self, client: &cubecl::client::Client, frame: &cubecl::server::Handle, frame_words: usize, sw: u32, sh: u32) {
        self.run_with(client, frame, frame_words, sw, sh, |_| {});
    }

    /// [`Self::run`], calling `after(name)` after each launch (a benchmark waits for the launch
    /// there to time it).
    pub fn run_with(
        &self,
        client: &cubecl::client::Client,
        frame: &cubecl::server::Handle,
        frame_words: usize,
        sw: u32,
        sh: u32,
        mut after: impl FnMut(&'static str),
    ) {
        let arg = |h: &cubecl::server::Handle, len: usize| unsafe { BufferArg::from_raw_parts(h.clone(), len) };
        let one_d = |n: usize| (cube_count(n, 64), CubeDim::new_1d(64));
        // SAFETY: the buffers have the sizes the kernels index (see their loops' bounds)
        unsafe {
            let both = FINE + COARSE;
            let taps_len = self.close_taps + (2 * self.wide_radius + 1) as usize;
            let (count, dim) = one_d(both);
            let (cells_count, _) = one_d(both + LIN_W * LIN_H);
            glow_cells::launch_unchecked(
                client, cells_count, dim,
                arg(frame, frame_words), arg(&self.grid, both * 4), arg(&self.lin, LIN_W * LIN_H * 4), sw, sh,
            );
            after("glow_cells");
            glow_blur::launch_unchecked(
                client, count.clone(), dim,
                arg(&self.grid, both * 4), arg(&self.taps, taps_len), arg(&self.tmp, both * 4),
                1u32, self.close_radius, self.wide_radius, self.close_taps as u32,
            );
            after("glow_blur_h");
            glow_blur::launch_unchecked(
                client, count, dim,
                arg(&self.tmp, both * 4), arg(&self.taps, taps_len), arg(&self.blurred, both * 4),
                0u32, self.close_radius, self.wide_radius, self.close_taps as u32,
            );
            after("glow_blur_v");
            let (count, dim) = one_d(FINE);
            glow_merge::launch_unchecked(
                client, count, dim,
                arg(&self.blurred, both * 4), arg(&self.glow, FINE * 4),
                self.kernel_weights.0, self.kernel_weights.1,
            );
            after("glow_merge");
            let (gw, gh) = self.out_size;
            let count = || grid_count(gw, gh, self.cube_dim);
            let ns = self.slot_count;
            hfilter::launch_unchecked(
                client,
                CubeCount::Static((ns as u32).div_ceil(self.cube_dim), HF_LINES as u32, 1),
                CubeDim::new_2d(self.cube_dim, 1),
                arg(&self.lin, LIN_W * LIN_H * 4), arg(&self.luts, self.luts_len), arg(&self.params, PARAMS),
                arg(&self.hf, HF_LINES * ns),
            );
            after("hfilter");
            slots::launch_unchecked(
                client,
                count(),
                CubeDim::new_2d(self.cube_dim, 1),
                arg(&self.lin, LIN_W * LIN_H * 4), arg(&self.luts, self.luts_len), arg(&self.params, PARAMS),
                arg(&self.hf, HF_LINES * ns), arg(&self.cols, gw * COL), arg(&self.rows, gh * ROW),
                arg(&self.lights, self.out_len),
            );
            after("slots");
            crt::launch_unchecked(
                client,
                count(),
                CubeDim::new_2d(self.cube_dim, 1),
                arg(&self.lights, self.out_len), arg(&self.glow, FINE * 4), arg(&self.params, PARAMS),
                arg(&self.cols, gw * COL), arg(&self.rows, gh * ROW),
                arg(&self.out, self.out_len),
            );
            after("crt");
        }
    }
}

/// The tables the kernels look up instead of evaluating Gaussians and the slot mask per pixel:
/// `PHASES` x 4 weights of the horizontal taps (normalised) for the fractional position of the beam
/// in the source, `PHASES` x 2 weights of the two scanlines, and the distance to the slot of one
/// period of the mask (`6 * slot` pixels across, one pitch down).
pub fn luts(p: &CrtParams) -> Vec<f32> {
    let mut v = Vec::with_capacity(M_OFF + 6 * p.slot_px() * p.pitch_rows());
    let gauss = |d: f32, sigma: f32| (-d * d / (2.0 * sigma * sigma)).exp();
    // the horizontal taps: the red and green guns, then the blue (a wider spot)
    for sigma in [p.beam_sigma_h, p.beam_sigma_h * p.blue_width] {
        for phase in 0..PHASES {
            let frac = (phase as f32 + 0.5) / PHASES as f32;
            let w: Vec<f32> = (0..4).map(|t| gauss(1.0 + frac - t as f32, sigma)).collect();
            let sum: f32 = w.iter().sum();
            v.extend(w.iter().map(|w| w / sum));
        }
    }
    // the scanline: its sigma grows from `beam_thin` of the full width at a black to all of it at a
    // white; the weight of the line above the beam at `frac` is `T[level][frac]` and the line
    // below's is `T[level][1 - frac]` (the table's last phase is the first's mirror)
    for level in 0..V_LEVELS {
        let share = p.beam_thin + (1.0 - p.beam_thin) * level as f32 / (V_LEVELS - 1) as f32;
        for phase in 0..PHASES {
            let frac = (phase as f32 + 0.5) / PHASES as f32;
            v.push(gauss(frac, p.beam_sigma_v * share));
        }
    }
    for y in 0..p.pitch_rows() {
        for x in 0..6 * p.slot_px() {
            v.push(p.slot_distance(x, y));
        }
    }
    v
}

/// The per-column and per-row tables of the `crt` kernel (see [`COL`]).
pub fn axis_tables(p: &CrtParams) -> (Vec<f32>, Vec<f32>) {
    let (w, h) = p.output_size();
    let sw = p.slot_px();
    let channel = |x: usize| if p.phosphor.is_some() { 1.0 } else { ((x / sw) % 3) as f32 };
    let halation = p.halation;
    let mut cols = Vec::with_capacity(w * COL);
    for x in 0..w {
        let (xl, xr) = (x.saturating_sub(sw), (x + sw).min(w - 1));
        let u = (x as f32 + 0.5) / w as f32;
        let gx = (u * GLOW_W as f32 - 0.5).clamp(0.0, (GLOW_W - 1) as f32);
        let x0 = gx.floor();
        let dx = u * 2.0 - 1.0;
        cols.extend_from_slice(&[
            channel(xl),
            channel(x),
            channel(xr),
            if x >= sw { halation } else { 0.0 },
            1.0 - 2.0 * halation,
            if x + sw < w { halation } else { 0.0 },
            x0,
            (x0 + 1.0).min((GLOW_W - 1) as f32),
            gx - x0,
            dx,
            dx * dx,
            (x / sw) as f32,
        ]);
    }
    let ns = w.div_ceil(sw);
    let mut rows = Vec::with_capacity(h * ROW);
    for y in 0..h {
        let v = (y as f32 + 0.5) / h as f32;
        let gy = (v * GLOW_H as f32 - 0.5).clamp(0.0, (GLOW_H - 1) as f32);
        let y0 = gy.floor();
        let dy = v * 2.0 - 1.0;
        // the flat glass: the source line the beam is on (and its phase), as the offsets in `hf`
        let qy = ((y as f32 + 0.5) / h as f32) * 2.0 - 1.0;
        let f = ((qy + 1.0) / 2.0) * h as f32 / p.rows_per_line as f32 - 0.5;
        let line = f.floor();
        let phase = (((f - line) * 1024.0) as usize).min(1023);
        let r = ((line as i32 + 1).clamp(0, HF_LINES as i32 - 2)) as usize;
        rows.extend_from_slice(&[
            y0, (y0 + 1.0).min((GLOW_H - 1) as f32), gy - y0, dy, dy * dy,
            (r * ns) as f32, phase as f32, ((r + 1) * ns) as f32,
        ]);
    }
    (cols, rows)
}

pub fn pack(p: &CrtParams, warp: Warp) -> Vec<f32> {
    let (w, h) = p.output_size();
    let mut v = vec![0.0f32; PARAMS];
    v[OUT_W] = w as f32;
    v[OUT_H] = h as f32;
    v[SLOT_PX] = p.slot_px() as f32;
    v[ROWS] = p.rows_per_line as f32;
    v[PITCH] = p.pitch_rows() as f32;
    v[FILL_X] = p.slot_fill.0;
    v[FILL_Y] = p.slot_fill.1;
    v[CORNER] = p.corner_radius;
    v[STAGGER] = p.stagger;
    v[SIGMA_H] = p.beam_sigma_h;
    v[SIGMA_V] = p.beam_sigma_v;
    v[SPILL] = p.spill;
    v[HALATION] = p.halation;
    v[SCALE] = p.gain * p.white_norm();
    v[X_RATIO] = p.x_ratio();
    v[MONO] = p.phosphor.map_or(0.0, |_| 1.0);
    let t = p.phosphor.unwrap_or([1.0; 3]);
    v[TINT..TINT + 3].copy_from_slice(&t);
    v[VEIL] = p.veil_light();
    v[VIGNETTE] = p.vignette;
    v[GLOW] = p.glow_light();
    v[WARP_KX] = warp.k.0;
    v[WARP_KY] = warp.k.1;
    v[WARP_CORNER] = warp.corner;
    v[EDGED] = if warp.is_flat() { 0.0 } else { 1.0 };
    v[BLUE_SHIFT] = p.blue_shift;
    v[WHITE..WHITE + 3].copy_from_slice(&if p.phosphor.is_some() { [1.0; 3] } else { p.white });
    v
}


/// The bytes the kernels read of a frame, and its size: the RGB bytes as they are (what is larger
/// than the monitor's picture is cut), padded to whole words.
pub fn frame_bytes(src: &RgbImage) -> (Cow<'_, [u8]>, u32, u32) {
    let (w, h) = (
        (src.width() as usize).min(SRC_WIDTH),
        (src.height() as usize).min(SRC_HEIGHT),
    );
    let raw = src.as_raw();
    if w == 0 || h == 0 {
        // an empty frame: one black word, so that there is a buffer to bind
        return (Cow::Owned(vec![0u8; 4]), 0, 0);
    }
    let mut bytes = Cow::Borrowed(&raw[..w * h * 3]);
    if w != src.width() as usize {
        let row = src.width() as usize * 3;
        bytes = Cow::Owned((0..h).flat_map(|y| raw[y * row..][..w * 3].iter().copied()).collect());
    }
    if bytes.len() % 4 != 0 {
        let mut padded = bytes.into_owned();
        padded.resize(padded.len().div_ceil(4) * 4, 0);
        bytes = Cow::Owned(padded);
    }
    (bytes, w as u32, h as u32)
}
