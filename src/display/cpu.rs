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
//!
//! Seen through curved glass ([`CpuCrt::curved`]), it is the beam's raster that bends, not the
//! phosphor mask, which is part of the glass: each slot reads the beam where the warp says (the
//! same taps as the flat model, their weights from a table of sub-pixel phases). Warping the finished
//! picture instead (as [`super::Curved`] does) resamples the fine pattern of the mask and shows
//! moire.

use super::crt::{coverage, CrtParams, Warp, SRC_HEIGHT, SRC_WIDTH};
use super::DisplayFilter;
use image::RgbImage;

/// Taps of the horizontal beam filter.
const TAPS: usize = 4;
/// Zeros on each side of a linear scanline, for the taps near the edges.
const PAD: usize = 2;
/// Sub-pixel phases of the beam's tap weights with curved glass.
const PHASES: usize = 1024;
/// One scanline in linear light, a padded plane per gun.
type Planes = [[f32; SRC_WIDTH + 2 * PAD]; 3];

struct Column {
    /// Index (before padding) of the first tap, in monitor pixels.
    first: i32,
    weights: [f32; TAPS],
    channel: usize,
}

struct RowMix {
    /// The scanline of weight `a`; `b` is the next one. May be out of range.
    line: i32,
    a: f32,
    b: f32,
}

struct Tables {
    columns: Vec<Column>,
    /// With curved glass: the beam's horizontal tap weights by sub-pixel phase, and the vertical
    /// mix of the two nearest scanlines by phase.
    phase_columns: Vec<[f32; TAPS]>,
    phase_rows: Vec<(f32, f32)>,
    rows: Vec<RowMix>,
    /// `pitch_rows` rows of the mask's signed distance, `out_w` each.
    mask: Vec<Vec<f32>>,
    /// `[c][x]`: 1 if output column `x` is lit by gun `c`, else 0 (with
    /// `slot` zeros of padding on each side).
    channel_masks: [Vec<f32>; 3],
    /// `gain` and the calibration of the mask's losses.
    scale: f32,
    linear: [f32; 256],
}

impl Tables {
    fn new(p: &CrtParams) -> Self {
        let (out_w, out_h) = p.output_size();
        let ratio = p.x_ratio();
        let slot = p.slot_px();
        // the taps of the beam at `centre` (in output pixels)
        let column = |centre: f32, channel: usize| {
            let u = centre / ratio;
            let first = (u - 0.5).floor() as i32 - 1;
            let mut weights = [0.0f32; TAPS];
            for (t, w) in weights.iter_mut().enumerate() {
                let d = u - ((first + t as i32) as f32 + 0.5);
                *w = (-d * d / (2.0 * p.beam_sigma_h * p.beam_sigma_h)).exp();
            }
            let sum: f32 = weights.iter().sum();
            weights.iter_mut().for_each(|w| *w /= sum);
            Column {
                first,
                weights,
                channel,
            }
        };
        // the beam is read at the slot's centre
        let columns = (0..out_w)
            .map(|x| column(((x / slot) as f32 + 0.5) * slot as f32, p.channel(x)))
            .collect();
        // the taps of a beam at `u` (in monitor pixels) only depend on frac(u - 1/2): see `column`
        let phase_columns = (0..PHASES)
            .map(|phase| column((phase as f32 / PHASES as f32 + 0.5) * ratio, 0).weights)
            .collect();
        let gauss = |d: f32| (-d * d / (2.0 * p.beam_sigma_v * p.beam_sigma_v)).exp();
        let phase_rows = (0..PHASES)
            .map(|phase| {
                let f = phase as f32 / PHASES as f32;
                (gauss(f), gauss(1.0 - f))
            })
            .collect();

        let rows = (0..out_h)
            .map(|y| {
                let f = (y as f32 + 0.5) / p.rows_per_line as f32 - 0.5;
                let line = f.floor() as i32;
                let gauss = |d: f32| (-d * d / (2.0 * p.beam_sigma_v * p.beam_sigma_v)).exp();
                RowMix {
                    line,
                    a: gauss(f - line as f32),
                    b: gauss(line as f32 + 1.0 - f),
                }
            })
            .collect();

        let mask = (0..p.pitch_rows())
            .map(|y| (0..out_w).map(|x| p.slot_distance(x, y)).collect())
            .collect();

        let mut channel_masks = [
            vec![0.0; out_w + 2 * slot],
            vec![0.0; out_w + 2 * slot],
            vec![0.0; out_w + 2 * slot],
        ];
        for x in 0..out_w {
            channel_masks[p.channel(x)][x + slot] = 1.0;
        }

        let mut linear = [0.0f32; 256];
        for (level, l) in linear.iter_mut().enumerate() {
            *l = (level as f32 / 255.0).powi(2);
        }

        Self {
            columns,
            phase_columns,
            phase_rows,
            rows,
            mask,
            channel_masks,
            scale: p.gain * p.white_norm(),
            linear,
        }
    }
}

