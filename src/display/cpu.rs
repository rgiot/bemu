//! CPU renderer of the CRT model of [`super::crt`].
//!
//! Like ACE-DL's renderer (whose approach it is inspired by, see
//! `crt.rs`) it works scanline by scanline and is split in bands of rows run
//! in parallel, but with two plain passes instead of a pipeline of
//! data-dependent ones:
//!
//! 1. *horizontal*, once per monitor scanline: the beam's intensity at each
//!    output column, for the gun of the slot at that column (a few taps of
//!    that one channel, summed in linear light);
//! 2. *vertical + mask*, once per output row: the two nearest scanlines'
//!    beams are mixed, multiplied by the slot's coverage (which depends on the
//!    intensity: bright slots spill), and the horizontal halation and
//!    gamma encoding give the pixel.
//!
//! The inner loops are branch-free over contiguous `f32` slices, so the
//! compiler vectorises them (SSE2 on any x86_64).

use super::crt::{coverage, CrtParams, SRC_HEIGHT, SRC_WIDTH};
use super::DisplayFilter;
use image::RgbImage;

/// Taps of the horizontal beam filter.
const TAPS: usize = 4;
/// Zeros on each side of a linear scanline, for the taps near the edges.
const PAD: usize = 2;
const ENCODE_STEPS: usize = 16384;

struct Column {
    /// Index (before padding) of the first tap, in monitor pixels.
    first: i32,
    weights: [f32; TAPS],
    channel: usize
}

struct RowMix {
    /// The scanline of weight `a`; `b` is the next one. May be out of range.
    line: i32,
    a: f32,
    b: f32
}

struct Tables {
    columns: Vec<Column>,
    rows: Vec<RowMix>,
    /// `pitch_rows` rows of the mask's signed distance, `out_w` each.
    mask: Vec<Vec<f32>>,
    /// `[c][x]`: 1 if output column `x` is lit by gun `c`, else 0 (with
    /// `slot` zeros of padding on each side).
    channel_masks: [Vec<f32>; 3],
    /// `gain` and the calibration of the mask's losses.
    scale: f32,
    linear: [f32; 256],
    encode: Vec<u8>
}

impl Tables {
    fn new(p: &CrtParams) -> Self {
        let (out_w, out_h) = p.output_size();
        let ratio = p.x_ratio();
        let slot = p.slot_px();
        let columns = (0..out_w)
            .map(|x| {
                // the beam is read at the slot's centre
                let centre = ((x / slot) as f32 + 0.5) * slot as f32;
                let u = centre / ratio;
                let first = (u - 0.5).floor() as i32 - 1;
                let mut weights = [0.0f32; TAPS];
                for (t, w) in weights.iter_mut().enumerate() {
                    let d = u - ((first + t as i32) as f32 + 0.5);
                    *w = (-d * d / (2.0 * p.beam_sigma_h * p.beam_sigma_h)).exp();
                }
                let sum: f32 = weights.iter().sum();
                weights.iter_mut().for_each(|w| *w /= sum);
                Column { first, weights, channel: p.channel(x) }
            })
            .collect();

        let rows = (0..out_h)
            .map(|y| {
                let f = (y as f32 + 0.5) / p.rows_per_line as f32 - 0.5;
                let line = f.floor() as i32;
                let gauss = |d: f32| (-d * d / (2.0 * p.beam_sigma_v * p.beam_sigma_v)).exp();
                RowMix { line, a: gauss(f - line as f32), b: gauss(line as f32 + 1.0 - f) }
            })
            .collect();

        let mask = (0..p.pitch_rows())
            .map(|y| (0..out_w).map(|x| p.slot_distance(x, y)).collect())
            .collect();

        let mut channel_masks = [vec![0.0; out_w + 2 * slot], vec![0.0; out_w + 2 * slot], vec![0.0; out_w + 2 * slot]];
        for x in 0..out_w {
            channel_masks[p.channel(x)][x + slot] = 1.0;
        }

        let mut linear = [0.0f32; 256];
        for (level, l) in linear.iter_mut().enumerate() {
            *l = (level as f32 / 255.0).powf(p.gamma);
        }
        let encode = (0..ENCODE_STEPS)
            .map(|i| (255.0 * (i as f32 / (ENCODE_STEPS - 1) as f32).powf(1.0 / p.gamma)).round() as u8)
            .collect();

        Self { columns, rows, mask, channel_masks, scale: p.gain * p.white_norm(), linear, encode }
    }
}

