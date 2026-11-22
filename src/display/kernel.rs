//! The CRT model on the GPU, written once as a CubeCL kernel (native Vulkan / Metal / DX12 and the
//! browser's WebGPU): one thread per output pixel, the model of `cpu.rs` evaluated per pixel (the
//! beam's Gaussian taps at the slot's centre, through the glass; the slot mask's coverage; the
//! halation of the neighbouring slots; the veil, the glow, the vignette; the glass's edge; the
//! gamma).
//!
//! The arguments are plain buffers: the monitor's frame (`0xBBGGRR` words), the glow cells of
//! [`super::crt::GlowField`] (four floats a cell), the parameters of [`pack`], and the picture
//! (`0xRRGGBB` words, row after row).
use super::crt::{CrtParams, Warp, SRC_HEIGHT, SRC_WIDTH};
use cubecl::prelude::*;

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

#[cube]
fn gauss(d: f32, sigma: f32) -> f32 {
    f32::exp(-d * d / (2.0 * sigma * sigma))
}

/// Linear light of gun `c` of the source pixel (i, j); black outside.
#[cube]
fn gun(frame: &[u32], pr: &[f32], i: i32, j: i32, c: u32) -> f32 {
    let mut out = 0.0f32;
    if i >= 0 && i < 768 && j >= 0 && j < 272 {
        let v = frame[(j * 768 + i) as usize];
        let r = (v & 255u32) as f32 / 255.0;
        let g = ((v >> 8u32) & 255u32) as f32 / 255.0;
        let b = ((v >> 16u32) & 255u32) as f32 / 255.0;
        if pr[MONO] > 0.5 {
            out = 0.299 * r * r + 0.587 * g * g + 0.114 * b * b;
        } else {
            let mut s = r;
            if c == 1u32 {
                s = g;
            } else if c == 2u32 {
                s = b;
            }
            out = s * s;
        }
    }
    out
}

#[cube]
fn slot_distance(pr: &[f32], x: i32, y: i32) -> f32 {
    let sw = pr[SLOT_PX] as i32;
    let slot_w = pr[SLOT_PX];
    let slot = x / sw;
    let triad = slot / 3;
    let mut shift = 0.0f32;
    if triad % 2 == 1 {
        shift = pr[STAGGER] * pr[PITCH];
    }
    let px = (x as f32 + 0.5) - slot as f32 * slot_w - slot_w / 2.0;
    let t = y as f32 + 0.5 - shift;
    let py = (t - pr[PITCH] * f32::floor(t / pr[PITCH])) - pr[PITCH] / 2.0;
    let hx = pr[FILL_X] * slot_w / 2.0;
    let hy = pr[FILL_Y] * pr[PITCH] / 2.0;
    let r = f32::max(pr[CORNER] * f32::min(hx, hy), 0.0);
    let qx = f32::abs(px) - (hx - r);
    let qy = f32::abs(py) - (hy - r);
    let ox = f32::max(qx, 0.0);
    let oy = f32::max(qy, 0.0);
    f32::sqrt(ox * ox + oy * oy) + f32::min(f32::max(qx, qy), 0.0) - r
}

/// The phosphor of the slot at output column `x`: a mono tube has the one (called green).
#[cube]
fn slot_channel(pr: &[f32], x: i32) -> u32 {
    let mut c = ((x / pr[SLOT_PX] as i32) % 3) as u32;
    if pr[MONO] > 0.5 {
        c = 1u32;
    }
    c
}

/// The light of the slot at output column `x`, row `y`, in its own phosphor; none outside the
/// picture.
#[cube]
fn slot_light(frame: &[u32], pr: &[f32], x: i32, y: i32) -> f32 {
    let mut out = 0.0f32;
    if x >= 0 && x < pr[OUT_W] as i32 {
        let sw = pr[SLOT_PX] as i32;
        let slot = x / sw;
        let c = slot_channel(pr, x);
        // the beam's raster bends under the fixed mask: the slot reads where its centre's warped place is
        let ow = pr[OUT_W];
        let oh = pr[OUT_H];
        let qx = ((slot as f32 + 0.5) * sw as f32 / ow) * 2.0 - 1.0;
        let qy = ((y as f32 + 0.5) / oh) * 2.0 - 1.0;
        let wx = (qx * (1.0 + pr[WARP_KX] * qy * qy) + 1.0) / 2.0;
        let wy = (qy * (1.0 + pr[WARP_KY] * qx * qx) + 1.0) / 2.0;
        let u = wx * ow / pr[X_RATIO];
        let first = f32::floor(u - 0.5) as i32 - 1;
        let w0 = gauss(u - (first as f32 + 0.5), pr[SIGMA_H]);
        let w1 = gauss(u - ((first + 1) as f32 + 0.5), pr[SIGMA_H]);
        let w2 = gauss(u - ((first + 2) as f32 + 0.5), pr[SIGMA_H]);
        let w3 = gauss(u - ((first + 3) as f32 + 0.5), pr[SIGMA_H]);
        let sum = w0 + w1 + w2 + w3;
        let f = wy * oh / pr[ROWS] - 0.5;
        let line = f32::floor(f) as i32;
        let a = gauss(f - line as f32, pr[SIGMA_V]);
        let b = gauss(line as f32 + 1.0 - f, pr[SIGMA_V]);
        let la = w0 * gun(frame, pr, first, line, c)
            + w1 * gun(frame, pr, first + 1, line, c)
            + w2 * gun(frame, pr, first + 2, line, c)
            + w3 * gun(frame, pr, first + 3, line, c);
        let lb = w0 * gun(frame, pr, first, line + 1, c)
            + w1 * gun(frame, pr, first + 1, line + 1, c)
            + w2 * gun(frame, pr, first + 2, line + 1, c)
            + w3 * gun(frame, pr, first + 3, line + 1, c);
        let level = (a * la + b * lb) / sum;
        let cov = f32::clamp(0.5 - (slot_distance(pr, x, y) - pr[SPILL] * level), 0.0, 1.0);
        out = level * cov * pr[SCALE];
    }
    out
}

