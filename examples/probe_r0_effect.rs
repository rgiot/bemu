//! After a CSL script: for the next `<changes>` changes of CRTC R0, the instruction that made it
//! (its first two opcode bytes), the microseconds (with the tick) from its start to the change and
//! C0 then: where in an instruction a register write takes effect.
//! `probe_r0_effect <script.csl> <changes>`
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let changes: usize = a.next().unwrap().parse().unwrap();
    let mut runner = CslRunner::new("/tmp/probe_r0_effect_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut ticks = 0u64;
    let mut start = (0u64, 0u16);
    let mut r0 = mb.crtc().r0();
    let mut seen = 0;
    let mut aligned_before = false;
    while seen < changes {
        mb.tick();
        ticks += 1;
        let aligned = mb.is_aligned_with_new_instruction();
        if aligned && !aligned_before {
            start = (ticks, mb.pc());
        }
        aligned_before = aligned;
        let now = mb.crtc().r0();
        if now != r0 {
            let pc = start.1;
            let bytes = [mb.read_byte(pc), mb.read_byte(pc.wrapping_add(1))];
            let since = ticks - start.0;
            println!(
                "R0 {r0}->{now} by PC={pc:04X} [{:02X} {:02X}] {} us {} ticks after its start, C0={}",
                bytes[0],
                bytes[1],
                since / 16,
                since % 16,
                mb.crtc().hcc
            );
            seen += 1;
            r0 = now;
        }
    }
}
