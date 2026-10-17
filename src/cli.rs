//! Command line of the `amsrust` binary, independent of the window backend.

use amsrust::crtc::CrtcType;
use amsrust::display::DisplayMode;
use amsrust::motherboard::CpcModel;

/// The window backend, `--frontend`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrontendKind {
    Minifb,
    /// winit + wgpu: the CRT is drawn by the GPU, with no read-back
    /// (`--features gpu-window`).
    Winit
}

/// ACE-DL's own: ink black is a dark grey, not the 0 of the blanking.
pub const DEFAULT_BLACK_LEVEL: u8 = 8;

pub struct Args {
    pub crtc: CrtcType,
    pub model: CpcModel,
    /// Total RAM in KB (64 = no extra page), if not the model's.
    pub ram_kb: Option<usize>,
    /// Disc interface (a DDI-1 on a 464), if not the model's.
    pub disc_interface: Option<bool>,
    /// `--french`: the French 6128 ROMs, AZERTY keyboard.
    pub french: bool,
    pub dsk_a: Option<String>,
    pub dsk_b: Option<String>,
    pub sna: Option<String>,
    pub screenshot_dir: Option<String>,
    pub seconds: u64,
    pub every: u64,
    pub window: bool,
    pub mute: bool,
    /// `--turbo`: the window starts at full speed (F10 toggles it).
    pub turbo: bool,
    pub record: Option<String>,
    pub debug_server: bool,
    pub debug_port: u16,
    /// Start paused, for a debugger to arm its breakpoints first.
    pub debug_paused: bool,
    pub text: Option<String>,
    pub type_at: f64,
    /// How the monitor's frames are filtered for the window, the recording
    /// and the screenshots.
    pub display: DisplayMode,
    /// Render the CRT on the GPU (needs the `crt-gpu` feature).
    pub gpu: bool,
    pub frontend: FrontendKind,
    /// Gate Array fitted, if not the default (40010).
    pub gate_array: Option<amsrust::ga::GateArrayType>,
    /// Level a black ink is drawn at, above the blanking and the sync
    /// pulses (0, blacker than black); not with the debug server.
    pub black_level: u8
}

