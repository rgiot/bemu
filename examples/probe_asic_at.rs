//! After a CSL script, run until the PC hits an address `n` times, printing the CPC+ ASIC's
//! registers (PRI, SPLT, SSA, SSCR, IVR, DCSR), the Gate Array's R52 and the CRTC counters each
//! time: `probe_asic_at <script.csl> <pc_hex> <hits> [max_instr]`.
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let pc = u16::from_str_radix(&a.next().expect("pc"), 16).unwrap();
    let hits: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let max: u64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(400_000_000);
    let mut runner = CslRunner::new("/tmp/probe_asic_at_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let (mut n, mut count) = (0, 0u64);
    // PROBE_TRACE=from_us,to_us: every instruction in that window, with PRI, R52 and the INT line
    let trace: Option<(usize, usize)> = std::env::var("PROBE_TRACE").ok().and_then(|s| {
        let (a, b) = s.split_once(',')?;
        Some((a.parse().ok()?, b.parse().ok()?))
    });
    while n < hits && count < max {
        mb.execute_next_opcode_no_video();
        count += 1;
        if let Some((from, to)) = trace {
            let us = mb.t_state_counter().value() / 4;
            if us >= to {
                break;
            }
            if (from..to).contains(&us) {
                let chunk = mb.asic_snapshot_chunk().unwrap_or_default();
                let c = mb.crtc().counters();
                println!(
                    "us={us} pc={:04X} PRI={:02X} R52={} C4={} C9={} vsync={} IFF1={}",
                    mb.pc(),
                    chunk.get(0x8c0).copied().unwrap_or(0),
                    mb.ga().r52(),
                    c.vcc,
                    c.vlc,
                    c.vsync,
                    mb.cpu().iff1()
                );
            }
        }
        if mb.pc() != pc {
            continue;
        }
        let chunk = mb.asic_snapshot_chunk().unwrap_or_default();
        let c = mb.crtc().counters();
        println!(
            "hit {n}: us={} PRI={:02X} SPLT={:02X} SSA={:02X}{:02X} SSCR={:02X} IVR={:02X} DCSR={:02X} R52={} C0={} C4={} C9={} vsync={} hsync={} IFF1={}",
            mb.t_state_counter().value() / 4,
            chunk.get(0x8c0).copied().unwrap_or(0),
            chunk.get(0x8c1).copied().unwrap_or(0),
            chunk.get(0x8c2).copied().unwrap_or(0),
            chunk.get(0x8c3).copied().unwrap_or(0),
            chunk.get(0x8c4).copied().unwrap_or(0),
            chunk.get(0x8c5).copied().unwrap_or(0),
            chunk.get(0x8df).copied().unwrap_or(0),
            mb.ga().r52(),
            c.hcc, c.vcc, c.vlc, c.vsync, c.hsync,
            mb.cpu().iff1()
        );
        n += 1;
    }
}
