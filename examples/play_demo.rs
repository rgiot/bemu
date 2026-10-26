//! Boots a CPC disc and lets whatever program it auto-selects run,
//! grabbing a screenshot every so often - the generic complement to
//! `run_csl` (which needs a script) for demos and other discs that don't
//! come with one, like Benediction's own (<http://benediction.cpcscene.net/releases.html>).
//!
//! ```text
//! cargo run --release --example play_demo -- <disc.dsk> <output_dir> \
//!     [--run NAME] [--crtc 0-4] [--seconds N] [--every N] [--boot-wait N] [--list]
//! ```
//!
//! With no `--run`, the program to `RUN"..."` is picked from the disc's own
//! AMSDOS catalog (see [`amsrust::disc_image::DiscImage::amsdos_catalog`]):
//! many demo discs replace their catalog with "CATART" (entries whose
//! name/extension bytes are cursor and colour control codes rather than
//! text, so `CAT` itself draws a picture) - [`AmsdosEntry::is_plausible_filename`]
//! tells those apart from real files. `--list` prints the catalog (marking
//! which entries look like CATART) and exits, without running anything -
//! useful to check the pick, or to find the right `--run` name by hand.
//!
//! `--seconds` (default 60) is how long the demo plays, `--every` (default
//! 2) how many seconds between screenshots, `--boot-wait` (default 8) how
//! long to leave between typing `RUN"..."` and the first screenshot -
//! loading is disc-timing-sensitive, so a demo that needs longer may show
//! the boot screen still in progress in the first frame or two; raise this
//! if so.
//!
//! Some discs `RUN` straight into the demo (`validation/Benediction/bnd4_package.dsk`'s
//! `#MENU` does); others are a small loader that shows credits and returns
//! to BASIC, waiting for a further command with its own parameters
//! (`validation/Benediction/HBL.DSK`'s `RUN"HBL.BAS"` does this - its own
//! `RUNME.TXT`, inside the original release archive, documents
//! `RUN"HBL" 'CRTC=<0-4>` and a `PART=` parameter). When a disc comes with
//! its own instructions like that, `--args` supplies them verbatim, typed
//! right after the closing quote: `--run HBL --args " 'CRTC=0"`.

use amsrust::csl_runner::CslRunner;
use amsrust::disc_image::{load_dsk, AmsdosEntry};
use std::path::PathBuf;

