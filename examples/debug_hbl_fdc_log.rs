//! Logs every FDC command (and its result bytes) issued during the
//! HBL.DSK CAT+RUN"HBL repro, up to the known stack-corruption crash, and
//! flags any whose ST0/ST1/ST2 is abnormal.
//!
//! `cargo run --release --example debug_hbl_fdc_log -- <HBL.DSK path>`

use amsrust::csl_runner::CslRunner;

fn main() {
    let dsk = std::env::args()
        .nth(1)
        .expect("usage: debug_hbl_fdc_log <HBL.DSK path>");
    let mut runner = CslRunner::new("/tmp/debug_hbl_fdc_log_shots");
    runner
        .run_text("csl_version 1.0\ncrtc_select 0\nreset\n", ".")
        .expect("reset");
    runner.motherboard_mut().fdc_mut().start_command_log();
    let boot = format!(
        "csl_version 1.0\ndisk_insert '{dsk}'\nwait 3000000\n\
         key_delay 70000 70000 400000\nkey_output 'CAT\\(RET)'\nwait 6000000\n\
         key_output 'RUN\"HBL\\(RET)'\nwait 3000000\n"
    );
    runner.run_text(&boot, ".").expect("boot");
    let mb = runner.motherboard_mut();
    let mut seen = 0;
    for step in 0..11_000_000u64 {
        mb.execute_next_opcode_no_video();
        if step % 1_000_000 == 0 {
            println!(
                "step {step}: msr={:02x} motor={} PC={:04X}",
                mb.fdc().read_status(),
                mb.fdc().motor(),
                mb.pc()
            );
        }
        let n = mb.fdc().command_log().len();
        if n != seen {
            seen = n;
            if n > 80 {
                println!(
                    "step {step}: command #{} complete, PC={:04X}",
                    n - 1,
                    mb.pc()
                );
            }
        }
    }
    {
        use std::collections::BTreeMap;
        let mut hist: BTreeMap<u16, u32> = BTreeMap::new();
        for _ in 0..200_000 {
            mb.execute_next_opcode_no_video();
            *hist.entry(mb.pc() & 0xff00).or_default() += 1;
        }
        println!("PC page histogram (next 200k steps after crash window): {hist:x?}");
        println!(
            "fdc msr={:02x} motor={}",
            mb.fdc().read_status(),
            mb.fdc().motor()
        );
    }
    let log = mb.fdc().command_log();
    let mut abnormal = 0;
    for (i, rec) in log.iter().enumerate() {
        let bad = matches!(
            rec.command[0] & 0x1f,
            0x06 | 0x0c | 0x05 | 0x09 | 0x02 | 0x0a | 0x0d
        ) && rec.result.first().is_some_and(|st0| st0 & 0xc0 != 0);
        if bad {
            abnormal += 1;
        }
        println!(
            "{i:4} {}cmd {:02x?} -> {:02x?}",
            if bad { "!! " } else { "   " },
            rec.command,
            rec.result
        );
    }
    println!("{} commands, {abnormal} abnormal", log.len());
}
