//! Finds where SP jumps (|delta| > 32 outside PUSH/POP/CALL/RET noise) in the
//! run-up to the known HBL crash, with PC, opcode bytes and RAM config.
//!
//! `cargo run --release --example debug_hbl_sp_trace -- <HBL.DSK> [crash_step]`

use amsrust::csl_runner::CslRunner;
use amsrust::z80::{HasValue, Reg16};
use std::collections::VecDeque;

fn main() {
    let mut args = std::env::args().skip(1);
    let dsk = args.next().expect("usage: <HBL.DSK> [crash_step]");
    let crash: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(10_992_120);
    let mut runner = CslRunner::new("/tmp/debug_hbl_sp_trace_shots");
    let boot = format!(
        "csl_version 1.0\ncrtc_select 0\nreset\ndisk_insert '{dsk}'\nwait 3000000\n\
         key_delay 70000 70000 400000\nkey_output 'CAT\\(RET)'\nwait 6000000\n\
         key_output 'RUN\"HBL\\(RET)'\nwait 3000000\n"
    );
    runner.run_text(&boot, ".").expect("boot");
    let mb = runner.motherboard_mut();
    let mut last_sp = mb.cpu().reg16(Reg16::Sp).value();
    let mut ring: VecDeque<String> = VecDeque::new();
    let mut jumps: Vec<String> = Vec::new();
    let mut tail: VecDeque<String> = VecDeque::new();
    let mut watch_prev: u16 = 0;
    let mut watch_log: Vec<String> = Vec::new();
    for step in 0..crash + 5 {
        let pc = mb.pc();
        let sp0 = last_sp;
        let bytes: Vec<u8> = (0..4).map(|i| mb.read_byte(pc.wrapping_add(i))).collect();
        mb.execute_next_opcode_no_video();
        let sp = mb.cpu().reg16(Reg16::Sp).value();
        if step + 400 > crash {
            tail.push_back(format!(
                "{step} PC={pc:04X} {bytes:02x?} SP={sp0:04X}->{sp:04X} lowrom={:?} upper={:?} ram={:?}",
                mb.memory_config().0.lower_rom,
                mb.memory_config().0.upper_rom,
                mb.memory_config().1 .0.map(|b| match b.0 { amsrust::memory::PageNumber::Base => 'B', _ => 'X' })
            ));
        }
        if step < crash + 10 {
            let now = u16::from(mb.read_byte(0xb1ee)) | u16::from(mb.read_byte(0xb1ef)) << 8;
            if now != watch_prev {
                watch_log.push(format!(
                    "step {step}: [B1EE] {watch_prev:04X}->{now:04X} by instr at PC={pc:04X} {bytes:02x?} ram={:?}",
                    mb.memory_config().1 .0.map(|b| match b.0 { amsrust::memory::PageNumber::Base => 'B', _ => 'X' })
                ));
                watch_prev = now;
            }
        }
        let delta = sp.wrapping_sub(sp0) as i16;
        if delta.unsigned_abs() > 32 {
            let line = format!(
                "step {step} PC={pc:04X} {bytes:02x?} SP {sp0:04X}->{sp:04X} cfg={:?}",
                mb.memory_config().1 .0.map(|b| (b.0, b.1))
            );
            jumps.push(line.clone());
            ring.push_back(line);
            if ring.len() > 12 {
                ring.pop_front();
            }
        }
        last_sp = sp;
    }
    for l in watch_log.iter().rev().take(25).rev() {
        println!("W {l}");
    }
    for l in &tail {
        println!("T {l}");
    }
    println!("{} SP jumps in total; last 12 before the crash:", jumps.len());
    for l in ring {
        println!("{l}");
    }
}
