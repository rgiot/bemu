//! Plays every Shaker test-suite module (see `validation/Shaker_CSL/`,
//! downloaded from <https://shaker.logonsystem.eu/Shaker_CSL.zip>) end to
//! end, unattended: each module's `-0` script chains through `csl_load` into
//! `-1`..`-4`, which is how the suite itself walks all five CRTC types, so
//! running the five `-0` scripts covers everything.
//!
//! Screenshots are named the way <https://shaker.logonsystem.eu/tests> names
//! its own, `<test id>_CRTC<n>_<subtest>.png`, via the bundled
//! [`amsrust::shakerland::Manifest`], so they sit side by side with a copy
//! of that site's images for a direct visual comparison (module E has no
//! published test IDs yet, so those fall back to `E_CRTC<n>_<code>.png`).
//!
//! This takes hours (the suite drives real firmware in real time, and this
//! emulator is not yet real-time - see the performance note in the
//! project's own backlog). It is meant to be started and left running:
//!
//! ```text
//! cargo run --release --example run_shaker_suite -- [shaker_dir] [output_dir] [modules]
//! ```
//!
//! `shaker_dir` defaults to `validation/Shaker_CSL`, `output_dir` to
//! `shaker_results` (gitignored), and `modules` to `ABCDE` (e.g. pass `E` to
//! run only module E). Each module writes its screenshots to
//! `<output_dir>/<module>/` and a `summary.json` there with every SSM code
//! seen, every screenshot taken and everything the runner had to ignore or
//! paper over (a missing disc, a `wait_ssm0000` that never got its code...).
//! `<output_dir>/progress.log` is appended to as the suite goes, so its tail
//! shows where an unattended run currently is.

use amsrust::csl_runner::CslRunner;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let shaker_dir = args.next().unwrap_or_else(|| "validation/Shaker_CSL".to_owned());
    let output_dir = args.next().unwrap_or_else(|| "shaker_results".to_owned());
    let modules = args.next().unwrap_or_else(|| "ABCDE".to_owned());

    let shaker_dir = PathBuf::from(shaker_dir);
    let output_dir = PathBuf::from(output_dir);
    std::fs::create_dir_all(&output_dir).expect("can't create the output directory");

    // Panics are reported per module (with the wall time already spent),
    // rather than losing the whole run to whatever ran into an emulator bug.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let suite_start = Instant::now();
    log(&output_dir, "=== Shaker suite starting ===");
    log(&output_dir, &format!("shaker_dir = {}", shaker_dir.display()));
    log(&output_dir, &format!("output_dir = {}", output_dir.display()));
    log(&output_dir, &format!("modules    = {modules}"));

    for module in modules.chars() {
        let script = shaker_dir.join(format!("MODULE_{module}")).join(format!("SHAKER-{module}-0.CSL"));
        if !script.exists() {
            log(&output_dir, &format!("[{module}] skipped: no {}", script.display()));
            continue;
        }

        let module_dir = output_dir.join(module.to_string());
        std::fs::create_dir_all(&module_dir).expect("can't create the module's output directory");
        log(&output_dir, &format!("[{module}] starting ({})", script.display()));
        let started = Instant::now();

        let outcome = std::panic::catch_unwind(|| {
            let mut runner = CslRunner::new(&module_dir);
            runner.set_verbose(true);
            runner.set_module(module);
            runner.run_file(&script)
        });

        let wall = started.elapsed();
        match outcome {
            Ok(Ok(report)) => {
                write_summary(&module_dir, module, &report, wall);
                log(
                    &output_dir,
                    &format!(
                        "[{module}] done in {wall:.0?} (wall), {:.0?} emulated: {} screenshots, {} SSM codes, {} ignored",
                        std::time::Duration::from_micros(report.elapsed_us),
                        report.screenshots.len(),
                        report.ssm_codes.len(),
                        report.ignored.len()
                    )
                );
                for ignored in &report.ignored {
                    log(&output_dir, &format!("[{module}]   ignored: {ignored}"));
                }
            },
            Ok(Err(error)) => {
                log(&output_dir, &format!("[{module}] FAILED after {wall:.0?}: {error}"));
            },
            Err(panic) => {
                let message = panic_message(&panic);
                log(&output_dir, &format!("[{module}] PANICKED after {wall:.0?}: {message}"));
            }
        }
    }

    log(&output_dir, &format!("=== Shaker suite finished in {:.0?} ===", suite_start.elapsed()));
    std::panic::set_hook(default_hook);
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_owned()
    }
    else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    }
    else {
        "(non-string panic payload)".to_owned()
    }
}

fn write_summary(module_dir: &Path, module: char, report: &amsrust::csl_runner::CslReport, wall: std::time::Duration) {
    let mut json = String::new();
    let _ = write!(
        json,
        "{{\n  \"module\": \"{module}\",\n  \"wall_seconds\": {:.1},\n  \"elapsed_us\": {},\n  \"screenshots\": [\n",
        wall.as_secs_f64(),
        report.elapsed_us
    );
    for (i, shot) in report.screenshots.iter().enumerate() {
        let comma = if i + 1 == report.screenshots.len() { "" } else { "," };
        let _ = writeln!(
            json,
            "    {{\"path\": {:?}, \"ssm_code\": {}, \"crtc\": {}, \"at_us\": {}}}{comma}",
            shot.path.file_name().unwrap().to_string_lossy(),
            shot.ssm_code.map_or("null".to_owned(), |c| format!("{c}")),
            shot.crtc as u8,
            shot.at_us
        );
    }
    let _ = write!(json, "  ],\n  \"ssm_codes\": {:?},\n  \"ignored\": {:?}\n}}\n", report.ssm_codes, report.ignored);
    let _ = std::fs::write(module_dir.join("summary.json"), json);
}

fn log(output_dir: &Path, line: &str) {
    eprintln!("{line}");
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(output_dir.join("progress.log")) {
        let _ = writeln!(file, "{line}");
    }
}
