//! Play a CSL script: `cargo run --release --example run_csl -- <script.csl> <screenshot dir>`
//!
//! Screenshots requested by SSM codes are written as
//! `AMSRUST_<crtc>_<code>.png` in the given directory.

use amsrust::csl_runner::CslRunner;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(script), Some(output)) = (args.next(), args.next()) else {
        eprintln!("usage: run_csl <script.csl> <screenshot dir>");
        std::process::exit(2);
    };

    let mut runner = CslRunner::new(output);
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
        },
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
