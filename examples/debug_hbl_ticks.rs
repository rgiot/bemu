//! Tick-level view of one raster line of HBL's moving-letters scene: CRTC
//! HSYNC edges, GA HSYNC-end events and mode (RMR next / active) changes.
//!
//! `cargo run --release --example debug_hbl_ticks -- <HBL.DSK> <wait_s> <ticks>`

use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let dsk = a.next().expect("usage: <HBL.DSK> <wait_s> <ticks>");
    let wait: u64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(90);
    let ticks: u64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(1100);
    let mut runner = CslRunner::new("/tmp/debug_hbl_ticks_shots");
    let script = format!(
        "csl_version 1.0\ncrtc_select 0\nreset\ndisk_insert '{dsk}'\nwait 3000000\n\
         key_delay 70000 70000 400000\nkey_output 'RUN\"HBL\\(RET)'\nwait {}\n",
        wait * 1_000_000
    );
    runner.run_text(&script, ".").expect("boot");
    let mb = runner.motherboard_mut();
    // align on the start of a rupture cycle (PC=9B8A: OUT mode0)
    let mut guard = 0;
    while mb.pc() != 0x9b8a {
        mb.execute_next_opcode_no_video();
        guard += 1;
        assert!(guard < 5_000_000, "never reached PC=9B8A");
    }
    let snap = |mb: &amsrust::motherboard::Motherboard| {
        (mb.crtc().hsync(), mb.ga().hsync_ends(), mb.ga().next_video_mode(), mb.ga().active_video_mode(), mb.crtc().r2(), mb.crtc().r3(), mb.pc())
    };
    let mut last = snap(mb);
    println!("tick 0 PC={:04X} crtc_hsync={} ga_hsync_ends={} next={:?} active={:?} R2={} R3={}", last.6, last.0, last.1, last.2, last.3, last.4, last.5);
    for t in 1..=ticks {
        mb.tick();
        let now = snap(mb);
        let mut ev = Vec::new();
        if now.0 != last.0 { ev.push(format!("CRTC_HSYNC->{}", now.0)); }
        if now.1 != last.1 { ev.push("GA_HSYNC_END".to_string()); }
        if now.2 != last.2 { ev.push(format!("RMR next->{:?}", now.2)); }
        if now.3 != last.3 { ev.push(format!("ACTIVE->{:?}", now.3)); }
        if now.4 != last.4 || now.5 != last.5 { ev.push(format!("R2/R3 = {}/{}", now.4, now.5)); }
        if !ev.is_empty() {
            println!("tick {t:5} PC={:04X} {}", now.6, ev.join(", "));
        }
        last = now;
    }
}
