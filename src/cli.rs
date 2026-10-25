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
    /// `--playcity`: a PlayCity (two more AYs and a CTC).
    pub playcity: bool,
    /// `--digiblaster`: an 8-bit DAC on the printer port.
    pub digiblaster: bool,
    pub dsk_a: Option<String>,
    pub dsk_b: Option<String>,
    /// `--cdt path`: a tape in the cassette deck, PLAY down.
    pub cdt: Option<String>,
    pub sna: Option<String>,
    pub screenshot_dir: Option<String>,
    pub seconds: u64,
    /// How often a screenshot is taken.
    pub every: Every,
    /// `--from S`: the first screenshot at S seconds.
    pub from: f64,
    pub window: bool,
    pub mute: bool,
    /// `--turbo`: the window starts at full speed (F10 toggles it).
    pub turbo: bool,
    /// `--watch`: the files are taken again when they change.
    pub watch: bool,
    /// `--overlay`: the timing overlay shown (window) or drawn (screenshots).
    pub overlay: bool,
    pub record: Option<String>,
    /// `--record-ym file`: the AY's registers frame by frame, as a YM6 file.
    pub record_ym: Option<String>,
    /// `--record-wav file`: the sound as a WAV file.
    pub record_wav: Option<String>,
    /// `--sym FILE` (repeatable): symbol files, for the debuggers.
    pub symbols: Vec<String>,
    /// `--rom SLOT=FILE` (repeatable): upper ROMs.
    pub roms: Vec<(u8, String)>,
    /// `--m4 DIR`: an M4 board, its SD card the folder.
    pub m4: Option<String>,
    /// `--m4-rom FILE`: another ROM for the M4 than the bundled one.
    pub m4_rom: Option<String>,
    /// `--m4-slot N`: the slot of the M4's ROM (6 by default).
    pub m4_slot: Option<u8>,
    /// `--albireo PATH`: an Albireo, its USB key a folder or a FAT image.
    pub albireo: Option<String>,
    /// `--nova FILE`: a Nova, its NVRAM kept in the file.
    pub nova: Option<String>,
    /// `--booster` (a CPC Booster+) or `--miniboster`.
    pub booster: Option<amsrust::booster::Model>,
    /// `--booster-uart loopback|listen:ADDR|connect:ADDR`: what its serial port is wired to.
    pub booster_uart: Option<String>,
    /// `--booster-eeprom FILE`: its 512 bytes of EEPROM, kept in the file.
    pub booster_eeprom: Option<String>,
    /// `--booster-flash FILE`: its 16K of program memory (a dump of the BIOS), kept in the file.
    pub booster_flash: Option<String>,
    /// `--booster-adc FILE`: what its ADC samples (a WAV or raw unsigned bytes), looped.
    pub booster_adc: Option<String>,
    /// `--save-sna path`: the machine's state saved there when the run ends.
    pub save_sna: Option<String>,
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
    /// How curved a CRT's glass is, times its profile's bulge (0: flat).
    pub curvature: f32,
    pub frontend: FrontendKind,
    /// Gate Array fitted, if not the default (40010).
    pub gate_array: Option<amsrust::ga::GateArrayType>,
    /// Level a black ink is drawn at, above the blanking and the sync
    /// pulses (0, blacker than black); not with the debug server.
    pub black_level: u8
}

/// `--every`: seconds apart, or frames apart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Every {
    Seconds(f64),
    Frames(u64)
}