/// The CRT model, rendered on the CPU.
pub struct CpuCrt {
    params: CrtParams,
    tables: Tables,
    /// The beam of every monitor scanline at every output column.
    lines: Vec<f32>,
    zeros: Vec<f32>,
    threads: usize,
    /// The glass the picture is seen through.
    warp: Warp,
    /// With curved glass: every scanline in linear light.
    planes: Vec<Planes>,
}

impl CpuCrt {
    pub fn new(params: CrtParams) -> Self {
        let (out_w, _) = params.output_size();
        let tables = Tables::new(&params);
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(16);
        Self {
            params,
            tables,
            lines: vec![0.0; SRC_HEIGHT * out_w],
            zeros: vec![0.0; out_w],
            threads,
            warp: Warp::FLAT,
            planes: Vec::new(),
        }
    }

    /// The same tube seen through the glass `warp`.
    pub fn curved(mut self, warp: Warp) -> Self {
        self.planes = if warp.is_flat() {
            Vec::new()
        } else {
            vec![[[0.0; SRC_WIDTH + 2 * PAD]; 3]; SRC_HEIGHT]
        };
        self.warp = warp;
        self
    }

    /// Render on the calling thread only (for tests and benchmarks).
    pub fn single_threaded(mut self) -> Self {
        self.threads = 1;
        self
    }

    pub fn params(&self) -> &CrtParams {
        &self.params
    }

