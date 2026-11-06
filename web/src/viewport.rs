/// Fit a picture in the physical canvas. CTM's phosphor pixels are preserved when possible.
pub fn fit(canvas: (u32, u32), picture: (usize, usize), integer: bool) -> (f32, f32, f32, f32, bool) {
    let (cw, ch) = canvas;
    let (pw, ph) = picture;
    let scale = (cw / pw as u32).min(ch / ph as u32);
    if integer && scale > 0 {
        let (w, h) = (pw as u32 * scale, ph as u32 * scale);
        return (((cw - w) / 2) as f32, ((ch - h) / 2) as f32, w as f32, h as f32, true);
    }
    let scale = (cw as f32 / pw as f32).min(ch as f32 / ph as f32);
    let (w, h) = (pw as f32 * scale, ph as f32 * scale);
    ((cw as f32 - w) / 2.0, (ch as f32 - h) / 2.0, w, h, false)
}

#[cfg(test)]
mod tests {
    use super::fit;

    #[test]
    fn ctm_uses_integer_scale_and_pixel_aligned_origin() {
        assert_eq!(fit((1921, 1201), (1440, 1088), true), (240.0, 56.0, 1440.0, 1088.0, true));
        assert_eq!(fit((3841, 2501), (1440, 1088), true), (480.0, 162.0, 2880.0, 2176.0, true));
        assert_eq!(fit((1440, 1088), (1440, 1088), true), (0.0, 0.0, 1440.0, 1088.0, true));
    }

    #[test]
    fn small_ctm_and_other_modes_keep_fractional_fit() {
        assert_eq!(fit((720, 544), (1440, 1088), true), (0.0, 0.0, 720.0, 544.0, false));
        assert_eq!(fit((720, 1088), (1440, 1088), true), (0.0, 272.0, 720.0, 544.0, false));
        assert_eq!(fit((2160, 1632), (1440, 1088), false), (0.0, 0.0, 2160.0, 1632.0, false));
    }
}