pub fn parse_args() -> Args {
    let mut args = Args {
        crtc: CrtcType::default(),
        model: CpcModel::default(),
        ram_kb: None,
        disc_interface: None,
        french: false,
        playcity: false,
        digiblaster: false,
        dsk_a: None,
        dsk_b: None,
        cdt: None,
        sna: None,
        screenshot_dir: None,
        seconds: 30,
        every: Every::Seconds(2.0),
        from: 0.0,
        window: false,
        mute: false,
        turbo: false,
        watch: false,
        overlay: false,
        record: None,
        record_ym: None,
        record_wav: None,
        symbols: Vec::new(),
        roms: Vec::new(),
        m4: None,
        m4_rom: None,
        albireo: None,
        m4_slot: None,
        nova: None,
        booster: None,
        booster_uart: None,
        booster_eeprom: None,
        booster_flash: None,
        booster_adc: None,
        save_sna: None,
        debug_server: false,
        debug_port: amsrust::debug_server::DEFAULT_PORT,
        debug_paused: false,
        text: None,
        type_at: 3.0,
        display: DisplayMode::default(),
        gpu: false,
        curvature: 0.0,
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
            "--playcity" => args.playcity = true,
            "--digiblaster" => args.digiblaster = true,
            "--dsk" => args.dsk_a = Some(next("--dsk")),
            "--dsk-b" => args.dsk_b = Some(next("--dsk-b")),
            "--cdt" => args.cdt = Some(next("--cdt")),
            "--sna" => args.sna = Some(next("--sna")),
            "--screenshot" => args.screenshot_dir = Some(next("--screenshot")),
            "--seconds" => args.seconds = next("--seconds").parse().expect("--seconds must be a number"),
            "--every" => {
                let value = next("--every");
                args.every = match value.strip_suffix('f') {
                    Some(frames) => Every::Frames(
                        frames.parse().ok().filter(|n| *n > 0).expect("--every Nf: N must be a whole number of frames, from 1")
                    ),
                    None => Every::Seconds(
                        value.parse().ok().filter(|s: &f64| *s > 0.0).expect("--every must be seconds, or frames as Nf")
                    )
                }
            },
            "--from" => args.from = next("--from").parse().ok().filter(|s: &f64| *s >= 0.0).expect("--from must be seconds"),
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
            "--record-ym" => args.record_ym = Some(next("--record-ym")),
            "--record-wav" => args.record_wav = Some(next("--record-wav")),
            "--sym" => args.symbols.push(next("--sym")),
            "--m4" => args.m4 = Some(next("--m4")),
            "--m4-rom" => args.m4_rom = Some(next("--m4-rom")),
            "--albireo" => args.albireo = Some(next("--albireo")),
            "--nova" => args.nova = Some(next("--nova")),
            "--booster" => args.booster = Some(amsrust::booster::Model::BoosterPlus),
            "--miniboster" => args.booster = Some(amsrust::booster::Model::MiniBooster),
            "--booster-uart" => args.booster_uart = Some(next("--booster-uart")),
            "--booster-eeprom" => args.booster_eeprom = Some(next("--booster-eeprom")),
            "--booster-flash" => args.booster_flash = Some(next("--booster-flash")),
            "--booster-adc" => args.booster_adc = Some(next("--booster-adc")),
            "--m4-slot" => {
                args.m4_slot = Some(next("--m4-slot").parse().unwrap_or_else(|_| panic!("--m4-slot: a slot, 0-255")))
            },
            "--rom" => {
                let value = next("--rom");
                let rom = value
                    .split_once('=')
                    .and_then(|(slot, path)| Some((slot.trim().parse::<u8>().ok()?, path.to_owned())))
                    .unwrap_or_else(|| panic!("--rom {value}: expected SLOT=FILE (slot 0-255)"));
                args.roms.push(rom);
            },
            "--watch" => args.watch = true,
            "--save-sna" => args.save_sna = Some(next("--save-sna")),
            "--overlay" => args.overlay = true,
            "--curvature" => {
                args.curvature = next("--curvature")
                    .parse()
                    .ok()
                    .filter(|c: &f32| (0.0..=4.0).contains(c))
                    .unwrap_or_else(|| panic!("--curvature must be a number from 0 (flat) to 4"))
            },
            "--display" => {
                args.display = next("--display").parse().unwrap_or_else(|e: String| panic!("--display: {e}"))
            },
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            },
            "--list-roms" => {
                print!("{}", amsrust::roms::listing());
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
        "amsrust [--model 464|664|6128] [--french] [--playcity] [--digiblaster] [--ram KB] [--ddi|--no-disc] [--crtc 0-4]\n",
        "        [--dsk path] [--dsk-b path] [--cdt path] [--sna path] [--save-sna path] [--watch]\n",
        "        [--screenshot dir [--seconds N] [--every N|Nf] [--from S]] [--window [--mute] [--turbo]]\n",
        "        [--record file [--seconds N]] [--record-ym file] [--record-wav file] [--type text [--type-at N]]\n",
        "        [--debug-server [--debug-port N] [--debug-paused]] [--sym file] [--rom slot=file]\n",
        "        [--m4 dir [--m4-rom file] [--m4-slot N]] [--albireo dir|image] [--nova file] [--list-roms]\n",
        "        [--booster|--miniboster [--booster-uart loopback|listen:addr|connect:addr] [--booster-eeprom file]\n",
        "         [--booster-flash file] [--booster-adc file.wav]]\n",
        "        [--display raw|ctm|trinitron|gt65|atari [--crt-gpu] [--curvature F]]\n",
        "        [--black-level 0-255] [--frontend minifb|winit] [--ga 40007|40008|40010]\n",
        "\n",
        "--model picks the ROMs (default 6128); --french the French ones of the 6128,\n",
        "with its AZERTY keyboard (typed text and keys follow it). The 464 and 664 have 64K and the\n",
        "6128 128K: --ram 128 (up to 576) adds the extension banked as a 6128's.\n",
        "The 664 and 6128 have a disc interface; --ddi adds a DDI-1 (AMSDOS) to\n",
        "a 464, --no-disc removes it. --playcity plugs a PlayCity on the expansion port (two\n",
        "more AYs, right and left, and a CTC whose channel 1 counts the CRTC's CURSOR to an\n",
        "NMI), --digiblaster\n",
        "a DAC on the printer port (a signed byte written to &EFxx).\n",
        "--rom 6=file.rom fits a 16K upper ROM in a slot (0-255; repeatable). A ROM is a file or\n",
        "inner://NAME, one embedded in the executable (--list-roms lists them): UniDOS as its manual\n",
        "advises is --rom 1=inner://unidos-nova --rom 7=inner://unidos --rom 14=inner://unitools\n",
        "--rom 15=inner://amsdos --rom 27=inner://unidos-albireo (with --nova and --albireo).\n",
        "Orgams: --rom 10=inner://orgams --rom 11=inner://orgext --rom 12=inner://monogams\n",
        "--rom 13=inner://bricbrac --rom 14=inner://orgass (|ORGAMS; --ram 576 gives it its banks\n",
        "from &FF down, a 256K RAM disc fitting).\n",
        "--m4 DIR plugs an M4 board, the folder its SD card (its ROM v2.0.8 in slot 6; --m4-rom\n",
        "another one, --m4-slot another slot): RUN\"file, CAT, |DIR, |CD... on the folder.\n",
        "--albireo DIR|image.img plugs an Albireo (a CH376 at &FE80), its USB key a folder or a\n",
        "FAT image (whole, or partitioned). --nova nvram.bin plugs a Nova (32K NVRAM, a clock;\n",
        "the NVRAM kept in the file). Their ROMs (UniDOS and its nodes) go in with --rom, e.g.\n",
        "--rom 1=nova.rom --rom 7=unidos.rom --rom 15=amsdos.rom --rom 27=albireo.rom.\n",
        "--booster plugs a CPC Booster+ (--miniboster its variant: &FF01 reads 238, not 85) at &FF00-&FF27:\n",
        "a UART (--booster-uart loopback, or listen:127.0.0.1:2323 / connect:host:port: a TCP socket as\n",
        "a null modem), two PWM channels (the card's stereo sound), a 2 channels ADC (--booster-adc: a\n",
        "WAV or raw bytes, looped), 512 bytes of EEPROM and 16K of program memory (kept in the files\n",
        "--booster-eeprom/--booster-flash), a 5 bits TTL port, a multiplier, a keyboard decoder.\n",
        "--cdt puts a tape (CDT/TZX) in the cassette\n",
        "deck, PLAY down: it plays when the CPC starts the motor (RUN\" on a 464,\n",
        "|TAPE first on a 664/6128).\n",
        "With no --screenshot/--window, drops into the interactive monitor once\n",
        "everything is loaded ('h' at its prompt lists its commands). With\n",
        "--screenshot, runs headless for --seconds (default 30), saving a\n",
        "screenshot every --every seconds (default 2), or every N frames with --every Nf\n",
        "(1f: each frame, for following an effect frame by frame), from --from seconds\n",
        "(default 0). With --window, opens a\n",
        "live, playable window redrawn at the CPC's own ~50Hz, keyboard included\n",
        "- close the window to quit. Built with --features audio, the window\n",
        "plays the sound too (--mute to silence it). F10 toggles full speed: the\n",
        "emulation runs as fast as it can, silent, the picture shown 50 times a\n",
        "second (--turbo starts so). F7 saves the machine's state in a snapshot,\n",
        "bemu_0000.sna (then 0001...) in the current directory. --save-sna saves it\n",
        "when the run ends (headless, recording, window closed). --record capture.mp4 records\n",
        "a video with sound through ffmpeg: what the window shows until it is\n",
        "closed, or without --window --seconds (default 30) run headless.\n",
        "--record-ym file.ym saves the AY's registers once a frame (on the VSYNC) as a YM6\n",
        "file, --record-wav file.wav the sound, until the run ends (headless, recording,\n",
        "window closed; the sound of full speed is not kept).\n",
        "--type 'run\"demo\\n' types on the CPC keyboard after --type-at seconds\n",
        "(default 3). --watch takes the --sna/--dsk/--dsk-b/--cdt files again when they change\n",
        "(with --window or --debug-server), for an edit, assemble, see loop: a disc or tape goes\n",
        "in again, then the snapshot is loaded again; without a snapshot the machine is switched\n",
        "off and on and --type typed again. Breakpoints of a snapshot's REMU/BRKC/BRKS/WABP chunks are set\n",
        "in the interactive monitor. Its symbols (SYMB chunk, REMU labels) name the addresses in\n",
        "the monitor and the debug server, with those of --sym files (repeatable: name equ #4000,\n",
        "name: equ 0x4000, name = $4000, name #4000, 4000 name).\n",
        "--display picks how the picture is rendered, in the window, the\n",
        "recording and the screenshots (raw: the monitor's ideal picture; ctm,\n",
        "trinitron, gt65, atari: CRT approximations). F9 cycles them in the\n",
        "window (not while recording). --crt-gpu renders them on the GPU (a build\n",
        "with --features crt-gpu). The CRTs can be seen through their curved glass:\n",
        "--curvature scales its bulge (default 0: flat; 1 is the tube's own bulge).\n",
        "--black-level (default 8) is what a black ink is\n",
        "drawn at: on a real tube it is a dark grey, blanking and sync being\n",
        "blacker (0); without the debug server, whose pictures stay exact.\n",
        "--overlay draws where the CPU wrote to the CRTC and the Gate Array, on the\n",
        "screenshots and in the window (F6 shows or hides it there): a short bar where\n",
        "the beam was, CRTC R0-R9 cyan, R12/R13 orange, other registers white, register\n",
        "selects grey, inks yellow (pen selects dark yellow), mode/ROMs magenta, RAM green.\n",
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
