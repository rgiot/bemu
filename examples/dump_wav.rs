//! Record what the PSG plays to a 16-bit PCM .wav file.
//!
//! ```text
//! cargo run --release --example dump_wav -- [options] <out.wav>
//!   --csl <script.csl>   play a CSL script; its whole run is recorded
//!   --dsk <path>         insert a disc in drive A
//!   --sna <path>         load a snapshot
//!   --type <text>        type <text> once booted ("\n" types RETURN),
//!                        e.g. --type 'run"disc\n'
//!   --type-at <N>        emulated second at which to type (default 3)
//!   --seconds <N>        emulated seconds to record (default 10)
//!   --rate <Hz>          output sample rate (default 44100)
//!   --stereo             CPC stereo socket wiring (A left, B centre, C
//!                        right) instead of the internal speaker's mono
//! ```

use amsrust::csl_runner::{keys_of_char, CslRunner};
use amsrust::disc_image::load_dsk;
use amsrust::fdc::FdcDrive;
use amsrust::keyboard::CpcKey;
use amsrust::motherboard::{Motherboard, MotherboardBuilder};
use amsrust::psg::AudioConfig;
use std::io::Write;

/// T-states per second at the CPC's 4MHz clock.
const T_STATES_PER_SECOND: usize = 4_000_000;
/// How long each typed key is held, then released (40ms: two frames).
const KEY_T_STATES: usize = T_STATES_PER_SECOND / 25;

struct Args {
    csl: Option<String>,
    dsk: Option<String>,
    sna: Option<String>,
    text: Option<String>,
    type_at: f64,
    seconds: f64,
    rate: u32,
    stereo: bool,
    output: String
}

fn usage() -> ! {
    eprintln!(
        "usage: dump_wav [--csl script.csl | [--dsk path] [--sna path] [--type text] [--type-at N] \
         [--seconds N]] [--rate Hz] [--stereo] <out.wav>"
    );
    std::process::exit(2);
}

fn parse_args() -> Args {
    let mut args = Args {
        csl: None,
        dsk: None,
        sna: None,
        text: None,
        type_at: 3.0,
        seconds: 10.0,
        rate: 44_100,
        stereo: false,
        output: String::new()
    };
    let mut output = None;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| usage());
        match arg.as_str() {
            "--csl" => args.csl = Some(value()),
            "--dsk" => args.dsk = Some(value()),
            "--sna" => args.sna = Some(value()),
            "--type" => args.text = Some(value().replace("\\n", "\n")),
            "--type-at" => args.type_at = value().parse().unwrap_or_else(|_| usage()),
            "--seconds" => args.seconds = value().parse().unwrap_or_else(|_| usage()),
            "--rate" => args.rate = value().parse().unwrap_or_else(|_| usage()),
            "--stereo" => args.stereo = true,
            "-h" | "--help" => usage(),
            _ if output.is_none() && !arg.starts_with("--") => output = Some(arg),
            _ => usage()
        }
    }
    args.output = output.unwrap_or_else(|| usage());
    args
}

fn main() {
    let args = parse_args();
    let config = if args.stereo { AudioConfig::stereo(args.rate) } else { AudioConfig::mono(args.rate) };

    let samples = match &args.csl {
        Some(script) => record_csl(script, config),
        None => record_machine(&args, config)
    };

    write_wav(&args.output, config, &samples).unwrap_or_else(|e| panic!("{}: {e}", args.output));
    let frames = samples.len() / config.nb_channels();
    eprintln!(
        "{}: {frames} frames, {:.2}s at {}Hz {}",
        args.output,
        frames as f64 / config.sample_rate as f64,
        config.sample_rate,
        if config.stereo { "stereo" } else { "mono" }
    );
}

fn record_csl(script: &str, config: AudioConfig) -> Vec<i16> {
    let screenshots = std::env::temp_dir().join("amsrust_dump_wav");
    let mut runner = CslRunner::new(screenshots);
    runner.motherboard_mut().set_audio_output(Some(config));
    if let Err(error) = runner.run_file(script) {
        eprintln!("{error}");
        std::process::exit(1);
    }
    to_pcm(&runner.motherboard_mut().take_audio_samples())
}

fn record_machine(args: &Args, config: AudioConfig) -> Vec<i16> {
    let mut motherboard = MotherboardBuilder::default().enable_interruptions().build();
    if let Some(path) = &args.dsk {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let image = load_dsk(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
        motherboard.fdc_mut().insert_image(FdcDrive::A, Some(image));
    }
    if let Some(path) = &args.sna {
        amsrust::snapshot::load_sna(&mut motherboard, path).unwrap_or_else(|e| panic!("{e}"));
    }
    motherboard.set_audio_output(Some(config));

    let start = motherboard.t_state_counter().value();
    let at = |seconds: f64| start + (seconds * T_STATES_PER_SECOND as f64) as usize;
    let end = at(args.seconds);
    let mut pcm = Vec::new();

    if let Some(text) = &args.text {
        run_until(&mut motherboard, at(args.type_at).min(end), &mut pcm);
        for c in text.chars() {
            let Some(keys) = keys_of_char(c)
            else {
                eprintln!("cannot type {c:?}, skipped");
                continue;
            };
            press(&mut motherboard, &keys, true);
            let now = motherboard.t_state_counter().value();
            run_until(&mut motherboard, (now + KEY_T_STATES).min(end), &mut pcm);
            press(&mut motherboard, &keys, false);
            let now = motherboard.t_state_counter().value();
            run_until(&mut motherboard, (now + KEY_T_STATES).min(end), &mut pcm);
        }
    }
    run_until(&mut motherboard, end, &mut pcm);
    pcm
}

fn press(motherboard: &mut Motherboard, keys: &[CpcKey], pressed: bool) {
    for &key in keys {
        motherboard.keyboard_mut().set_pressed(key, pressed);
    }
}

fn run_until(motherboard: &mut Motherboard, t_states: usize, pcm: &mut Vec<i16>) {
    while motherboard.t_state_counter().value() < t_states {
        motherboard.execute_next_opcode_no_video();
        // Drain regularly rather than letting the f32 buffer grow.
        if motherboard.psg().pending_samples() >= 4096 {
            pcm.extend(to_pcm(&motherboard.take_audio_samples()));
        }
    }
    pcm.extend(to_pcm(&motherboard.take_audio_samples()));
}

fn to_pcm(samples: &[f32]) -> Vec<i16> {
    samples
        .iter()
        .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16)
        .collect()
}

/// RIFF/WAVE, PCM 16 bits little-endian, interleaved channels.
fn write_wav(path: &str, config: AudioConfig, samples: &[i16]) -> std::io::Result<()> {
    let channels = config.nb_channels() as u16;
    let data_len = (samples.len() * 2) as u32;
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    out.write_all(b"RIFF")?;
    out.write_all(&(36 + data_len).to_le_bytes())?;
    out.write_all(b"WAVEfmt ")?;
    out.write_all(&16u32.to_le_bytes())?; // fmt chunk size
    out.write_all(&1u16.to_le_bytes())?; // PCM
    out.write_all(&channels.to_le_bytes())?;
    out.write_all(&config.sample_rate.to_le_bytes())?;
    out.write_all(&(config.sample_rate * channels as u32 * 2).to_le_bytes())?; // byte rate
    out.write_all(&(channels * 2).to_le_bytes())?; // block align
    out.write_all(&16u16.to_le_bytes())?; // bits per sample
    out.write_all(b"data")?;
    out.write_all(&data_len.to_le_bytes())?;
    for sample in samples {
        out.write_all(&sample.to_le_bytes())?;
    }
    out.flush()
}
