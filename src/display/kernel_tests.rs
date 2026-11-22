//! The behaviour of the CRT model, tested through its kernels run on CubeCL's CPU backend (no GPU
//! needed): the mask, the beam, the glass, the glow, the veil, the vignette, the mono tubes.

use super::crt::{CrtParams, CrtProfile, Warp, SRC_HEIGHT, SRC_WIDTH};
use super::kernel_crt::{Backend, KernelCrt};
use super::DisplayFilter;
use image::{Rgb, RgbImage};


fn field(color: [u8; 3]) -> RgbImage {
    RgbImage::from_pixel(SRC_WIDTH as u32, SRC_HEIGHT as u32, Rgb(color))
}

/// A profile's parameters without the veil, for the tests of the mask and the beam, where a
/// black has to be 0.
fn bare(profile: CrtProfile) -> CrtParams {
    CrtParams {
        veil: 0.0,
        vignette: 0.0,
        glow: 0.0,
        ..profile.params()
    }
}

fn render(profile: CrtProfile, src: &RgbImage) -> (Vec<u32>, usize, usize) {
    let mut filter = KernelCrt::new(bare(profile), Warp::FLAT, Backend::Cpu).unwrap();
    let (w, h) = filter.output_size();
    let mut dst = vec![0; w * h];
    filter.render(src, &mut dst);
    (dst, w, h)
}

/// The brightest pixel of a render, as `[r, g, b]`.
fn brightest(profile: CrtProfile, color: [u8; 3]) -> [i32; 3] {
    let (dst, _, _) = render(profile, &field(color));
    let best = dst
        .iter()
        .copied()
        .max_by_key(|p| (p >> 16 & 255) + (p >> 8 & 255) + (p & 255));
    let p = best.unwrap();
    [
        (p >> 16 & 255) as i32,
        (p >> 8 & 255) as i32,
        (p & 255) as i32,
    ]
}

/// The GT64/GT65 is a green tube and the MM12 a grey one: every slot shows the luminance of the
/// picture in the colour of the phosphor, whatever the colour of the picture.
#[test]
fn mono_tubes_show_the_luminance_in_their_phosphor() {
    let [r, g, b] = brightest(CrtProfile::Gt65, [255, 255, 255]);
    assert!(
        g > 200 && r < g / 2 && b < g * 2 / 3 && b > r,
        "green: {r} {g} {b}"
    );
    let [r, g, b] = brightest(CrtProfile::Mm12, [255, 255, 255]);
    assert!(
        g > 180 && (r - g).abs() < 30 && (b - g).abs() < 30,
        "grey: {r} {g} {b}"
    );
    // luminance: green is brighter than red, and red brighter than blue
    let green = brightest(CrtProfile::Gt65, [0, 255, 0])[1];
    let red = brightest(CrtProfile::Gt65, [255, 0, 0])[1];
    let blue = brightest(CrtProfile::Gt65, [0, 0, 255])[1];
    assert!(
        green > red && red > blue && blue > 0,
        "{green} {red} {blue}"
    );
    // a colour tube keeps its colours
    let [r, g, b] = brightest(CrtProfile::Ctm644, [255, 0, 0]);
    assert!(r > 100 && g == 0 && b == 0);
}

/// A tube never is black: a black picture shows the veil of the glass, the share of a white's
/// light the photographs of real CTMs give (3.4 %: a pixel of 47 of 255 on the encoded scale),
/// the same on every slot whatever its gun; a mono tube in its phosphor's colour.
#[test]
fn the_edges_are_darker_than_the_centre() {
    let mut filter = KernelCrt::new(CrtProfile::Ctm644.params(), Warp::FLAT, Backend::Cpu).unwrap();
    let (w, h) = filter.output_size();
    let mut white = vec![0; w * h];
    filter.render(&field([255, 255, 255]), &mut white);
    let mean = |x0: usize, y0: usize| {
        let mut sum = 0.0;
        for y in y0..y0 + 8 {
            for x in x0..x0 + 8 {
                sum += ((white[y * w + x] & 255) as f64 / 255.0).powi(2);
            }
        }
        sum / 64.0
    };
    let (centre, edge) = (mean(w / 2, h / 2), mean(w - 9, h / 2));
    // the photographs: 0.72 of the centre's at the middle of an edge
    assert!((edge / centre - 0.71).abs() < 0.08, "{}", edge / centre);
}

