//! After a CSL script, count sync edges over some ticks and list the CRTC
//! register sets seen. `probe_sync <script.csl> <ticks>`
use amsrust::csl_runner::CslRunner;
use std::collections::BTreeMap;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let ticks: u64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(2_000_000);
    let mut runner = CslRunner::new("/tmp/probe_sync_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let (mut vs_edges, mut hs_edges, mut c_vs, mut c_hs) = (0, 0, 0, 0);
    let (mut pv, mut ph, mut pcv, mut pch) = (false, false, false, false);
    let mut sets: BTreeMap<[u8; 8], u64> = BTreeMap::new();
    for _ in 0..ticks {
        let (_rgb, sync) = mb.tick();
        let c = mb.crtc();
        if c.vsync() && !pv { vs_edges += 1; }
        if c.hsync() && !ph { hs_edges += 1; }
        if sync.vsync && !pcv { c_vs += 1; }
        if sync.hsync && !pch { c_hs += 1; }
        pv = c.vsync(); ph = c.hsync(); pcv = sync.vsync; pch = sync.hsync;
        *sets.entry([c.r0(), c.r1(), c.r2(), c.r3(), c.r4(), c.r6(), c.r7(), c.r9()]).or_default() += 1;
    }
    println!("CRTC vsync edges {vs_edges}, hsync edges {hs_edges}; C-VSYNC {c_vs}, C-HSYNC {c_hs}");
    println!("sets [R0,R1,R2,R3,R4,R6,R7,R9] : ticks");
    for (k, v) in sets { println!("{k:?} {v}"); }
}