pub fn parse_args() -> Args {
    let mut args = Args {
        crtc: CrtcType::default(),
        model: CpcModel::default(),
        ram_kb: None,
        disc_interface: None,
        french: false,
        dsk_a: None,
        dsk_b: None,
        sna: None,
        screenshot_dir: None,
        seconds: 30,
        every: 2,
        window: false,
        mute: false,
        turbo: false,
        record: None,
        debug_server: false,
        debug_port: amsrust::debug_server::DEFAULT_PORT,
        debug_paused: false,
        text: None,
        type_at: 3.0,
        display: DisplayMode::default(),
        gpu: false,
        frontend: FrontendKind::Minifb,
        gate_array: None,
        black_level: DEFAULT_BLACK_LEVEL
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
            "--model" => {
                args.model = next("--model").parse().unwrap_or_else(|e: String| panic!("--model: {e}"))
            },
            "--ram" => {
                let kb: usize = next("--ram").parse().expect("--ram must be a size in KB");
                assert!(kb >= 64 && kb % 64 == 0 && kb <= 576, "--ram must be 64, 128, ... 576 (KB)");
                args.ram_kb = Some(kb);
            },
            "--ddi" => args.disc_interface = Some(true),
            "--no-disc" => args.disc_interface = Some(false),
            "--french" => args.french = true,
            "--dsk" => args.dsk_a = Some(next("--dsk")),
            "--dsk-b" => args.dsk_b = Some(next("--dsk-b")),
            "--sna" => args.sna = Some(next("--sna")),
            "--screenshot" => args.screenshot_dir = Some(next("--screenshot")),
            "--seconds" => args.seconds = next("--seconds").parse().expect("--seconds must be a number"),
            "--every" => args.every = next("--every").parse().expect("--every must be a number"),
            "--window" => args.window = true,
            "--mute" => args.mute = true,
            "--turbo" => args.turbo = true,
            "--record" => args.record = Some(next("--record")),
            "--debug-server" => args.debug_server = true,
            "--debug-paused" => {
                args.debug_server = true;
                args.debug_paused = true;
            },
            "--debug-port" => {
                args.debug_server = true;
                args.debug_port = next("--debug-port").parse().expect("--debug-port must be a port number")
            },
            "--type" => args.text = Some(next("--type").replace("\\n", "\n")),
            "--type-at" => args.type_at = next("--type-at").parse().expect("--type-at must be a number"),
            "--black-level" => {
                args.black_level = next("--black-level").parse().expect("--black-level must be 0-255")
            },
            "--ga" => {
                args.gate_array = Some(match next("--ga").as_str() {
                    "40010" => amsrust::ga::GateArrayType::Ga40010,
                    "40007" | "40008" => amsrust::ga::GateArrayType::Ga40007_8,
                    other => panic!("--ga must be 40007, 40008 or 40010, got {other:?}")
                })
            },
            "--frontend" => {
                args.frontend = match next("--frontend").as_str() {
                    "minifb" => FrontendKind::Minifb,
                    "winit" => FrontendKind::Winit,
                    other => panic!("--frontend must be minifb or winit, got {other:?}")
                }
            },
            "--crt-gpu" => args.gpu = true,
            "--display" => {
                args.display = next("--display").parse().unwrap_or_else(|e: String| panic!("--display: {e}"))
            },
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
        "amsrust [--model 464|664|6128] [--french] [--ram KB] [--ddi|--no-disc] [--crtc 0-4]\n",
        "        [--dsk path] [--dsk-b path] [--sna path]\n",
        "        [--screenshot dir [--seconds N] [--every N]] [--window [--mute] [--turbo]]\n",
        "        [--record file [--seconds N]] [--type text [--type-at N]]\n",
        "        [--debug-server [--debug-port N] [--debug-paused]]\n",
        "        [--display raw|ctm|trinitron|gt65|atari [--crt-gpu]]\n",
        "        [--black-level 0-255] [--frontend minifb|winit] [--ga 40007|40008|40010]\n",
        "\n",
        "--model picks the ROMs (default 6128); --french the French ones of the 6128,\n",
        "with its AZERTY keyboard (typed text and keys follow it). The 464 and 664 have 64K and the\n",
        "6128 128K: --ram 128 (up to 576) adds the extension banked as a 6128's.\n",
        "The 664 and 6128 have a disc interface; --ddi adds a DDI-1 (AMSDOS) to\n",
        "a 464, --no-disc removes it.\n",
        "With no --screenshot/--window, drops into the interactive monitor once\n",
        "everything is loaded ('h' at its prompt lists its commands). With\n",
        "--screenshot, runs headless for --seconds (default 30), saving a\n",
        "screenshot every --every seconds (default 2). With --window, opens a\n",
        "live, playable window redrawn at the CPC's own ~50Hz, keyboard included\n",
        "- close the window to quit. Built with --features audio, the window\n",
        "plays the sound too (--mute to silence it). F10 toggles full speed: the\n",
        "emulation runs as fast as it can, silent, the picture shown 50 times a\n",
        "second (--turbo starts so). --record capture.mp4 records\n",
        "a video with sound through ffmpeg: what the window shows until it is\n",
        "closed, or without --window --seconds (default 30) run headless.\n",
        "--type 'run\"demo\\n' types on the CPC keyboard after --type-at seconds\n",
        "(default 3). Breakpoints of a snapshot's REMU/BRKC/BRKS/WABP chunks are set\n",
        "in the interactive monitor.\n",
        "--display picks how the picture is rendered, in the window, the\n",
        "recording and the screenshots (raw: the monitor's ideal picture; ctm,\n",
        "trinitron, gt65, atari: CRT approximations). F9 cycles them in the\n",
        "window (not while recording). --crt-gpu renders them on the GPU (a build\n",
        "with --features crt-gpu). --black-level (default 8) is what a black ink is\n",
        "drawn at: on a real tube it is a dark grey, blanking and sync being\n",
        "blacker (0); without the debug server, whose pictures stay exact.\n",
        "--frontend winit (a build with --features gpu-window) opens a window the\n",
        "GPU draws, the CRT included, with nothing read back; F11 toggles its\n",
        "fullscreen (a 50 Hz video mode when the monitor has one: frames on the\n",
        "vertical sync). F4 closes the window. --ga picks the Gate\n",
        "Array (default 40010): the 40007/8 are one pixel ahead (palette writes).\n",
        "--debug-server serves the AMSpiriT-Lite compatible debug API on\n",
        "127.0.0.1:6128 (GET /api/doc lists the endpoints), with --window or\n",
        "headless. --debug-paused starts it paused, for a debugger to set its\n",
        "breakpoints before anything runs."
    ));
}
