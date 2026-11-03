//! After a CSL script, record the CPU's writes to an address range for a number of frames,
//! with the beam position: `probe_page_writes <script.csl> <from_hex> <to_hex> [microseconds]`
//! (the CPC+ palette is 6400 643F).
use amsrust::csl_runner::CslRunner;
use amsrust::motherboard::BusAccessKind;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let from = u16::from_str_radix(&a.next().expect("from"), 16).unwrap();
    let to = u16::from_str_radix(&a.next().expect("to"), 16).unwrap();
    let length: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(19968);
    let mut runner = CslRunner::new("/tmp/probe_page_writes_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    mb.watch_bus(true);
    let start = mb.t_state_counter().value() / 4;
    let show_vma = std::env::var("PROBE_VMA").is_ok();
    let mut last_line = (255, 255);
    while mb.t_state_counter().value() / 4 < start + length {
        let pc = mb.pc();
        mb.execute_next_opcode_no_video();
        let c = mb.crtc().counters();
        if show_vma && (c.vcc, c.vlc) != last_line {
            last_line = (c.vcc, c.vlc);
            println!("line C4={} C9={} C0={} vma={:04X}", c.vcc, c.vlc, c.hcc, mb.crtc().vma());
        }
        for access in mb.take_bus_accesses() {
            if (access.kind == BusAccessKind::MemoryWrite && (from..=to).contains(&access.address))
                || (access.kind == BusAccessKind::IoWrite && access.address >> 8 == 0x7f && std::env::var("PROBE_GA").is_ok())
            {
                println!(
                    "us={} pc={pc:04X} {:04X}={:02X} C0={} C4={} C9={}",
                    mb.t_state_counter().value() / 4 - start, access.address, access.value, c.hcc, c.vcc, c.vlc
                );
            }
        }
    }
}