fn main() {
    let mut disc_path = None;
    let mut output_dir = None;
    let mut run_name: Option<String> = None;
    let mut run_args = String::new();
    let mut crtc = 0u8;
    let mut seconds = 60u64;
    let mut every = 2u64;
    let mut boot_wait = 8u64;
    let mut list_only = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--run" => run_name = Some(args.next().expect("--run needs a name")),
            "--args" => run_args = args.next().expect("--args needs the text to type after the closing quote"),
            "--crtc" => crtc = args.next().expect("--crtc needs 0-4").parse().expect("--crtc must be 0-4"),
            "--seconds" => seconds = args.next().expect("--seconds needs a number").parse().expect("bad number"),
            "--every" => every = args.next().expect("--every needs a number").parse().expect("bad number"),
            "--boot-wait" => boot_wait = args.next().expect("--boot-wait needs a number").parse().expect("bad number"),
            "--list" => list_only = true,
            _ if disc_path.is_none() => disc_path = Some(PathBuf::from(arg)),
            _ if output_dir.is_none() => output_dir = Some(PathBuf::from(arg)),
            other => panic!("unexpected argument {other:?}")
        }
    }
    let disc_path = disc_path.expect("usage: play_demo <disc.dsk> <output_dir> [options]");

    let bytes = std::fs::read(&disc_path).unwrap_or_else(|e| panic!("{}: {e}", disc_path.display()));
    let disc = load_dsk(&bytes).unwrap_or_else(|e| panic!("{}: {e}", disc_path.display()));
    let catalog = disc.amsdos_catalog();

    print_catalog(&catalog);
    if list_only {
        return;
    }

    let entry = run_name
        .as_deref()
        .map(|name| {
            catalog
                .iter()
                .find(|e| e.filename().eq_ignore_ascii_case(name) || e.name.eq_ignore_ascii_case(name))
                .unwrap_or_else(|| panic!("{name:?} is not on {}", disc_path.display()))
        })
        .or_else(|| pick_program(&catalog))
        .unwrap_or_else(|| panic!("no plausible file to run on {} (see the catalog above, try --run)", disc_path.display()));
    let program = entry.filename();
    eprintln!("-> RUN\"{program}\"{run_args}");

    let output_dir = output_dir.unwrap_or_else(|| PathBuf::from("demo_screenshots"));
    std::fs::create_dir_all(&output_dir).expect("can't create the output directory");

    let mut runner = CslRunner::new(&output_dir);
    runner.set_verbose(true);
    let disc_name = disc_path.file_name().unwrap().to_string_lossy().replace('\'', "");
    let mut script = format!(
        "csl_version 1.0\n\
         crtc_select {crtc}\n\
         reset\n\
         wait 3000000\n\
         disk_insert '{disc_name}'\n\
         key_delay 70000 70000 400000\n\
         key_output 'RUN\"{program}\"{}\\(RET)'\n\
         wait {}\n",
        escape_for_csl(&run_args),
        boot_wait * 1_000_000
    );
    // `every`/`seconds` are runtime values a fixed script string can't
    // repeat a variable number of times, so the "wait; screenshot" tail is
    // appended in a loop instead.
    let ticks = seconds / every.max(1);
    for _ in 0..ticks {
        script.push_str(&format!("wait {}\nscreenshot\n", every * 1_000_000));
    }

    let base_dir = disc_path.parent().unwrap_or_else(|| std::path::Path::new("."));
    match runner.run_text(&script, base_dir) {
        Ok(report) => {
            eprintln!(
                "{:.1}s emulated, {} screenshots, {} SSM codes, {} ignored",
                report.elapsed_us as f64 / 1e6,
                report.screenshots.len(),
                report.ssm_codes.len(),
                report.ignored.len()
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

/// Escapes `'`, `{` and `}` the way a CSL `key_output` string needs (see
/// `CSL-STANDARD-EN.pdf`), so `--args` can contain the quote AMSDOS
/// parameters typically need (`'CRTC=0`) without breaking out of the
/// single-quoted `key_output '...'` it gets embedded in.
fn escape_for_csl(text: &str) -> String {
    let mut escaped = String::new();
    for c in text.chars() {
        match c {
            '\'' => escaped.push_str(r"\(')"),
            '{' => escaped.push_str(r"\({)"),
            '}' => escaped.push_str(r"\(})"),
            c => escaped.push(c)
        }
    }
    escaped
}

fn print_catalog(catalog: &[AmsdosEntry]) {
    if catalog.is_empty() {
        eprintln!("(empty or unreadable catalog)");
        return;
    }
    for entry in catalog {
        let flag = if !entry.is_plausible_filename() {
            " (CATART, not a real file)"
        }
        else if entry.hidden {
            " (hidden)"
        }
        else {
            ""
        };
        eprintln!("  {}:{}{flag}", entry.user, entry.filename());
    }
}

/// A default entry to `RUN` when the caller doesn't name one: prefers an
/// obvious menu/loader name, then the biggest `.BIN`, then the biggest
/// `.BAS`, then whatever plausible entry sorts first - all among entries
/// [`AmsdosEntry::is_plausible_filename`] accepts (real files, not CATART).
fn pick_program(catalog: &[AmsdosEntry]) -> Option<&AmsdosEntry> {
    let real: Vec<&AmsdosEntry> = catalog.iter().filter(|e| e.is_plausible_filename()).collect();

    let by_name = |wanted: &str| real.iter().find(|e| e.name.eq_ignore_ascii_case(wanted)).copied();
    let contains = |needle: &str| {
        real.iter()
            .filter(|e| e.name.to_ascii_uppercase().contains(needle))
            .max_by_key(|e| e.records)
            .copied()
    };
    let biggest_with_extension = |extension: &str| {
        real.iter()
            .filter(|e| e.extension.eq_ignore_ascii_case(extension))
            .max_by_key(|e| e.records)
            .copied()
    };

    by_name("DISC")
        .or_else(|| contains("MENU"))
        .or_else(|| contains("INTRO"))
        .or_else(|| biggest_with_extension("BIN"))
        .or_else(|| biggest_with_extension("BAS"))
        .or_else(|| real.into_iter().min_by_key(|e| e.filename()))
}