/// The CRT model, rendered on the CPU.
pub struct CpuCrt {
    params: CrtParams,
    tables: Tables,
    /// The beam of every monitor scanline at every output column.
    lines: Vec<f32>,
    zeros: Vec<f32>,
    threads: usize
}

impl CpuCrt {
    pub fn new(params: CrtParams) -> Self {
        let (out_w, _) = params.output_size();
        let tables = Tables::new(&params);
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(16);
        Self { params, tables, lines: vec![0.0; SRC_HEIGHT * out_w], zeros: vec![0.0; out_w], threads }
    }

    /// Render on the calling thread only (for tests and benchmarks).
    pub fn single_threaded(mut self) -> Self {
        self.threads = 1;
        self
    }

    pub fn params(&self) -> &CrtParams {
        &self.params
    }

    /// Pass 1 for the scanlines `first..first + lines.len() / out_w`.
    fn horizontal(params: &CrtParams, tables: &Tables, src: &RgbImage, first: usize, lines: &mut [f32]) {
        let out_w = params.output_size().0;
        let mut planes = [[0.0f32; SRC_WIDTH + 2 * PAD]; 3];
        for (n, out) in lines.chunks_exact_mut(out_w).enumerate() {
            let j = first + n;
            for plane in &mut planes {
                plane.fill(0.0);
            }
            if j < src.height() as usize {
                let width = (src.width() as usize).min(SRC_WIDTH);
                let row = &src.as_raw()[j * src.width() as usize * 3..][..width * 3];
                for (x, pixel) in row.chunks_exact(3).enumerate() {
                    for c in 0..3 {
                        planes[c][x + PAD] = tables.linear[pixel[c] as usize];
                    }
                }
            }
            for (x, value) in out.iter_mut().enumerate() {
                let column = &tables.columns[x];
                let plane = &planes[column.channel];
                let base = (column.first + PAD as i32) as usize;
                *value = (0..TAPS).map(|t| column.weights[t] * plane[base + t]).sum();
            }
        }
    }

    /// Pass 2 for the output rows `first..first + dst.len() / out_w`.
    fn vertical(
        params: &CrtParams,
        tables: &Tables,
        lines: &[f32],
        zeros: &[f32],
        first: usize,
        dst: &mut [u32]
    ) {
        let out_w = params.output_size().0;
        let slot = params.slot_px();
        let pitch = params.pitch_rows();
        let line_of = |j: i32| -> &[f32] {
            if (0..SRC_HEIGHT as i32).contains(&j) { &lines[j as usize * out_w..][..out_w] } else { zeros }
        };
        // light of each gun, with `slot` zeros on each side for the halation
        let mut lit = [vec![0.0f32; out_w + 2 * slot], vec![0.0f32; out_w + 2 * slot], vec![0.0f32; out_w + 2 * slot]];
        let (h, centre) = (params.halation, 1.0 - 2.0 * params.halation);
        let max_index = (ENCODE_STEPS - 1) as f32;

        for (n, out) in dst.chunks_exact_mut(out_w).enumerate() {
            let y = first + n;
            let mix = &tables.rows[y];
            let (la, lb) = (line_of(mix.line), line_of(mix.line + 1));
            let distance = &tables.mask[y % pitch];
            let [r, g, b] = &mut lit;
            let masks = &tables.channel_masks;
            for x in 0..out_w {
                let level = mix.a * la[x] + mix.b * lb[x];
                let light = level * coverage(distance[x], params.spill, level) * tables.scale;
                r[x + slot] = light * masks[0][x + slot];
                g[x + slot] = light * masks[1][x + slot];
                b[x + slot] = light * masks[2][x + slot];
            }
            let encode = |light: f32| tables.encode[(light.clamp(0.0, 1.0) * max_index + 0.5) as usize] as u32;
            for x in 0..out_w {
                let at = x + slot;
                let red = centre * r[at] + h * (r[at - slot] + r[at + slot]);
                let green = centre * g[at] + h * (g[at - slot] + g[at + slot]);
                let blue = centre * b[at] + h * (b[at - slot] + b[at + slot]);
                out[x] = encode(red) << 16 | encode(green) << 8 | encode(blue);
            }
        }
    }
}