/// What is bright lights what is around it (TuxAPE's glow): the dark beside a white area is
/// lit more than the dark far from it, which is lit more than nothing, and a black picture is
/// black.
#[test]
fn the_bright_glows_on_the_dark() {
    let params = || CrtParams {
        vignette: 0.0,
        ..CrtProfile::Ctm644.params()
    };
    let mut filter = KernelCrt::new(params(), Warp::FLAT, Backend::Cpu).unwrap();
    let (w, h) = filter.output_size();
    let mut dst = vec![0; w * h];
    filter.render(&field([0, 0, 0]), &mut dst);
    assert!(dst.iter().all(|&p| p == 0), "a black picture is black");
    // the left quarter white
    let mut src = field([0, 0, 0]);
    for y in 0..SRC_HEIGHT as u32 {
        for x in 0..(SRC_WIDTH / 4) as u32 {
            src.put_pixel(x, y, Rgb([255, 255, 255]));
        }
    }
    filter.render(&src, &mut dst);
    let light = |x: usize| {
        let row = h / 2;
        (0..12)
            .map(|d| {
                let p = dst[row * w + x + d];
                ((p >> 8 & 255) as f64 / 255.0).powi(2)
            })
            .sum::<f64>()
            / 12.0
    };
    let (near, mid, far) = (light(w / 4 + 10), light(w / 2), light(w - 60));
    assert!(near > mid && mid > far && far > 0.0, "{near} {mid} {far}");
    // a few hundredths beside it, a trace far across
    assert!((0.01..0.12).contains(&near), "{near}");
    assert!(far < 0.005, "{far}");
}

/// A profile's parameters with an even light, for the tests of the veil.
fn flat_light(profile: CrtProfile) -> CrtParams {
    CrtParams {
        vignette: 0.0,
        glow: 0.0,
        veil: 0.035,
        ..CrtParams::from_profile(profile)
    }
}

