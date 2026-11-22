//! The CRT model on the GPU, written once as a CubeCL kernel (native Vulkan / Metal / DX12 and the
//! browser's WebGPU): one thread per output pixel, the model of `cpu.rs` evaluated per pixel (the
//! beam's Gaussian taps at the slot's centre, through the glass; the slot mask's coverage; the
//! halation of the neighbouring slots; the veil, the glow, the vignette; the glass's edge; the
//! gamma).
//!
//! The arguments are plain buffers: the monitor's frame (its RGB bytes as they are, `src_w` x `src_h`
//! pixels), the glow cells (four floats a cell; worked out on the GPU too, by [`Pipeline`], as
//! [`super::crt::GlowField`] does for the CPU renderer), the parameters of [`pack`], and the
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

// the tables of `luts` (see `Pipeline::new`): the Gaussian weights of 1024 phases, and the slot mask
pub const PHASES: usize = 1024;
pub const V_OFF: usize = PHASES * 4;
pub const M_OFF: usize = V_OFF + PHASES * 2;

#[cube]
fn gauss(d: f32, sigma: f32) -> f32 {
    f32::exp(-d * d / (2.0 * sigma * sigma))
}

/// Byte `at` of the frame (its RGB bytes as they are, four to a word).
#[cube]
fn byte(frame: &[u32], at: u32) -> f32 {
    ((frame[(at >> 2u32) as usize] >> ((at & 3u32) * 8u32)) & 255u32) as f32
}

/// Linear light of gun `c` of the source pixel (i, j); black outside the `sw` x `sh` frame.
#[cube]
fn gun(frame: &[u32], pr: &[f32], i: i32, j: i32, c: u32, sw: u32, sh: u32) -> f32 {
    let mut out = 0.0f32;
    if i >= 0 && i < sw as i32 && j >= 0 && j < sh as i32 {
        let at = (j as u32 * sw + i as u32) * 3u32;
        if pr[MONO] > 0.5 {
            let r = byte(frame, at) / 255.0;
            let g = byte(frame, at + 1u32) / 255.0;
            let b = byte(frame, at + 2u32) / 255.0;
            out = 0.299 * r * r + 0.587 * g * g + 0.114 * b * b;
        } else {
            let s = byte(frame, at + c) / 255.0;
            out = s * s;
        }
    }
    out
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
/// picture. The Gaussian weights and the slot mask come from the tables `luts` (a phase of 1/1024
/// of a pixel: below the rounding of the picture).
#[cube]
fn slot_light(frame: &[u32], luts: &[f32], pr: &[f32], x: i32, y: i32, sw_: u32, sh_: u32) -> f32 {
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
        let v = wx * ow / pr[X_RATIO] - 0.5;
        let fl = f32::floor(v);
        let first = fl as i32 - 1;
        let hp = u32::min(((v - fl) * 1024.0) as u32, 1023u32) as usize * 4;
        let (w0, w1, w2, w3) = (luts[hp], luts[hp + 1], luts[hp + 2], luts[hp + 3]);
        let f = wy * oh / pr[ROWS] - 0.5;
        let line = f32::floor(f) as i32;
        let vp = V_OFF + u32::min(((f - line as f32) * 1024.0) as u32, 1023u32) as usize * 2;
        let (a, b) = (luts[vp], luts[vp + 1]);
        let la = w0 * gun(frame, pr, first, line, c, sw_, sh_)
            + w1 * gun(frame, pr, first + 1, line, c, sw_, sh_)
            + w2 * gun(frame, pr, first + 2, line, c, sw_, sh_)
            + w3 * gun(frame, pr, first + 3, line, c, sw_, sh_);
        let lb = w0 * gun(frame, pr, first, line + 1, c, sw_, sh_)
            + w1 * gun(frame, pr, first + 1, line + 1, c, sw_, sh_)
            + w2 * gun(frame, pr, first + 2, line + 1, c, sw_, sh_)
            + w3 * gun(frame, pr, first + 3, line + 1, c, sw_, sh_);
        let level = a * la + b * lb;
        let period = (6 * sw) as usize;
        let distance = luts[M_OFF + (y as usize % pr[PITCH] as usize) * period + x as usize % period];
        let cov = f32::clamp(0.5 - (distance - pr[SPILL] * level), 0.0, 1.0);
        out = level * cov * pr[SCALE];
    }
    out
}

