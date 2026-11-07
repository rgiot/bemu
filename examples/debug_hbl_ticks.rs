//! Tick-level view of one raster line of HBL's moving-letters scene: CRTC
//! HSYNC edges, GA HSYNC-end events and mode (RMR next / active) changes.
//!
//! `cargo run --release --example debug_hbl_ticks -- <HBL.DSK> <wait_s> <ticks>`
//! or `-- --csl <script.csl> <align_pc_hex> <ticks>` to trace any CSL-booted program
//! (aligns on the first execution of that PC).

use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let dsk = a.next().expect("usage: <HBL.DSK> <wait_s> <ticks>");
    let csl_path = if dsk == "--csl" {
        Some(a.next().expect("script"))
    } else {
        None
    };
    let wait_arg = a.next();
    let align_pc: u16 = if csl_path.is_some() {
        u16::from_str_radix(wait_arg.as_deref().unwrap_or("9b8a"), 16).unwrap()
    } else {
        0x9b8a
    };
    let wait: u64 = if csl_path.is_some() {
        0
    } else {
        wait_arg.and_then(|s| s.parse().ok()).unwrap_or(90)
    };
    let ticks: u64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(1100);
    let mut runner = CslRunner::new("/tmp/debug_hbl_ticks_shots");
    let script = format!(
        "csl_version 1.0\ncrtc_select 0\nreset\ndisk_insert '{dsk}'\nwait 3000000\n\
         key_delay 70000 70000 400000\nkey_output 'RUN\"HBL\" \\(')CRTC=0\\(RET)'\nwait {}\n",
        wait * 1_000_000
    );
    if let Some(path) = &csl_path {
        let text = std::fs::read_to_string(path).unwrap();
        let base = std::path::Path::new(path).parent().unwrap().to_path_buf();
        runner.run_text(&text, &base).expect("csl");
    } else {
        runner.run_text(&script, ".").expect("boot");
    }
    let mb = runner.motherboard_mut();
    // align on the start of a rupture cycle (PC=9B8A: OUT mode0)
    let mut guard = 0;
    while mb.pc() != align_pc {
        mb.execute_next_opcode_no_video();
        guard += 1;
        assert!(guard < 5_000_000, "never reached the alignment PC");
    }
    let snap = |mb: &amsrust::motherboard::Motherboard| {
        (
            mb.crtc().hsync(),
            mb.ga().hsync_ends(),
            mb.ga().next_video_mode(),
            mb.ga().active_video_mode(),
            mb.crtc().r2(),
            mb.crtc().r3(),
            mb.pc(),
        )
    };
    let mut last = snap(mb);
    let mut vs_prev = mb.crtc().vsync();
    println!(
        "tick 0 PC={:04X} crtc_hsync={} ga_hsync_ends={} next={:?} active={:?} R2={} R3={}",
        last.6, last.0, last.1, last.2, last.3, last.4, last.5
    );
    let (rgb_from, rgb_to): (u64, u64) = std::env::var("RGB_RANGE")
        .ok()
        .and_then(|v| {
            let mut p = v.split('-');
            Some((p.next()?.parse().ok()?, p.next()?.parse().ok()?))
        })
        .unwrap_or((0, 0));
    for t in 1..=ticks {
        mb.tick();
        if t >= rgb_from && t < rgb_to {
            println!(
                "rgb tick {t:5} hsync_ga={} rgb={:?} active={:?}",
                mb.ga().hsync_ga(),
                mb.ga().rgb(),
                mb.ga().active_video_mode()
            );
        }
        let now = snap(mb);
        let mut ev = Vec::new();
        if mb.crtc().vsync() != vs_prev {
            vs_prev = mb.crtc().vsync();
            println!(
                "tick {t:5} CRTC VSYNC -> {} (C0={})",
                vs_prev,
                mb.crtc().hcc
            );
        }
        if now.0 != last.0 {
            ev.push(format!("CRTC_HSYNC->{}", now.0));
        }
        if now.1 != last.1 {
            ev.push("GA_HSYNC_END".to_string());
        }
        if now.2 != last.2 {
            ev.push(format!("RMR next->{:?} (C0={})", now.2, mb.crtc().hcc));
        }
        if now.3 != last.3 {
            ev.push(format!("ACTIVE->{:?}", now.3));
        }
        if now.4 != last.4 || now.5 != last.5 {
            ev.push(format!(
                "R2/R3 = {}/{} (C0={})",
                now.4,
                now.5,
                mb.crtc().hcc
            ));
        }
        if !ev.is_empty() {
            println!("tick {t:5} PC={:04X} {}", now.6, ev.join(", "));
        }
        last = now;
    }
}