#[test]
fn a_black_picture_shows_the_veil() {
    let black = field([0, 0, 0]);
    for (profile, tint) in [
        (CrtProfile::Ctm644, [1.0, 1.0, 1.0]),
        (CrtProfile::Gt65, [0.12, 1.0, 0.16]),
    ] {
        let mut filter = KernelCrt::new(flat_light(profile), Warp::FLAT, Backend::Cpu).unwrap();
        let (w, h) = filter.output_size();
        let mut dst = vec![0; w * h];
        filter.render(&black, &mut dst);
        let want = |t: f32| (flat_light(profile).veil_light() * t).sqrt() * 255.0;
        let [r, g, b] = [
            (dst[0] >> 16 & 255) as f32,
            (dst[0] >> 8 & 255) as f32,
            (dst[0] & 255) as f32,
        ];
        for (got, t) in [(r, tint[0]), (g, tint[1]), (b, tint[2])] {
            assert!((got - want(t)).abs() <= 1.0, "{profile:?}: {got} for {}", want(t));
        }
        assert!(dst.iter().all(|&p| p == dst[0]), "the veil is the same everywhere");
    }
    // 3.5 % of the mean light of a white field: a black of about 35 on the encoded scale,
    // where it was 0
    let r = flat_light(CrtProfile::Ctm644).veil_light().sqrt() * 255.0;
    assert!((33.0..=37.0).contains(&r), "{r}");
    // and measured on a white field, it is the share of the photographs
    let (white, w, _) = render(CrtProfile::Ctm644, &field([255, 255, 255]));
    let (dark, ..) = {
        let mut filter = KernelCrt::new(flat_light(CrtProfile::Ctm644), Warp::FLAT, Backend::Cpu).unwrap();
        let (w, h) = filter.output_size();
        let mut dst = vec![0; w * h];
        filter.render(&black, &mut dst);
        (dst, w, h)
    };
    let mean = |image: &[u32], row: usize| {
        let row = &image[row * w..(row + 1) * w];
        row.iter()
            .map(|&p| {
                [16, 8, 0]
                    .iter()
                    .map(|s| ((p >> s & 255) as f64 / 255.0).powi(2))
                    .sum::<f64>()
                    / 3.0
            })
            .sum::<f64>()
            / w as f64
    };
    let (white, dark) = (
        (400..404).map(|y| mean(&white, y)).sum::<f64>() / 4.0,
        (400..404).map(|y| mean(&dark, y)).sum::<f64>() / 4.0,
    );
    // (a scanline's gap is darker than a line's middle: the rows average over four)
    assert!((dark / white - 0.035).abs() < 0.012, "ratio {} dark {dark} white {white} veil {}", dark / white, flat_light(CrtProfile::Ctm644).veil_light());
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
    let mut curved = KernelCrt::new(bare(CrtProfile::Ctm644), Warp {
        k: (0.0, 0.0),
        corner: 1e-6,
    }, Backend::Cpu).unwrap();
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
        KernelCrt::new(bare(CrtProfile::Ctm644), Warp::of(CrtProfile::Ctm644, 1.0), Backend::Cpu).unwrap();
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

/// Just the beam of a CTM: no veil, glow or vignette, so that what the new touches do shows.
fn beam_only(edit: impl FnOnce(&mut CrtParams)) -> KernelCrt {
    let mut params = CrtParams {
        veil: 0.0,
        vignette: 0.0,
        glow: 0.0,
        ..CrtProfile::Ctm644.params()
    };
    edit(&mut params);
    KernelCrt::new(params, Warp::FLAT, Backend::Cpu).unwrap()
}

fn render_with(filter: &mut KernelCrt, src: &RgbImage) -> (Vec<u32>, usize, usize) {
    let (w, h) = filter.output_size();
    let mut dst = vec![0; w * h];
    filter.render(src, &mut dst);
    (dst, w, h)
}

/// The mean of a channel (0: blue, 1: green, 2: red, the low byte first) over the picture's slots of
/// that gun, in light.
fn channel_light(dst: &[u32], w: usize, gun: usize) -> f64 {
    let shift = 16 - 8 * gun;
    let (mut sum, mut n) = (0.0, 0usize);
    for (i, p) in dst.iter().enumerate() {
        if i % w % 3 == gun {
            sum += ((p >> shift & 255) as f64 / 255.0).powi(2);
            n += 1;
        }
    }
    sum / n as f64
}

/// The tube's white is a cold one (TuxAPE; the photographs' whites are bluer still): blue above
/// green above red on a white field, and the three equal on a tube without it.
#[test]
fn the_white_of_a_ctm_is_cold() {
    let white = field([255, 255, 255]);
    let (cold, w, _) = render_with(&mut beam_only(|_| {}), &white);
    let (r, g, b) = (channel_light(&cold, w, 0), channel_light(&cold, w, 1), channel_light(&cold, w, 2));
    assert!(b > g && g > r, "red {r}, green {g}, blue {b}");
    let (neutral, w, _) = render_with(&mut beam_only(|p| p.white = [1.0; 3]), &white);
    let (r, g, b) = (channel_light(&neutral, w, 0), channel_light(&neutral, w, 1), channel_light(&neutral, w, 2));
    assert!((r - g).abs() < 0.03 && (g - b).abs() < 0.03, "red {r}, green {g}, blue {b}");
}

/// Where, along a row, a gun's light falls under half of its level on the white side of an edge
/// (white on the left, black on the right): interpolated between the gun's own slots.
fn edge_position(dst: &[u32], w: usize, y: usize, gun: usize) -> f64 {
    let shift = 16 - 8 * gun;
    let slots: Vec<(usize, f64)> = (gun..w).step_by(3).map(|x| (x, (dst[y * w + x] >> shift & 255) as f64)).collect();
    let top = slots.iter().take(20).map(|s| s.1).fold(0.0, f64::max);
    let at = slots.iter().position(|s| s.1 < top / 2.0).expect("an edge");
    let ((x0, v0), (x1, v1)) = (slots[at - 1], slots[at]);
    x0 as f64 + (x1 - x0) as f64 * (v0 - top / 2.0) / (v0 - v1)
}

/// The blue gun lands to the right of the red and green (TuxAPE: a fifth of a pixel).
#[test]
fn blue_lands_to_the_right_of_red() {
    let mut src = field([0, 0, 0]);
    for y in 0..SRC_HEIGHT as u32 {
        for x in 0..(SRC_WIDTH / 2) as u32 {
            src.put_pixel(x, y, Rgb([255, 255, 255]));
        }
    }
    let shifted = |filter: &mut KernelCrt| {
        let (dst, w, h) = render_with(filter, &src);
        // averaged over a few rows of the middle: the mask's stagger moves the slots about
        let rows = [h / 2, h / 2 + 4, h / 2 + 8];
        let at = |gun| rows.iter().map(|&y| edge_position(&dst, w, y, gun)).sum::<f64>() / rows.len() as f64;
        at(2) - at(0)
    };
    let with = shifted(&mut beam_only(|p| p.white = [1.0; 3]));
    let without = shifted(&mut beam_only(|p| {
        p.white = [1.0; 3];
        p.blue_shift = 0.0;
        p.blue_width = 1.0;
    }));
    // (the blue slots sit two pixels right of the red's in a triad: that is the same in both)
    assert!(with - without > 0.4 && with - without < 3.0, "blue edge {with} px from red's, {without} without");
}

/// How much the scanlines show on a grey field: the dark between two lines against the light on
/// them, over the rows of a period.
fn scanline_contrast(filter: &mut KernelCrt, level: u8) -> f64 {
    let (dst, w, h) = render_with(filter, &field([level, level, level]));
    let rows = 4;
    let mut sum = vec![0.0f64; rows];
    for y in h / 4..3 * h / 4 {
        for x in (w / 4..3 * w / 4).step_by(1) {
            sum[y % rows] += ((dst[y * w + x] >> 8 & 255) as f64 / 255.0).powi(2);
        }
    }
    let (min, max) = (sum.iter().cloned().fold(f64::MAX, f64::min), sum.iter().cloned().fold(0.0, f64::max));
    (max - min) / (max + min)
}

/// The beam is finer at low light, so the scanlines show more there than with a constant width.
#[test]
fn the_beam_is_finer_at_low_light() {
    let thin = scanline_contrast(&mut beam_only(|p| p.white = [1.0; 3]), 100);
    let constant = scanline_contrast(&mut beam_only(|p| {
        p.white = [1.0; 3];
        p.beam_thin = 1.0;
    }), 100);
    assert!(thin > constant, "contrast {thin} with a finer beam at low light, {constant} without");
    // and at a white the two are the same beam
    let white_thin = scanline_contrast(&mut beam_only(|p| p.white = [1.0; 3]), 255);
    let white_constant = scanline_contrast(&mut beam_only(|p| {
        p.white = [1.0; 3];
        p.beam_thin = 1.0;
    }), 255);
    assert!((white_thin - white_constant).abs() < 0.02, "{white_thin} / {white_constant}");
}
