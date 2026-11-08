//! Play a CSL script: `cargo run --release --example run_csl -- <script.csl> <screenshot dir>`
//! (`--drive-a 3.5`, `--drive-b 3.5`: a double-sided 3.5" drive instead of a 3" one)
//!
//! Screenshots requested by SSM codes are written as
//! `AMSRUST_<crtc>_<code>.png` in the given directory.

use amsrust::csl_runner::CslRunner;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(script), Some(output)) = (args.next(), args.next()) else {
        eprintln!(
            "usage: run_csl <script.csl> <screenshot dir> [--drive-a 3|3.5] [--drive-b 3|3.5] [--display raw|ctm|..] [--curvature 0-4] [--black-level 0-255]"
        );
        std::process::exit(2);
    };

    let mut runner = CslRunner::new(output);
    let (mut display, mut curvature, mut black_level) =
        (amsrust::display::DisplayMode::Raw, 0.0f32, 0u8);
    while let Some(option) = args.next() {
        let unit = match option.as_str() {
            "--display" | "--curvature" | "--black-level" => {
                let value = args.next().unwrap_or_default();
                let bad = |e: String| -> ! {
                    eprintln!("{option}: {e}");
                    std::process::exit(2)
                };
                match option.as_str() {
                    "--display" => display = value.parse().unwrap_or_else(|e: String| bad(e)),
                    "--curvature" => {
                        curvature = value
                            .parse()
                            .unwrap_or_else(|e: std::num::ParseFloatError| bad(e.to_string()))
                    }
                    _ => {
                        black_level = value
                            .parse()
                            .unwrap_or_else(|e: std::num::ParseIntError| bad(e.to_string()))
                    }
                }
                continue;
            }
            "--drive-a" => 0,
            "--drive-b" => 1,
            _ => {
                eprintln!("unknown option {option}");
                std::process::exit(2);
            }
        };
        let kind = args
            .next()
            .unwrap_or_default()
            .parse()
            .unwrap_or_else(|e: String| {
                eprintln!("{option}: {e}");
                std::process::exit(2);
            });
        runner.set_drive_kind(unit, kind);
    }
    runner.set_display_filter((!display.is_raw()).then(|| display.filter_on(false, curvature)));
    runner.set_black_level(black_level);
    runner.set_verbose(true);
    match runner.run_file(&script) {
        Ok(report) => {
            eprintln!(
                "{} µs played, {} screenshots, {} SSM codes",
                report.elapsed_us,
                report.screenshots.len(),
                report.ssm_codes.len()
            );
            for ignored in report.ignored {
                eprintln!("ignored: {ignored}");
            }
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
