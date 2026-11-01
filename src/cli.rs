//! Command line of the `amsrust` binary, independent of the window backend.

use amsrust::crtc::CrtcType;
use amsrust::display::DisplayMode;
use amsrust::motherboard::CpcModel;
use clap::{Parser, ValueEnum};

/// The window backend, `--frontend`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum FrontendKind {
    Minifb,
    /// winit + wgpu: the CRT is drawn by the GPU, with no read-back
    /// (`--features gpu-window`).
    Winit
}

/// ACE-DL's own: ink black is a dark grey, not the 0 of the blanking.
pub const DEFAULT_BLACK_LEVEL: u8 = 8;

const MACHINE: &str = "Machine";
const MEDIA: &str = "Discs, tapes, snapshots";
const RUN: &str = "Running";
const RECORD: &str = "Recording";
const TYPING: &str = "Typing";
const DEBUG: &str = "Debugging";
const DISPLAY: &str = "Display";
const EXPANSIONS: &str = "Expansions";

/// An Amstrad CPC emulator.
#[derive(Parser)]
#[command(name = "amsrust", version, after_long_help = AFTER_HELP)]
pub struct Args {
    /// The model, its ROMs: the 464 and 664 have 64K, the 6128 128K; the 664 and 6128 a disc
    /// interface.
    #[arg(long, default_value = "6128", help_heading = MACHINE)]
    pub model: CpcModel,

    /// The French ROMs of the 6128, with its AZERTY keyboard (typed text and keys follow it).
    #[arg(long, help_heading = MACHINE)]
    pub french: bool,

    /// Total RAM in KB, the extension banked as a 6128's: 64, 128... 576.
    #[arg(long = "ram", value_name = "KB", value_parser = parse_ram, help_heading = MACHINE)]
    pub ram_kb: Option<usize>,

    /// Adds a DDI-1 (AMSDOS) to a 464.
    #[arg(long, conflicts_with = "no_disc", help_heading = MACHINE)]
    ddi: bool,

    /// Removes the disc interface.
    #[arg(long, help_heading = MACHINE)]
    no_disc: bool,

    /// The CRTC type.
    #[arg(long, value_name = "0-4", default_value = "0", value_parser = parse_crtc, help_heading = MACHINE)]
    pub crtc: CrtcType,

    /// The Gate Array (the 40007/8 are one pixel ahead: palette writes).
    #[arg(long = "ga", value_name = "40007|40008|40010", value_parser = parse_gate_array, help_heading = MACHINE)]
    pub gate_array: Option<amsrust::ga::GateArrayType>,

    /// A 16K upper ROM in a slot (0-255; repeatable). FILE is a file or inner://NAME, one
    /// embedded in the executable (--list-roms).
    ///
    /// UniDOS as its manual advises: --rom 1=inner://unidos-nova --rom 7=inner://unidos
    /// --rom 14=inner://unitools --rom 15=inner://amsdos --rom 27=inner://unidos-albireo (with
    /// --nova and --albireo). Orgams: --rom 10=inner://orgams --rom 11=inner://orgext
    /// --rom 12=inner://monogams --rom 13=inner://bricbrac --rom 14=inner://orgass (|ORGAMS;
    /// --ram 576 gives it its banks from &FF down, a 256K RAM disc fitting).
    #[arg(long = "rom", value_name = "SLOT=FILE", value_parser = parse_rom, help_heading = MACHINE)]
    pub roms: Vec<(u8, String)>,

    /// Lists the ROMs embedded in the executable.
    #[arg(long, help_heading = MACHINE)]
    list_roms: bool,

    /// A disc in drive A: a DSK or HFE image, a zip archive (its first DSK/HFE) or
    /// `archive.zip#name.dsk` (that file of the archive). It goes in side A up; F5 flips it in a
    /// 3" drive.
    #[arg(long = "dsk", value_name = "PATH", help_heading = MEDIA)]
    pub dsk_a: Option<String>,

    /// A disc in drive B (as --dsk; Shift+F5 flips it).
    #[arg(long, value_name = "PATH", help_heading = MEDIA)]
    pub dsk_b: Option<String>,

    /// Drive A: 3 (a 3" drive, one head: the side read is the side put up, F5 turns the disc
    /// over) or 3.5 (a double-sided 3.5" drive: the FDC selects the side, 80 tracks).
    #[arg(long, value_name = "3|3.5", default_value = "3", help_heading = MEDIA)]
    pub drive_a: amsrust::fdc::DriveKind,