impl DisplayFilter for CpuCrt {
    fn output_size(&self) -> (usize, usize) {
        self.params.output_size()
    }

    fn render(&mut self, src: &RgbImage, dst: &mut [u32]) {
        let (out_w, out_h) = self.params.output_size();
        assert_eq!(dst.len(), out_w * out_h);
        let Self { params, tables, lines, zeros, threads } = self;
        let (params, tables, threads) = (&*params, &*tables, *threads);

        let band = |total: usize| total.div_ceil(threads).max(1);
        std::thread::scope(|scope| {
            for (i, chunk) in lines.chunks_mut(band(SRC_HEIGHT) * out_w).enumerate() {
                let first = i * band(SRC_HEIGHT);
                if threads == 1 {
                    Self::horizontal(params, tables, src, first, chunk);
                }
                else {
                    scope.spawn(move || Self::horizontal(params, tables, src, first, chunk));
                }
            }
        });
        let (lines, zeros): (&[f32], &[f32]) = (lines, zeros);
        std::thread::scope(|scope| {
            for (i, chunk) in dst.chunks_mut(band(out_h) * out_w).enumerate() {
                let first = i * band(out_h);
                if threads == 1 {
                    Self::vertical(params, tables, lines, zeros, first, chunk);
                }
                else {
                    scope.spawn(move || Self::vertical(params, tables, lines, zeros, first, chunk));
                }
            }
        });
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::display::crt::CrtProfile;
    use image::Rgb;

    fn field(color: [u8; 3]) -> RgbImage {
        RgbImage::from_pixel(SRC_WIDTH as u32, SRC_HEIGHT as u32, Rgb(color))
    }

    fn render(profile: CrtProfile, src: &RgbImage) -> (Vec<u32>, usize, usize) {
        let mut filter = CpuCrt::new(profile.params());
        let (w, h) = filter.output_size();
        let mut dst = vec![0; w * h];
        filter.render(src, &mut dst);
        (dst, w, h)
    }

    /// Straightforward per-pixel implementation of the model, without any
    /// table: what the optimised passes must match.
    fn reference(p: &CrtParams, src: &RgbImage, x: usize, y: usize) -> u32 {
        let lin = |v: u8| (v as f32 / 255.0).powf(p.gamma);
        let level_at = |x: usize, y: usize| -> f32 {
            let slot = p.slot_px();
            let u = ((x / slot) as f32 + 0.5) * slot as f32 / p.x_ratio();
            let v = (y as f32 + 0.5) / p.rows_per_line as f32;
            let channel = p.channel(x);
            let mut horizontal = vec![];
            for j in 0..SRC_HEIGHT {
                let mut sum = 0.0;
                let mut weights = 0.0;
                // beyond the picture, the beam reads black
                for i in -3..SRC_WIDTH as i32 + 3 {
                    let d = u - (i as f32 + 0.5);
                    if d.abs() > 2.0 {
                        continue;
                    }
                    let w = (-d * d / (2.0 * p.beam_sigma_h * p.beam_sigma_h)).exp();
                    weights += w;
                    if (0..SRC_WIDTH as i32).contains(&i) {
                        sum += w * lin(src.get_pixel(i as u32, j as u32)[channel]);
                    }
                }
                horizontal.push(sum / weights);
            }
            let f = v - 0.5;
            let j = f.floor() as i32;
            let gauss = |d: f32| (-d * d / (2.0 * p.beam_sigma_v * p.beam_sigma_v)).exp();
            let at = |j: i32| if (0..SRC_HEIGHT as i32).contains(&j) { horizontal[j as usize] } else { 0.0 };
            gauss(f - j as f32) * at(j) + gauss(j as f32 + 1.0 - f) * at(j + 1)
        };
        let light = |x: isize, channel: usize| -> f32 {
            let (out_w, _) = p.output_size();
            if x < 0 || x as usize >= out_w || p.channel(x as usize) != channel {
                return 0.0;
            }
            let level = level_at(x as usize, y);
            level * coverage(p.slot_distance(x as usize, y), p.spill, level) * p.gain * p.white_norm()
        };
        let slot = p.slot_px() as isize;
        let encode = |l: f32| (255.0 * l.clamp(0.0, 1.0).powf(1.0 / p.gamma)).round() as u32;
        let channel = |c: usize| {
            let x = x as isize;
            encode((1.0 - 2.0 * p.halation) * light(x, c) + p.halation * (light(x - slot, c) + light(x + slot, c)))
        };
        channel(0) << 16 | channel(1) << 8 | channel(2)
    }

    #[test]
    fn matches_the_reference_model() {
        let mut src = RgbImage::new(SRC_WIDTH as u32, SRC_HEIGHT as u32);
        for (x, y, pixel) in src.enumerate_pixels_mut() {
            *pixel = Rgb([(x * 7 % 256) as u8, (y * 5 % 256) as u8, ((x / 3 + y) * 11 % 256) as u8]);
        }
        let profile = CrtProfile::Ctm644;
        let (dst, w, _) = render(profile, &src);
        let p = profile.params();
        for &(x, y) in &[(0usize, 0usize), (1, 1), (700, 400), (1439, 1087), (333, 2), (900, 903), (12, 640)] {
            let (a, b) = (dst[y * w + x], reference(&p, &src, x, y));
            for shift in [0, 8, 16] {
                let d = ((a >> shift) & 255) as i32 - ((b >> shift) & 255) as i32;
                assert!(d.abs() <= 1, "({x},{y}) {a:06x} vs {b:06x}");
            }
        }
    }

    #[test]
    fn threads_do_not_change_the_picture() {
        let mut src = RgbImage::new(SRC_WIDTH as u32, SRC_HEIGHT as u32);
        for (x, y, pixel) in src.enumerate_pixels_mut() {
            *pixel = Rgb([(x * 3) as u8, (y * 9) as u8, (x ^ y) as u8]);
        }
        let mut one = CpuCrt::new(CrtProfile::Ctm644.params()).single_threaded();
        let mut many = CpuCrt::new(CrtProfile::Ctm644.params());
        many.threads = 5;
        let (w, h) = one.output_size();
        let (mut a, mut b) = (vec![0; w * h], vec![0; w * h]);
        one.render(&src, &mut a);
        many.render(&src, &mut b);
        assert_eq!(a, b);
    }

    #[test]
    fn black_is_black() {
        let (dst, ..) = render(CrtProfile::Ctm644, &field([0, 0, 0]));
        assert!(dst.iter().all(|&p| p == 0));
    }

    #[test]
    fn a_red_picture_lights_no_green_nor_blue() {
        let (dst, ..) = render(CrtProfile::Ctm644, &field([255, 0, 0]));
        assert!(dst.iter().all(|&p| p & 0xFFFF == 0));
        assert!(dst.iter().any(|&p| p >> 16 > 100));
    }

    #[test]
    fn white_is_bright_but_not_uniform_on_a_slot_mask() {
        let (dst, w, h) = render(CrtProfile::Ctm644, &field([255, 255, 255]));
        let mean = dst.iter().map(|&p| (p >> 8 & 255) as f64).sum::<f64>() / dst.len() as f64;
        assert!(mean > 100.0 && mean < 255.0, "mean green {mean}");
        // mid-picture, the mask's gaps make the rows differ
        let row = |y: usize| dst[y * w + w / 3..y * w + w / 3 + 30].to_vec();
        assert!((0..4).any(|y| row(400 + y) != row(400)), "all rows alike in {h} rows");
    }

    #[test]
    fn a_grille_has_no_gap_between_rows_of_a_white_field() {
        let (dst, w, _) = render(CrtProfile::Gt65, &field([255, 255, 255]));
        // rows differ only by the scanlines' beam profile: same lit slots
        let lit = |y: usize| dst[y * w..(y + 1) * w].iter().filter(|&&p| p != 0).count();
        assert_eq!(lit(500), lit(501));
    }

    #[test]
    fn a_smaller_frame_is_letterboxed_in_black() {
        let src = RgbImage::from_pixel(100, 50, Rgb([255, 255, 255]));
        let (dst, w, h) = render(CrtProfile::Ctm644, &src);
        assert!(dst[(h - 4) * w + w - 8] == 0);
        assert!(dst[40 * w..41 * w][..200].iter().any(|&p| p != 0), "the picture itself must show");
    }
}
