//! Filters a screenshot (a 768x272 PNG saved by `amsrust --screenshot` in
//! raw mode) through a CRT model, for tuning it without running a machine.
//!
//! ```text
//! cargo run --release --example crt_render -- in.png out.png ctm [triad_px] [bench]
//! ```

use amsrust::display::{CpuCrt, CrtParams, DisplayFilter, DisplayMode, RawFilter};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output, mode, rest @ ..] = &args[..]
    else {
        eprintln!("usage: crt_render in.png out.png raw|ctm|trinitron|gt65|atari [triad_px] [bench]");
        std::process::exit(2);
    };
    let frame = image::open(input).expect("cannot read the input").to_rgb8();
    let mode: DisplayMode = mode.parse().unwrap_or_else(|e| panic!("{e}"));
    let mut filter: Box<dyn DisplayFilter> = match (mode, rest.first().and_then(|s| s.parse::<usize>().ok())) {
        (DisplayMode::Crt(profile), Some(triad_px)) => {
            let params = CrtParams { triad_px, rows_per_line: triad_px * 4 / 3, ..profile.params() };
            Box::new(CpuCrt::new(params))
        },
        (DisplayMode::Crt(_), None) => mode.filter(),
        (DisplayMode::Raw, _) => Box::new(RawFilter)
    };
    if rest.iter().any(|a| a == "bench") {
        let (w, h) = filter.output_size();
        let mut buffer = vec![0u32; w * h];
        let start = std::time::Instant::now();
        let n = 50;
        for _ in 0..n {
            filter.render(&frame, &mut buffer);
        }
        eprintln!("{w}x{h}: {:.2} ms per frame", start.elapsed().as_secs_f64() * 1000.0 / n as f64);
    }
    filter.render_to_image(&frame).save(output).expect("cannot save");
}