/// The light of the slot of every pixel (its own phosphor), once: the halation of the neighbours is
/// then a sum of three of them.
#[cube(launch_unchecked)]
pub fn slots(frame: &[u32], luts: &[f32], pr: &[f32], lights: &mut [f32], src_w: u32, src_h: u32) {
    let i = ABSOLUTE_POS;
    if i < lights.len() {
        let w = pr[OUT_W] as usize;
        lights[i] = slot_light(frame, luts, pr, (i % w) as i32, (i / w) as i32, src_w, src_h);
    }
}

/// The light of the slot at column `x` of row `y`, from [`slots`]; none outside.
#[cube]
fn lit(lights: &[f32], pr: &[f32], x: i32, y: i32) -> f32 {
    let mut out = 0.0f32;
    if x >= 0 && x < pr[OUT_W] as i32 {
        out = lights[(y as u32 * pr[OUT_W] as u32 + x as u32) as usize];
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
pub fn crt(lights: &[f32], glow: &[f32], pr: &[f32], out: &mut [u32]) {
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
            h * lit(lights, pr, xl, y),
            (1.0 - 2.0 * h) * lit(lights, pr, x, y),
            h * lit(lights, pr, xr, y),
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

// ---- the glow of the frame, on the GPU (`GlowField::new` of crt.rs, step for step) ----

/// The mean light of the 4 x 4 source pixels of each fine cell (`GLOW_W` x `GLOW_H` cells).
#[cube(launch_unchecked)]
pub fn glow_cells(frame: &[u32], fine: &mut [f32], sw: u32, sh: u32) {
    let i = ABSOLUTE_POS;
    if i < 13056 {
        let cx = (i % 192) as u32;
        let cy = (i / 192) as u32;
        let mut r = 0.0f32;
        let mut g = 0.0f32;
        let mut b = 0.0f32;
        for dy in 0..4u32 {
            for dx in 0..4u32 {
                let x = cx * 4u32 + dx;
                let y = cy * 4u32 + dy;
                if x < sw && y < sh {
                    let at = (y * sw + x) * 3u32;
                    let lr = byte(frame, at) / 255.0;
                    let lg = byte(frame, at + 1u32) / 255.0;
                    let lb = byte(frame, at + 2u32) / 255.0;
                    r += lr * lr / 16.0;
                    g += lg * lg / 16.0;
                    b += lb * lb / 16.0;
                }
            }
        }
        fine[i * 4] = r;
        fine[i * 4 + 1] = g;
        fine[i * 4 + 2] = b;
        fine[i * 4 + 3] = 0.0;
    }
}

/// The coarse grid of the wide blur: the mean of 4 x 4 fine cells.
#[cube(launch_unchecked)]
pub fn glow_coarse(fine: &[f32], coarse: &mut [f32]) {
    let i = ABSOLUTE_POS;
    if i < 864 {
        let gx = (i % 48) as u32;
        let gy = (i / 48) as u32;
        let mut r = 0.0f32;
        let mut g = 0.0f32;
        let mut b = 0.0f32;
        for dy in 0..4u32 {
            let fy = gy * 4u32 + dy;
            if fy < 68u32 {
                for dx in 0..4u32 {
                    let at = ((fy * 192u32 + gx * 4u32 + dx) * 4u32) as usize;
                    r += fine[at] / 16.0;
                    g += fine[at + 1] / 16.0;
                    b += fine[at + 2] / 16.0;
                }
            }
        }
        coarse[i * 4] = r;
        coarse[i * 4 + 1] = g;
        coarse[i * 4 + 2] = b;
        coarse[i * 4 + 3] = 0.0;
    }
}

/// One pass of a separable Gaussian blur (zero outside the grid) of a `w` x `h` grid, along the
/// rows (`horizontal` 1) or the columns; its `2 * radius + 1` taps are `taps[offset..]`.
#[cube(launch_unchecked)]
pub fn glow_blur(
    src: &[f32],
    taps: &[f32],
    dst: &mut [f32],
    w: u32,
    h: u32,
    horizontal: u32,
    radius: u32,
    offset: u32,
) {
    let i = ABSOLUTE_POS;
    if i < (w * h) as usize {
        let x = (i as u32 % w) as i32;
        let y = (i as u32 / w) as i32;
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
                let at = ((sy as u32 * w + sx as u32) * 4u32) as usize;
                let k = taps[(offset + t) as usize];
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
    let top = wide[((y0 * 48u32 + x0) * 4u32 + c) as usize] * (1.0 - fx)
        + wide[((y0 * 48u32 + x1) * 4u32 + c) as usize] * fx;
    let bottom = wide[((y1 * 48u32 + x0) * 4u32 + c) as usize] * (1.0 - fx)
        + wide[((y1 * 48u32 + x1) * 4u32 + c) as usize] * fx;
    top * (1.0 - fy) + bottom * fy
}

/// The glow cells the `crt` kernel reads: the close blur plus the (bilinear) wide one, weighted.
#[cube(launch_unchecked)]
pub fn glow_merge(close: &[f32], wide: &[f32], glow: &mut [f32], kc: f32, kw: f32) {
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
        glow[i * 4] = kc * close[i * 4] + kw * bilinear(wide, x0, x1, y0, y1, fx, fy, 0u32);
        glow[i * 4 + 1] = kc * close[i * 4 + 1] + kw * bilinear(wide, x0, x1, y0, y1, fx, fy, 1u32);
        glow[i * 4 + 2] = kc * close[i * 4 + 2] + kw * bilinear(wide, x0, x1, y0, y1, fx, fy, 2u32);
        glow[i * 4 + 3] = 0.0;
    }
}

/// Every buffer and launch of a frame's picture: the glow of the frame, then the `crt` kernel.
pub struct Pipeline {
    fine: cubecl::server::Handle,
    coarse: cubecl::server::Handle,
    tmp_fine: cubecl::server::Handle,
    close: cubecl::server::Handle,
    tmp_coarse: cubecl::server::Handle,
    wide: cubecl::server::Handle,
    taps: cubecl::server::Handle,
    glow: cubecl::server::Handle,
    params: cubecl::server::Handle,
    luts: cubecl::server::Handle,
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

impl Pipeline {
    pub fn new(client: &cubecl::client::Client, params: &CrtParams, warp: Warp) -> Self {
        let (w, h) = params.output_size();
        let close = gauss_taps(GLOW_CLOSE.0 / 4.0);
        let wide = gauss_taps(GLOW_WIDE.0 / 16.0);
        let taps: Vec<f32> = close.iter().chain(wide.iter()).copied().collect();
        let luts = luts(params);
        let cells = |n: usize| client.create_from_slice(&vec![0u8; n * 16]);
        Self {
            fine: cells(FINE),
            coarse: cells(COARSE),
            tmp_fine: cells(FINE),
            close: cells(FINE),
            tmp_coarse: cells(COARSE),
            wide: cells(COARSE),
            taps: client.create_from_slice(f32::as_bytes(&taps)),
            glow: cells(FINE),
            params: client.create_from_slice(f32::as_bytes(&pack(params, warp))),
            luts_len: luts.len(),
            luts: client.create_from_slice(f32::as_bytes(&luts)),
            lights: client.empty(w * h * 4),
            out: client.empty(w * h * 4),
            close_radius: (close.len() / 2) as u32,
            wide_radius: (wide.len() / 2) as u32,
            close_taps: close.len(),
            kernel_weights: (GLOW_CLOSE.1, GLOW_WIDE.1),
            out_len: w * h,
            cube_dim: 256,
        }
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
        let arg = |h: &cubecl::server::Handle, len: usize| unsafe { BufferArg::from_raw_parts(h.clone(), len) };
        let one_d = |n: usize| (CubeCount::Static((n as u32).div_ceil(64), 1, 1), CubeDim::new_1d(64));
        // SAFETY: the buffers have the sizes the kernels index (see their loops' bounds)
        unsafe {
            let (count, dim) = one_d(FINE);
            glow_cells::launch_unchecked(client, count, dim, arg(frame, frame_words), arg(&self.fine, FINE * 4), sw, sh);
            let (count, dim) = one_d(COARSE);
            glow_coarse::launch_unchecked(client, count, dim, arg(&self.fine, FINE * 4), arg(&self.coarse, COARSE * 4));
            let taps_len = self.close_taps + (2 * self.wide_radius + 1) as usize;
            let (count, dim) = one_d(FINE);
            glow_blur::launch_unchecked(
                client, count.clone(), dim,
                arg(&self.fine, FINE * 4), arg(&self.taps, taps_len), arg(&self.tmp_fine, FINE * 4),
                GLOW_W as u32, GLOW_H as u32, 1u32, self.close_radius, 0u32,
            );
            glow_blur::launch_unchecked(
                client, count, dim,
                arg(&self.tmp_fine, FINE * 4), arg(&self.taps, taps_len), arg(&self.close, FINE * 4),
                GLOW_W as u32, GLOW_H as u32, 0u32, self.close_radius, 0u32,
            );
            let (count, dim) = one_d(COARSE);
            glow_blur::launch_unchecked(
                client, count.clone(), dim,
                arg(&self.coarse, COARSE * 4), arg(&self.taps, taps_len), arg(&self.tmp_coarse, COARSE * 4),
                GLOW_CW as u32, GLOW_CH as u32, 1u32, self.wide_radius, self.close_taps as u32,
            );
            glow_blur::launch_unchecked(
                client, count, dim,
                arg(&self.tmp_coarse, COARSE * 4), arg(&self.taps, taps_len), arg(&self.wide, COARSE * 4),
                GLOW_CW as u32, GLOW_CH as u32, 0u32, self.wide_radius, self.close_taps as u32,
            );
            let (count, dim) = one_d(FINE);
            glow_merge::launch_unchecked(
                client, count, dim,
                arg(&self.close, FINE * 4), arg(&self.wide, COARSE * 4), arg(&self.glow, FINE * 4),
                self.kernel_weights.0, self.kernel_weights.1,
            );
            let count = || CubeCount::Static((self.out_len as u32).div_ceil(self.cube_dim), 1, 1);
            slots::launch_unchecked(
                client,
                count(),
                CubeDim::new_1d(self.cube_dim),
                arg(frame, frame_words), arg(&self.luts, self.luts_len), arg(&self.params, 26),
                arg(&self.lights, self.out_len),
                sw, sh,
            );
            crt::launch_unchecked(
                client,
                count(),
                CubeDim::new_1d(self.cube_dim),
                arg(&self.lights, self.out_len), arg(&self.glow, FINE * 4), arg(&self.params, 26),
                arg(&self.out, self.out_len),
            );
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
    for phase in 0..PHASES {
        let frac = (phase as f32 + 0.5) / PHASES as f32;
        let w: Vec<f32> = (0..4).map(|t| gauss(1.0 + frac - t as f32, p.beam_sigma_h)).collect();
        let sum: f32 = w.iter().sum();
        v.extend(w.iter().map(|w| w / sum));
    }
    for phase in 0..PHASES {
        let frac = (phase as f32 + 0.5) / PHASES as f32;
        v.push(gauss(frac, p.beam_sigma_v));
        v.push(gauss(1.0 - frac, p.beam_sigma_v));
    }
    for y in 0..p.pitch_rows() {
        for x in 0..6 * p.slot_px() {
            v.push(p.slot_distance(x, y));
        }
    }
    v
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


/// The bytes the kernels read of a frame, and its size: the RGB bytes as they are (what is larger
/// than the monitor's picture is cut), padded to whole words.
pub fn frame_bytes(src: &RgbImage) -> (Cow<'_, [u8]>, u32, u32) {
    let (w, h) = (
        (src.width() as usize).min(SRC_WIDTH),
        (src.height() as usize).min(SRC_HEIGHT),
    );
    let raw = src.as_raw();
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