    /// Drive B: 3 or 3.5 (see --drive-a).
    #[arg(long, value_name = "3|3.5", default_value = "3", help_heading = MEDIA)]
    pub drive_b: amsrust::fdc::DriveKind,

    /// A tape (CDT/TZX) in the cassette deck, PLAY down: it plays when the CPC starts the motor
    /// (RUN" on a 464, |TAPE first on a 664/6128).
    #[arg(long, value_name = "PATH", help_heading = MEDIA)]
    pub cdt: Option<String>,

    /// A snapshot to start from. Breakpoints of its REMU/BRKC/BRKS/WABP chunks are set in the
    /// interactive monitor; its symbols (SYMB chunk, REMU labels) name the addresses in the
    /// monitor and the debug server.
    #[arg(long, value_name = "PATH", help_heading = MEDIA)]
    pub sna: Option<String>,

    /// The machine's state saved there when the run ends (headless, recording, window closed).
    #[arg(long, value_name = "PATH", help_heading = MEDIA)]
    pub save_sna: Option<String>,

    /// Takes the --sna/--dsk/--dsk-b/--cdt files again when they change (with --window or
    /// --debug-server).
    ///
    /// For an edit, assemble, see loop: a disc or tape goes in again, then the snapshot is
    /// loaded again; without a snapshot the machine is switched off and on and --type typed
    /// again.
    #[arg(long, help_heading = MEDIA)]
    pub watch: bool,

    /// Runs headless for --seconds, saving screenshots in DIR.
    #[arg(long = "screenshot", value_name = "DIR", help_heading = RUN)]
    pub screenshot_dir: Option<String>,

    /// How long a headless run (--screenshot, --record) lasts.
    #[arg(long, default_value_t = 30, help_heading = RUN)]
    pub seconds: u64,

    /// How often a screenshot is taken: seconds, or frames as Nf (1f: each frame, for following
    /// an effect frame by frame).
    #[arg(long, value_name = "N|Nf", default_value = "2", value_parser = parse_every, help_heading = RUN)]
    pub every: Every,

    /// The first screenshot at these seconds.
    #[arg(long, value_name = "S", default_value_t = 0.0, value_parser = parse_seconds, help_heading = RUN)]
    pub from: f64,

    /// A live, playable window redrawn at the CPC's ~50 Hz, keyboard included; close it to quit.
    #[arg(long, help_heading = RUN)]
    pub window: bool,

    /// The window silent (a build with --features audio plays the sound).
    #[arg(long, help_heading = RUN)]
    pub mute: bool,

    /// The window starts at full speed (F10 toggles it).
    #[arg(long, help_heading = RUN)]
    pub turbo: bool,

    /// The window's backend: winit is a window the GPU draws, the CRT included, with nothing read
    /// back (a build with --features gpu-window); F11 toggles its fullscreen.
    #[arg(long, value_enum, default_value_t = FrontendKind::Minifb, help_heading = RUN)]
    pub frontend: FrontendKind,

    /// A video with sound through ffmpeg: what the window shows until it is closed, or without
    /// --window, --seconds run headless.
    #[arg(long, value_name = "FILE", help_heading = RECORD)]
    pub record: Option<String>,

    /// The AY's registers once a frame (on the VSYNC), as a YM6 file, until the run ends.
    #[arg(long, value_name = "FILE", help_heading = RECORD)]
    pub record_ym: Option<String>,

    /// The sound as a WAV file, until the run ends (the sound of full speed is not kept).
    #[arg(long, value_name = "FILE", help_heading = RECORD)]
    pub record_wav: Option<String>,

    /// Typed on the CPC keyboard at --type-at, the keys written as in CSL's key_output: \n,
    /// \(RET), \(ESC), \(CPY), \(DEL)... and \(WAIT n), n seconds (a program's prompt) before
    /// the keys after it.
    #[arg(long = "type", value_name = "TEXT", value_parser = parse_typed, help_heading = TYPING)]
    pub text: Option<String>,

    /// A BASIC listing (text) put in memory at --type-at, then RUN typed (and --type).
    #[arg(long, value_name = "FILE.bas", help_heading = TYPING)]
    pub basic: Option<String>,