    /// Scanline `j` of `src` in linear light, one plane per gun, padded with zeros.
    fn load_planes(tables: &Tables, src: &RgbImage, j: usize, planes: &mut Planes) {
        for plane in planes.iter_mut() {
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
    }

    /// Pass 1 for the scanlines `first..first + lines.len() / out_w`.
    fn horizontal(
        params: &CrtParams,
        tables: &Tables,
        src: &RgbImage,
        first: usize,
        lines: &mut [f32],
    ) {
        let out_w = params.output_size().0;
        let mut planes: Planes = [[0.0f32; SRC_WIDTH + 2 * PAD]; 3];
        for (n, out) in lines.chunks_exact_mut(out_w).enumerate() {
            Self::load_planes(tables, src, first + n, &mut planes);
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
        dst: &mut [u32],
    ) {
        let out_w = params.output_size().0;
        let slot = params.slot_px();
        let pitch = params.pitch_rows();
        let line_of = |j: i32| -> &[f32] {
            if (0..SRC_HEIGHT as i32).contains(&j) {
                &lines[j as usize * out_w..][..out_w]
            } else {
                zeros
            }
        };
        // light of each gun, with `slot` zeros on each side for the halation
        let mut lit = [
            vec![0.0f32; out_w + 2 * slot],
            vec![0.0f32; out_w + 2 * slot],
            vec![0.0f32; out_w + 2 * slot],
        ];

        for (n, out) in dst.chunks_exact_mut(out_w).enumerate() {
            let y = first + n;
            let mix = &tables.rows[y];
            let (la, lb) = (line_of(mix.line), line_of(mix.line + 1));
            let distance = &tables.mask[y % pitch];
            let [r, g, b] = &mut lit;
            let masks = &tables.channel_masks;
            let n = out_w;
            // Equal-length slices, so the loops below have no bounds checks
            // and vectorise.
            let (la, lb, distance) = (&la[..n], &lb[..n], &distance[..n]);
            let (m0, m1, m2) = (
                &masks[0][slot..][..n],
                &masks[1][slot..][..n],
                &masks[2][slot..][..n],
            );
            let (rc, gc, bc) = (
                &mut r[slot..][..n],
                &mut g[slot..][..n],
                &mut b[slot..][..n],
            );
            for x in 0..n {
                let level = mix.a * la[x] + mix.b * lb[x];
                let light = level * coverage(distance[x], params.spill, level) * tables.scale;
                rc[x] = light * m0[x];
                gc[x] = light * m1[x];
                bc[x] = light * m2[x];
            }
            Self::finish_row(params, &lit, out);
        }
    }

    /// The light of each gun on a row (with `slot` zeros on each side) through the glass's
    /// halation, gamma-encoded into `out`.
    fn finish_row(params: &CrtParams, lit: &[Vec<f32>; 3], out: &mut [u32]) {
        let slot = params.slot_px();
        let n = out.len();
        let (h, centre) = (params.halation, 1.0 - 2.0 * params.halation);
        let [r, g, b] = lit;
        let encode = |light: f32| (light.clamp(0.0, 1.0).sqrt() * 255.0 + 0.5) as i32;
        let (r_l, r_c, r_r) = (&r[..n], &r[slot..][..n], &r[2 * slot..][..n]);
        let (g_l, g_c, g_r) = (&g[..n], &g[slot..][..n], &g[2 * slot..][..n]);
        let (b_l, b_c, b_r) = (&b[..n], &b[slot..][..n], &b[2 * slot..][..n]);
        for x in 0..n {
            let red = centre * r_c[x] + h * (r_l[x] + r_r[x]);
            let green = centre * g_c[x] + h * (g_l[x] + g_r[x]);
            let blue = centre * b_c[x] + h * (b_l[x] + b_r[x]);
            out[x] = (encode(red) << 16 | encode(green) << 8 | encode(blue)) as u32;
        }
    }
}

impl CpuCrt {
    /// Pass 2 with curved glass, for the output rows `first..`: each slot reads the beam at its
    /// centre's place on the bent raster; the mask itself does not move.
    fn vertical_curved(
        params: &CrtParams,
        tables: &Tables,
        warp: Warp,
        planes: &[Planes],
        first: usize,
        dst: &mut [u32],
    ) {
        let (out_w, out_h) = params.output_size();
        let slot = params.slot_px();
        let pitch = params.pitch_rows();
        let ratio = params.x_ratio();
        let pixel = (1.0 / out_w as f32, 1.0 / out_h as f32);
        // the beam of gun `c` on scanline `j` at `u` - 1/2 = `i` + `weights`' phase (monitor pixels)
        let beam = |j: i32, c: usize, i: i32, weights: &[f32; TAPS]| -> f32 {
            let base = i - 1 + PAD as i32;
            if !(0..SRC_HEIGHT as i32).contains(&j)
                || base < 0
                || base as usize + TAPS > SRC_WIDTH + 2 * PAD
            {
                return 0.0;
            }
            let plane = &planes[j as usize][c][base as usize..][..TAPS];
            (0..TAPS).map(|t| weights[t] * plane[t]).sum()
        };
        let phase = |v: f32| -> (i32, usize) {
            let whole = v.floor();
            (
                whole as i32,
                (((v - whole) * PHASES as f32) as usize).min(PHASES - 1),
            )
        };
        let mut lit = [
            vec![0.0f32; out_w + 2 * slot],
            vec![0.0f32; out_w + 2 * slot],
            vec![0.0f32; out_w + 2 * slot],
        ];
        for (n, out) in dst.chunks_exact_mut(out_w).enumerate() {
            let y = first + n;
            let v = (y as f32 + 0.5) * pixel.1;
            let distance = &tables.mask[y % pitch];
            for light in &mut lit {
                light.fill(0.0);
            }
            for x in 0..out_w {
                let centre = ((x / slot) as f32 + 0.5) * slot as f32;
                let (su, sv, _) = warp.source(centre * pixel.0, v, pixel);
                let (line, row_phase) =
                    phase(sv * out_h as f32 / params.rows_per_line as f32 - 0.5);
                let (a, b) = tables.phase_rows[row_phase];
                let (i, column_phase) = phase(su * out_w as f32 / ratio - 0.5);
                let weights = &tables.phase_columns[column_phase];
                let c = params.channel(x);
                let level = a * beam(line, c, i, weights) + b * beam(line + 1, c, i, weights);
                lit[c][x + slot] =
                    level * coverage(distance[x], params.spill, level) * tables.scale;
            }
            Self::finish_row(params, &lit, out);
            // the glass's rounded edge fades the pixels themselves (as `crt.wgsl`)
            for (x, pixel_out) in out.iter_mut().enumerate() {
                let (_, _, alpha) = warp.source((x as f32 + 0.5) * pixel.0, v, pixel);
                if alpha < 1.0 {
                    let fade = |shift: u32| {
                        ((((*pixel_out >> shift) & 0xff) as f32 * alpha) as u32) << shift
                    };
                    *pixel_out = fade(16) | fade(8) | fade(0);
                }
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
        let Self {
            params,
            tables,
            lines,
            zeros,
            threads,
            warp,
            planes,
        } = self;
        let (params, tables, threads, warp) = (&*params, &*tables, *threads, *warp);

        let band = |total: usize| total.div_ceil(threads).max(1);
        if !warp.is_flat() {
            for (j, line) in planes.iter_mut().enumerate() {
                Self::load_planes(tables, src, j, line);
            }
            let planes: &[Planes] = planes;
            std::thread::scope(|scope| {
                for (i, chunk) in dst.chunks_mut(band(out_h) * out_w).enumerate() {
                    scope.spawn(move || {
                        Self::vertical_curved(params, tables, warp, planes, i * band(out_h), chunk)
                    });
                }
            });
            return;
        }
        std::thread::scope(|scope| {
            for (i, chunk) in lines.chunks_mut(band(SRC_HEIGHT) * out_w).enumerate() {
                let first = i * band(SRC_HEIGHT);
                if threads == 1 {
                    Self::horizontal(params, tables, src, first, chunk);
                } else {
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
                } else {
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
        let lin = |v: u8| (v as f32 / 255.0).powi(2);
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
            let at = |j: i32| {
                if (0..SRC_HEIGHT as i32).contains(&j) {
                    horizontal[j as usize]
                } else {
                    0.0
                }
            };
            gauss(f - j as f32) * at(j) + gauss(j as f32 + 1.0 - f) * at(j + 1)
        };
        let light = |x: isize, channel: usize| -> f32 {
            let (out_w, _) = p.output_size();
            if x < 0 || x as usize >= out_w || p.channel(x as usize) != channel {
                return 0.0;
            }
            let level = level_at(x as usize, y);
            level
                * coverage(p.slot_distance(x as usize, y), p.spill, level)
                * p.gain
                * p.white_norm()
        };
        let slot = p.slot_px() as isize;
        let encode = |l: f32| (255.0 * l.clamp(0.0, 1.0).sqrt()).round() as u32;
        let channel = |c: usize| {
            let x = x as isize;
            encode(
                (1.0 - 2.0 * p.halation) * light(x, c)
                    + p.halation * (light(x - slot, c) + light(x + slot, c)),
            )
        };
        channel(0) << 16 | channel(1) << 8 | channel(2)
    }

    #[test]
    fn matches_the_reference_model() {
        let mut src = RgbImage::new(SRC_WIDTH as u32, SRC_HEIGHT as u32);
        for (x, y, pixel) in src.enumerate_pixels_mut() {
            *pixel = Rgb([
                (x * 7 % 256) as u8,
                (y * 5 % 256) as u8,
                ((x / 3 + y) * 11 % 256) as u8,
            ]);
        }
        let profile = CrtProfile::Ctm644;
        let (dst, w, _) = render(profile, &src);
        let p = profile.params();
        for &(x, y) in &[
            (0usize, 0usize),
            (1, 1),
            (700, 400),
            (1439, 1087),
            (333, 2),
            (900, 903),
            (12, 640),
        ] {
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
        assert!(
            (0..4).any(|y| row(400 + y) != row(400)),
            "all rows alike in {h} rows"
        );
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
        assert!(
            dst[40 * w..41 * w][..200].iter().any(|&p| p != 0),
            "the picture itself must show"
        );
    }

    #[test]
    fn unbent_glass_reads_the_beam_as_the_flat_model() {
        let src = RgbImage::from_fn(SRC_WIDTH as u32, SRC_HEIGHT as u32, |x, y| {
            Rgb([
                (x * 7 % 256) as u8,
                (y * 13 % 256) as u8,
                ((x + y) % 256) as u8,
            ])
        });
        let (flat, w, h) = render(CrtProfile::Ctm644, &src);
        // the curved passes, with no bulge and corners too small to cut a pixel
        let mut curved = CpuCrt::new(CrtProfile::Ctm644.params()).curved(Warp {
            k: (0.0, 0.0),
            corner: 1e-6,
        });
        let mut dst = vec![0; w * h];
        curved.render(&src, &mut dst);
        for (i, (a, b)) in flat.iter().zip(&dst).enumerate() {
            // (the glass's edge still fades the outermost pixels a little)
            if i % w == 0 || i % w == w - 1 || i / w == 0 || i / w == h - 1 {
                continue;
            }
            for shift in [0, 8, 16] {
                let (a, b) = ((a >> shift & 0xff) as i32, (b >> shift & 0xff) as i32);
                assert!(
                    (a - b).abs() <= 1,
                    "pixel ({}, {}): {a} / {b}",
                    i % w,
                    i / w
                );
            }
        }
    }

    #[test]
    fn curved_glass_shows_no_colour_moire() {
        // a grey field through the bulge of a CTM: averaged over blocks of a few slots, every gun
        // gives the same light (warping the finished picture instead beats the R, G, B slots
        // against the pixel grid: 0.6 of 255 apart)
        let src = RgbImage::from_pixel(SRC_WIDTH as u32, SRC_HEIGHT as u32, Rgb([160, 160, 160]));
        let mut filter =
            CpuCrt::new(CrtProfile::Ctm644.params()).curved(Warp::of(CrtProfile::Ctm644, 1.0));
        let (w, h) = filter.output_size();
        let mut dst = vec![0; w * h];
        filter.render(&src, &mut dst);
        let (bw, bh) = (12, 16);
        for by in (h / 4..3 * h / 4 - bh).step_by(bh) {
            for bx in (w / 4..3 * w / 4 - bw).step_by(bw) {
                let mut sum = [0.0f32; 3];
                for y in by..by + bh {
                    for x in bx..bx + bw {
                        for (c, s) in sum.iter_mut().enumerate() {
                            *s += (dst[y * w + x] >> (8 * c) & 0xff) as f32 / (bw * bh) as f32;
                        }
                    }
                }
                let mean = sum.iter().sum::<f32>() / 3.0;
                assert!(
                    sum.iter().all(|s| (s - mean).abs() < 0.25),
                    "block ({bx}, {by}): {sum:?}"
                );
            }
        }
    }
}
