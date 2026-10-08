//! After a CSL script and a run to a PC, print every CRTC register write
//! (C0, C4, C9, reg=value) line by line for N lines.
//! `probe_writes <script.csl> <pc_hex> <lines>`
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let pc = u16::from_str_radix(&a.next().expect("pc"), 16).unwrap();
    let lines: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(24);
    let mut runner = CslRunner::new("/tmp/probe_writes_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut guard = 0u64;
    while mb.pc() != pc {
        mb.execute_next_opcode_no_video();
        guard += 1;
        assert!(guard < 80_000_000, "never reached the pc");
    }
    mb.crtc_mut().start_write_log();
    let mut n = 0;
    let mut last_hcc = 255u8;
    let mut line = String::new();
    while n < lines {
        mb.tick();
        let hcc = mb.crtc().hcc;
        let writes = mb.crtc_mut().take_write_log();
        for (c0, c4, c9, r, v) in writes {
            line.push_str(&format!(" R{r}={v}@{c0}"));
            let _ = (c4, c9);
        }
        if hcc != last_hcc && hcc == 0 {
            let c = mb.crtc();
            println!("C4={:?} C9={} LL={} |{}", c.vcc, c.vlc.value(), c.last_line(), line);
            line.clear();
            n += 1;
        }
        last_hcc = hcc;
    }
}