    /// When --type and --basic start, in seconds.
    #[arg(long, value_name = "S", default_value_t = 3.0, help_heading = TYPING)]
    pub type_at: f64,

    /// Serves the AMSpiriT-Lite compatible debug API on 127.0.0.1 (GET /api/doc lists the
    /// endpoints), with --window or headless.
    #[arg(long, help_heading = DEBUG)]
    debug_server: bool,

    /// The debug server's port (--debug-server implied).
    #[arg(long, value_name = "PORT", help_heading = DEBUG)]
    debug_port: Option<u16>,

    /// The debug server starts paused, for a debugger to set its breakpoints before anything
    /// runs (--debug-server implied).
    #[arg(long, help_heading = DEBUG)]
    pub debug_paused: bool,

    /// A symbol file for the monitor and the debug server (repeatable): name equ #4000,
    /// name: equ 0x4000, name = $4000, name #4000, 4000 name.
    #[arg(long = "sym", value_name = "FILE", help_heading = DEBUG)]
    pub symbols: Vec<String>,

    /// How the picture is rendered, in the window, the recording and the screenshots (raw: the
    /// monitor's ideal picture; the others CRT approximations). F9 cycles them in the window.
    #[arg(long, value_name = DisplayMode::NAMES, default_value = "raw", help_heading = DISPLAY)]
    pub display: DisplayMode,

    /// Renders the CRTs on the GPU (a build with --features crt-gpu).
    #[arg(long = "crt-gpu", help_heading = DISPLAY)]
    pub gpu: bool,

    /// How curved a CRT's glass is, times its tube's own bulge: 0 (flat) to 4.
    #[arg(long, default_value_t = 0.0, value_parser = parse_curvature, help_heading = DISPLAY)]
    pub curvature: f32,

    /// What a black ink is drawn at: on a real tube a dark grey, blanking and sync being blacker
    /// (0). Not with the debug server, whose pictures stay exact.
    #[arg(long, value_name = "0-255", default_value_t = DEFAULT_BLACK_LEVEL, help_heading = DISPLAY)]
    pub black_level: u8,

    /// Draws where the CPU wrote to the CRTC and the Gate Array, on the screenshots and in the
    /// window (F6 shows or hides it there).
    ///
    /// A short bar where the beam was: CRTC R0-R9 cyan, R12/R13 orange, other registers white,
    /// register selects grey, inks yellow (pen selects dark yellow), mode/ROMs magenta, RAM green.
    #[arg(long, help_heading = DISPLAY)]
    pub overlay: bool,

    /// A PlayCity on the expansion port: two more AYs, right and left, and a CTC whose channel 1
    /// counts the CRTC's CURSOR to an NMI.
    #[arg(long, help_heading = EXPANSIONS)]
    pub playcity: bool,

    /// A DAC on the printer port (a signed byte written to &EFxx).
    #[arg(long, help_heading = EXPANSIONS)]
    pub digiblaster: bool,

    /// An M4 board, the folder its SD card: RUN"file, CAT, |DIR, |CD... on the folder.
    ///
    /// Its WiFi is the host's network (sockets, |HTTPGET, |HTTPMEM, |NETSTAT, |TIME with
    /// m4/config.txt's tz; nothing is joined, |NETSET only writes m4/config.txt). RulezCharge
    /// downloads from cpcrulez.fr: --m4 DIR --rom 5=inner://rulezcharge, then |RZ
    /// (examples/rulezcharge.sh).
    #[arg(long, value_name = "DIR", help_heading = EXPANSIONS)]
    pub m4: Option<String>,

    /// Another ROM for the M4 than its bundled v2.0.8.
    #[arg(long, value_name = "FILE", help_heading = EXPANSIONS)]
    pub m4_rom: Option<String>,

    /// The slot of the M4's ROM (6 by default).
    #[arg(long, value_name = "SLOT", help_heading = EXPANSIONS)]
    pub m4_slot: Option<u8>,

    /// The M4's WiFi off (|WIFI,0 too).
    #[arg(long, help_heading = EXPANSIONS)]
    pub m4_no_wifi: bool,

    /// An Albireo (a CH376 at &FE80), its USB key a folder or a FAT image (whole, or
    /// partitioned).
    #[arg(long, value_name = "DIR|IMAGE", help_heading = EXPANSIONS)]
    pub albireo: Option<String>,

