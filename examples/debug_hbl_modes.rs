//! Boots HBL, runs to the moving-letters scene, then logs - for a couple of
//! frames - every change of CRTC R0-R9 / GA video mode (next & active), tagged
//! with a tick-ish step counter, to see what the demo does per raster line.
//!
//! `cargo run --release --example debug_hbl_modes -- <HBL.DSK> <wait_seconds> <instructions>`
//! or with a CSL script instead of a disc: `... -- --csl <script.csl> 0 <instructions>`

use amsrust::chip::Chip;
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let first = a.next().expect("usage: <HBL.DSK> <wait_s> <instructions>");
    let csl = if first == "--csl" {
        Some(a.next().expect("script"))
    } else {
        None
    };
    let dsk = first;
    let wait: u64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(40);
    let count: u64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(4000);
    let mut runner = CslRunner::new("/tmp/debug_hbl_modes_shots");
    let script = format!(
        "csl_version 1.0\ncrtc_select 0\nreset\ndisk_insert '{dsk}'\nwait 3000000\n\
         key_delay 70000 70000 400000\nkey_output 'RUN\"HBL\" \\(')CRTC=0\\(RET)'\nwait {}\n",
        wait * 1_000_000
    );
    if let Some(path) = &csl {
        let text = std::fs::read_to_string(path).unwrap();
        let base = std::path::Path::new(path).parent().unwrap().to_path_buf();
        runner.run_text(&text, &base).expect("csl");
    } else {
        runner.run_text(&script, ".").expect("boot");
    }
    let mb = runner.motherboard_mut();
    let regs = |mb: &amsrust::motherboard::Motherboard| {
        let c = mb.crtc();
        [
            c.r0(),
            c.r1(),
            c.r2(),
            c.r3(),
            c.r4(),
            c.r5(),
            c.r6(),
            c.r7(),
            c.r9(),
        ]
    };
    if csl.is_some() {
        use amsrust::z80::{HasValue, Reg16};
        let sp = mb.cpu().reg16(Reg16::Sp).value();
        println!(
            "REGS DE={:04X} HL={:04X} BC={:04X} SP={:04X} stack={:02X?}",
            mb.cpu().reg16(Reg16::De).value(),
            mb.cpu().reg16(Reg16::Hl).value(),
            mb.cpu().reg16(Reg16::Bc).value(),
            sp,
            (0..8)
                .map(|i| mb.read_byte(sp.wrapping_add(i)))
                .collect::<Vec<_>>()
        );
        for i in mb.disassemble_instructions_from(0x0, 0) {
            let t = format!("{:?}", i.token);
            if true {
                println!("DIS {:04X} {}", i.address, t);
            }
        }
    }
    let mut last = (
        regs(mb),
        mb.ga().next_video_mode(),
        mb.ga().active_video_mode(),
        mb.ga().hsync_ends(),
    );
    println!(
        "start PC={:04X} regs R0-R7,R9={:?} next={:?} active={:?}",
        mb.pc(),
        last.0,
        last.1,
        last.2
    );
    let mut hist: std::collections::BTreeMap<u16, u32> = Default::default();
    let mut last_visit: Option<u64> = None;
    let mut vs_prev = false;
    let mut seq: Vec<(u16, u64)> = Vec::new();
    let mut call_t: Option<(u64, u16)> = None;
    let mut delays: Vec<(u16, u64)> = Vec::new();
    let mut ppi_prev = false;
    let mut ppi_rise: u64 = 0;
    let mut widths: Vec<(u64, u64)> = Vec::new();
    let mut crtc_fall_w: u64 = 0;
    let mut vs_rise: u64 = 0;
    let mut phases: Vec<(u64, bool)> = Vec::new();
    let mut periods: Vec<u64> = Vec::new();
    for i in 0..count {
        let pc = mb.pc();
        *hist.entry(pc & 0xfff0).or_default() += 1;
        if csl.is_some() && (0xA4A9..=0xA4B8).contains(&pc) && seq.len() < 40 {
            seq.push((pc, mb.nops_counter().value() as u64));
        }
        if csl.is_some() {
            use amsrust::z80::{HasValue, Reg16};
            if pc == 0xA4A6 || pc == 0xA4B3 {
                call_t = Some((
                    mb.nops_counter().value() as u64,
                    mb.cpu().reg16(Reg16::De).value(),
                ));
            }
            if (pc == 0xA4A9 || pc == 0xA4B6) && call_t.is_some() {
                let (t, de) = call_t.take().unwrap();
                delays.push((de, mb.nops_counter().value() as u64 - t));
            }
            let vs = mb.crtc().vsync();
            if vs && !vs_prev {
                vs_rise = mb.nops_counter().value() as u64;
            }
            if !vs && vs_prev {
                crtc_fall_w = mb.nops_counter().value() as u64 - vs_rise;
            }
            vs_prev = vs;
            let pv = mb.ppi().get_pin(amsrust::ppi::PpiPin::B0);
            if pv && !ppi_prev {
                ppi_rise = mb.nops_counter().value() as u64;
            }
            if !pv && ppi_prev {
                widths.push((crtc_fall_w, mb.nops_counter().value() as u64 - ppi_rise));
            }
            ppi_prev = pv;
            if pc == 0xA4AB {
                phases.push((mb.nops_counter().value() as u64 - vs_rise, vs));
            }
        }
        if csl.is_some() && pc == 0xA4A9 {
            let n = mb.nops_counter().value() as u64;
            if let Some(l) = last_visit {
                periods.push(n.wrapping_sub(l));
            }
            last_visit = Some(n);
        }
        mb.execute_next_opcode_no_video();
        let now = (
            regs(mb),
            mb.ga().next_video_mode(),
            mb.ga().active_video_mode(),
            mb.ga().hsync_ends(),
        );
        if (now.0, now.1, now.2) != (last.0, last.1, last.2) {
            println!(
                "{i:6} PC={pc:04X} R2={} R3={} next={:?} active={:?} hsync_ends={}",
                (now.0)[2],
                (now.0)[3],
                now.1,
                now.2,
                now.3
            );
            last = now;
        }
    }
    if csl.is_some() {
        println!(
            "PHASE at IN A,(C) (us since last CRTC vsync rise, crtc vsync now): first 10 {:?}",
            &phases[..phases.len().min(10)]
        );
    }
    if csl.is_some() {
        println!(
            "VSYNC widths us (crtc, ppi B0 pin): {:?}",
            &widths[..widths.len().min(4)]
        );
    }
    if csl.is_some() {
        println!(
            "DELAY calls (DE, us elapsed CALL..after): {:?}",
            &delays[..delays.len().min(6)]
        );
    }
    if csl.is_some() {
        let d: Vec<(String, u64)> = seq
            .windows(2)
            .map(|w| (format!("{:04X}", w[0].0), w[1].1 - w[0].1))
            .collect();
        println!("INSTR durations us: {:?}", d);
    }
    if csl.is_some() {
        println!(
            "PERIODS(first 12 of {}) {:?}",
            periods.len(),
            &periods[..periods.len().min(12)]
        );
    }
    if csl.is_some() {
        println!("PCHIST {:x?}", hist);
    }
}
