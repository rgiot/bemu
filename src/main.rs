//! `amsrust`: boots a machine with the given CRTC/disc(s)/snapshot, for
//! manually testing the emulator against real-world software.
//!
//! ```text
//! amsrust [--crtc 0-4] [--dsk path] [--dsk-b path] [--sna path]
//!         [--screenshot dir [--seconds N] [--every N]]
//! ```
//!
//! With no `--screenshot`, drops into the existing interactive monitor
//! (`amsrust::debug::Monitor`) once everything is loaded: `h` at its `>>`
//! prompt lists its commands (`run`/`next`/`step`/`continue`, breakpoints,
//! `info z`/`info crtc`/`info ga`, `disassemble`...) - a real debugger, for
//! stepping through what a piece of software actually does.
//!
//! With `--screenshot <dir>`, runs headless instead: plays for `--seconds`
//! (default 30) real-time seconds of emulated time, saving a screenshot to
//! `<dir>` every `--every` seconds (default 2) - useful to eyeball what a
//! disc/snapshot does without a terminal to type commands into.
//!
//! `--sna` restores CPU registers, the base 64K of RAM, the CRTC's
//! registers, the gate array's ink/mode/ROM-paging state and the PSG's
//! registers (see `amsrust::snapshot`); apply it after `--dsk` if the
//! software expects a particular disc already in the drive.

use amsrust::crtc::CrtcType;
use amsrust::debug::{Monitor, MonitorContext};
use amsrust::disc_image::load_dsk;
use amsrust::fdc::FdcDrive;
use amsrust::monitor::FrameGrabber;
use amsrust::motherboard::MotherboardBuilder;

struct Args {
    crtc: CrtcType,
    dsk_a: Option<String>,
    dsk_b: Option<String>,
    sna: Option<String>,
    screenshot_dir: Option<String>,
    seconds: u64,
    every: u64
}

fn parse_args() -> Args {
    let mut args = Args {
        crtc: CrtcType::default(),
        dsk_a: None,
        dsk_b: None,
        sna: None,
        screenshot_dir: None,
        seconds: 30,
        every: 2
    };

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut next = |name: &str| it.next().unwrap_or_else(|| panic!("{name} needs a value"));
        match arg.as_str() {
            "--crtc" => {
                let value = next("--crtc");
                args.crtc = value
                    .parse::<u8>()
                    .ok()
                    .and_then(|n| CrtcType::try_from(n).ok())
                    .unwrap_or_else(|| panic!("--crtc must be 0-4, got {value:?}"));
            },
            "--dsk" => args.dsk_a = Some(next("--dsk")),
            "--dsk-b" => args.dsk_b = Some(next("--dsk-b")),
            "--sna" => args.sna = Some(next("--sna")),
            "--screenshot" => args.screenshot_dir = Some(next("--screenshot")),
            "--seconds" => args.seconds = next("--seconds").parse().expect("--seconds must be a number"),
            "--every" => args.every = next("--every").parse().expect("--every must be a number"),
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            },
            other => {
                eprintln!("unknown argument {other:?}\n");
                print_help();
                std::process::exit(2);
            }
        }
    }
    args
}

fn print_help() {
    eprintln!(concat!(
        "amsrust [--crtc 0-4] [--dsk path] [--dsk-b path] [--sna path]\n",
        "        [--screenshot dir [--seconds N] [--every N]]\n",
        "\n",
        "With no --screenshot, drops into the interactive monitor once everything\n",
        "is loaded ('h' at its prompt lists its commands). With --screenshot,\n",
        "runs headless for --seconds (default 30), saving a screenshot every\n",
        "--every seconds (default 2)."
    ));
}

fn insert_disc(path: &str, drive: FdcDrive, motherboard: &mut amsrust::motherboard::Motherboard) {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let image = load_dsk(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
    motherboard.fdc_mut().insert_image(drive, Some(image));
}

fn main() {
    let args = parse_args();

    let mut motherboard = MotherboardBuilder::default().set_crtc(args.crtc).enable_interruptions().build();

    if let Some(path) = &args.dsk_a {
        insert_disc(path, FdcDrive::A, &mut motherboard);
    }
    if let Some(path) = &args.dsk_b {
        insert_disc(path, FdcDrive::B, &mut motherboard);
    }
    if let Some(path) = &args.sna {
        amsrust::snapshot::load_sna(&mut motherboard, path).unwrap_or_else(|e| panic!("{e}"));
    }

    match args.screenshot_dir {
        Some(dir) => run_headless(&mut motherboard, &dir, args.seconds, args.every),
        None => {
            let mut handle = |_motherboard: &mut amsrust::motherboard::Motherboard| false;
            let mut ctx = MonitorContext::new(Box::new(&mut handle));
            Monitor::cli_monitoring(&mut motherboard, &mut ctx);
        }
    }
}

/// T-states per second at the CPC's 4MHz clock.
const T_STATES_PER_SECOND: usize = 4_000_000;

fn run_headless(motherboard: &mut amsrust::motherboard::Motherboard, dir: &str, seconds: u64, every: u64) {
    std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("{dir}: {e}"));

    let target_t_states = seconds as usize * T_STATES_PER_SECOND;
    let every_t_states = (every.max(1) as usize * T_STATES_PER_SECOND).max(1);
    // The first screenshot is taken as soon as a frame is available (0
    // isn't reached by running first), the rest paced `every` seconds
    // apart from there.
    let mut next_shot_at = 0;
    let mut shot = 0usize;

    while motherboard.t_state_counter().value() < target_t_states {
        while motherboard.t_state_counter().value() < next_shot_at.min(target_t_states) {
            // between screenshots, run without collecting video (cheaper)
            motherboard.execute_next_opcode_no_video();
        }
        if motherboard.t_state_counter().value() >= target_t_states {
            break;
        }

        let mut grabber = FrameGrabber::default();
        let mut frame = None;
        // Capture the next whole frame once due, rather than every tick:
        // matches how the CSL runner and play_demo grab a screenshot.
        while frame.is_none() && motherboard.t_state_counter().value() < target_t_states {
            motherboard.execute_next_opcode_with_video(|(rgb, sync)| {
                grabber.receive(rgb, sync);
                // take_frame() consumes it: a later tick within this same
                // multi-tick opcode would otherwise immediately overwrite
                // a just-captured frame with the None the second take gives.
                if frame.is_none() {
                    frame = grabber.take_frame();
                }
            });
        }
        let Some(frame) = frame
        else {
            break; // ran out of time mid-frame
        };

        let path = format!("{dir}/frame_{shot:04}.png");
        frame.save(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        eprintln!("[{:>6}s] {path}", motherboard.t_state_counter().value() / T_STATES_PER_SECOND);
        shot += 1;
        next_shot_at = motherboard.t_state_counter().value() + every_t_states;
    }
    eprintln!("{shot} screenshot(s) over {seconds}s emulated, saved to {dir}/");
}