    /// A Nova (32K NVRAM, a clock), the NVRAM kept in the file.
    #[arg(long, value_name = "FILE", help_heading = EXPANSIONS)]
    pub nova: Option<String>,

    /// A CPC Booster+ at &FF00-&FF27: a UART, two PWM channels (stereo sound), a 2 channels ADC,
    /// 512 bytes of EEPROM, 16K of program memory, a 5 bits TTL port, a multiplier, a keyboard
    /// decoder.
    #[arg(long, conflicts_with = "miniboster", help_heading = EXPANSIONS)]
    booster: bool,

    /// The MiniBooster, the Booster+'s variant (&FF01 reads 238, not 85).
    #[arg(long, help_heading = EXPANSIONS)]
    miniboster: bool,

    /// What the Booster's UART is wired to: loopback, or a TCP socket as a null modem
    /// (listen:127.0.0.1:2323, connect:host:port).
    #[arg(long, value_name = "loopback|listen:ADDR|connect:ADDR", help_heading = EXPANSIONS)]
    pub booster_uart: Option<String>,

    /// The Booster's 512 bytes of EEPROM, kept in the file.
    #[arg(long, value_name = "FILE", help_heading = EXPANSIONS)]
    pub booster_eeprom: Option<String>,

    /// The Booster's 16K of program memory (a dump of the BIOS), kept in the file.
    #[arg(long, value_name = "FILE", help_heading = EXPANSIONS)]
    pub booster_flash: Option<String>,

    /// What the Booster's ADC samples (a WAV or raw unsigned bytes), looped.
    #[arg(long, value_name = "FILE", help_heading = EXPANSIONS)]
    pub booster_adc: Option<String>,

    /// Willy with an OPL3LPT (an OPL3: &FEBC/&FEBE select a register of bank 0/1, &FEBD/&FEBF
    /// write it).
    #[arg(long, help_heading = EXPANSIONS)]
    willy_opl3: bool,

    /// The OPL3LPT at another base, Willy's jumper: &FEBC, &FEAC, &FEA4, &FE9C, &FE94, &FE8C,
    /// &FE84 (--willy-opl3 implied).
    #[arg(long, value_name = "ADDR", help_heading = EXPANSIONS)]
    willy_opl3_at: Option<String>,

    /// Willy with an S2P (a General MIDI synth: &FEAC the MIDI data, &FEAD the control/status),
    /// its instruments the SoundFont's.
    #[arg(long, value_name = "FILE.sf2", help_heading = EXPANSIONS)]
    pub willy_s2p: Option<String>,

    /// The S2P at another base, as --willy-opl3-at.
    #[arg(long, value_name = "ADDR", requires = "willy_s2p", help_heading = EXPANSIONS)]
    pub willy_s2p_at: Option<String>,

    /// A CPC Dandanator Mini (512K of flash: its romset boots in place of the firmware). Its USB
    /// serial port is not emulated.
    #[arg(long, value_name = "ROMSET", help_heading = EXPANSIONS)]
    pub dandanator: Option<String>,

    /// What the CPC programs into the Dandanator's flash goes back to the file.
    #[arg(long, requires = "dandanator", help_heading = EXPANSIONS)]
    pub dandanator_writable: bool
}

const AFTER_HELP: &str = "\
With no --screenshot, --window, --record or --debug-server, drops into the interactive monitor
once everything is loaded ('h' at its prompt lists its commands).

In the window: F10 toggles full speed (the emulation as fast as it can, silent, the picture shown
50 times a second), F8 pauses and resumes (through the debug server when it runs, its clients
told), F7 saves the machine's state in bemu_0000.sna (then 0001...) in the current directory,
F9 cycles the displays (not while recording), F6 shows or hides the overlay, F5 turns the disc
of drive A over (Shift+F5: drive B; 3\" drives), F4 closes it.";

/// `--every`: seconds apart, or frames apart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Every {
    Seconds(f64),
    Frames(u64)
}

impl Args {
    /// The disc interface (a DDI-1 on a 464), if not the model's.
    pub fn disc_interface(&self) -> Option<bool> {
        (self.ddi || self.no_disc).then_some(self.ddi)
    }