#[cube]
fn glow_cell(glow: &[f32], x: i32, y: i32, c: u32) -> f32 {
    glow[((y * 192 + x) * 4) as usize + c as usize]
}

#[cube]
fn glow_at(glow: &[f32], u: f32, v: f32, c: u32) -> f32 {
    let gx = f32::clamp(u * 192.0 - 0.5, 0.0, 191.0);
    let gy = f32::clamp(v * 68.0 - 0.5, 0.0, 67.0);
    let x0 = f32::floor(gx) as i32;
    let y0 = f32::floor(gy) as i32;
    let x1 = i32::min(x0 + 1, 191);
    let y1 = i32::min(y0 + 1, 67);
    let fx = gx - x0 as f32;
    let fy = gy - y0 as f32;
    let top = glow_cell(glow, x0, y0, c) * (1.0 - fx) + glow_cell(glow, x1, y0, c) * fx;
    let bottom = glow_cell(glow, x0, y1, c) * (1.0 - fx) + glow_cell(glow, x1, y1, c) * fx;
    top * (1.0 - fy) + bottom * fy
}

/// `l` if the slot's phosphor is `want`.
#[cube]
fn only(c: u32, want: u32, l: f32) -> f32 {
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
pub fn crt(frame: &[u32], glow: &[f32], pr: &[f32], out: &mut [u32]) {
    let i = ABSOLUTE_POS;
    if i < out.len() {
        let w = pr[OUT_W] as usize;
        let x = (i % w) as i32;
        let y = (i / w) as i32;
        // the slot of the pixel and its two neighbours (halation), each worked out once
        let sw = pr[SLOT_PX] as i32;
        let h = pr[HALATION];
        let (xl, xr) = (x - sw, x + sw);
        let (ll, lc, lr) = (
            h * slot_light(frame, pr, xl, y),
            (1.0 - 2.0 * h) * slot_light(frame, pr, x, y),
            h * slot_light(frame, pr, xr, y),
        );
        let (cl, cc, cr) = (slot_channel(pr, xl), slot_channel(pr, x), slot_channel(pr, xr));
        let mut r = only(cl, 0u32, ll) + only(cc, 0u32, lc) + only(cr, 0u32, lr);
        let mut g = only(cl, 1u32, ll) + only(cc, 1u32, lc) + only(cr, 1u32, lr);
        let mut b = only(cl, 2u32, ll) + only(cc, 2u32, lc) + only(cr, 2u32, lr);
        // the veil and the glow of the picture around (a mono tube: of its luminance, in its colour)
        let u = (x as f32 + 0.5) / pr[OUT_W];
        let v = (y as f32 + 0.5) / pr[OUT_H];
        let mut ar = pr[VEIL] + pr[GLOW] * glow_at(glow, u, v, 0u32);
        let mut ag = pr[VEIL] + pr[GLOW] * glow_at(glow, u, v, 1u32);
        let mut ab = pr[VEIL] + pr[GLOW] * glow_at(glow, u, v, 2u32);
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
        let dx = u * 2.0 - 1.0;
        let dy = v * 2.0 - 1.0;
        let fall = f32::max(1.0 - pr[VIGNETTE] * (dx * dx + dy * dy), 0.0);
        // the glass's rounded edge, smoothed over about a pixel
        let wx = dx * (1.0 + pr[WARP_KX] * dy * dy);
        let wy = dy * (1.0 + pr[WARP_KY] * dx * dx);
        let ex = f32::abs(wx) - (1.0 - pr[WARP_CORNER]);
        let ey = f32::abs(wy) - (1.0 - pr[WARP_CORNER]);
        let ox = f32::max(ex, 0.0);
        let oy = f32::max(ey, 0.0);
        let distance = f32::sqrt(ox * ox + oy * oy) + f32::min(f32::max(ex, ey), 0.0) - pr[WARP_CORNER];
        let pixel = 2.0 / f32::max(pr[OUT_W], pr[OUT_H]);
        let edge = f32::clamp(0.5 - distance / pixel, 0.0, 1.0);
        let alpha = (1.0 - pr[EDGED]) + pr[EDGED] * edge;
        out[i] = (encode(r, ar, fall, alpha) << 16u32)
            | (encode(g, ag, fall, alpha) << 8u32)
            | encode(b, ab, fall, alpha);
    }
}

pub fn pack(p: &CrtParams, warp: Warp) -> Vec<f32> {
    let (w, h) = p.output_size();
    let mut v = vec![0.0f32; 26];
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
    v
}