    /// `--booster` or `--miniboster`.
    pub fn booster(&self) -> Option<&'static str> {
        if self.booster {
            Some("booster")
        }
        else if self.miniboster {
            Some("miniboster")
        }
        else {
            None
        }
    }

    /// The OPL3LPT's base, if Willy has one.
    pub fn willy_opl3(&self) -> Option<String> {
        match &self.willy_opl3_at {
            Some(base) => Some(base.clone()),
            None => self.willy_opl3.then(|| "&FEBC".to_owned())
        }
    }

    pub fn debug_server(&self) -> bool {
        self.debug_server || self.debug_paused || self.debug_port.is_some()
    }

    pub fn debug_port(&self) -> u16 {
        self.debug_port.unwrap_or(amsrust::debug_server::DEFAULT_PORT)
    }
}

pub fn parse_args() -> Args {
    let args = Args::parse();
    if args.list_roms {
        print!("{}", amsrust::roms::listing());
        std::process::exit(0);
    }
    args
}

fn parse_ram(value: &str) -> Result<usize, String> {
    let kb: usize = value.parse().map_err(|_| "a size in KB".to_owned())?;
    if kb >= 64 && kb.is_multiple_of(64) && kb <= 576 {
        Ok(kb)
    }
    else {
        Err("64, 128, ... 576 (KB)".to_owned())
    }
}

fn parse_crtc(value: &str) -> Result<CrtcType, String> {
    value.parse::<u8>().map_err(|_| "0-4".to_owned()).and_then(CrtcType::try_from)
}

fn parse_gate_array(value: &str) -> Result<amsrust::ga::GateArrayType, String> {
    match value {
        "40010" => Ok(amsrust::ga::GateArrayType::Ga40010),
        "40007" | "40008" => Ok(amsrust::ga::GateArrayType::Ga40007_8),
        _ => Err("40007, 40008 or 40010".to_owned())
    }
}

fn parse_rom(value: &str) -> Result<(u8, String), String> {
    value
        .split_once('=')
        .and_then(|(slot, path)| Some((slot.trim().parse::<u8>().ok()?, path.to_owned())))
        .ok_or_else(|| "SLOT=FILE (slot 0-255)".to_owned())
}

fn parse_every(value: &str) -> Result<Every, String> {
    match value.strip_suffix('f') {
        Some(frames) => frames
            .parse()
            .ok()
            .filter(|n| *n > 0)
            .map(Every::Frames)
            .ok_or_else(|| "Nf: N a whole number of frames, from 1".to_owned()),
        None => parse_seconds(value).ok().filter(|s| *s > 0.0).map(Every::Seconds).ok_or_else(|| "seconds, or frames as Nf".to_owned())
    }
}

fn parse_seconds(value: &str) -> Result<f64, String> {
    value.parse().ok().filter(|s: &f64| *s >= 0.0).ok_or_else(|| "seconds".to_owned())
}

fn parse_curvature(value: &str) -> Result<f32, String> {
    value
        .parse()
        .ok()
        .filter(|c: &f32| (0.0..=4.0).contains(c))
        .ok_or_else(|| "a number from 0 (flat) to 4".to_owned())
}

/// `--type`'s text, `\n` a newline.
fn parse_typed(value: &str) -> Result<String, String> {
    Ok(value.replace("\\n", "\n"))
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    use super::{Args, Every};

    #[test]
    fn the_command_line_is_consistent() {
        Args::command().debug_assert();
    }

    #[test]
    fn options_parse_as_before() {
        let args = Args::try_parse_from(["amsrust", "--every", "3f", "--rom", "7=inner://unidos", "--no-disc", "--debug-port", "7000", "--type", "run\"a\\n", "--willy-opl3"]).unwrap();
        assert_eq!(args.every, Every::Frames(3));
        assert_eq!(args.roms, [(7, "inner://unidos".to_owned())]);
        assert_eq!(args.disc_interface(), Some(false));
        assert!(args.debug_server());
        assert_eq!(args.debug_port(), 7000);
        assert_eq!(args.text.as_deref(), Some("run\"a\n"));
        assert_eq!(args.willy_opl3().as_deref(), Some("&FEBC"));
        assert!(Args::try_parse_from(["amsrust", "--ddi", "--no-disc"]).is_err());
        assert!(Args::try_parse_from(["amsrust", "--ram", "100"]).is_err());
    }
}
