//! Runs a .sna for a while and prints a RAM range as hex: `probe_sna_ram <file.sna> <microseconds> <addr hex> <len> [crtc]`
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let usage = "usage: <file.sna> <us> <addr hex> <len> [crtc]";
    let sna = a.next().expect(usage);
    let us: usize = a.next().expect(usage).parse().unwrap();
    let addr = usize::from_str_radix(&a.next().expect(usage), 16).unwrap();
    let len: usize = a.next().expect(usage).parse().unwrap();
    let crtc: u8 = a.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut runner = CslRunner::new("/tmp/probe_sna_ram_shots");
    runner
        .run_text(
            &format!("csl_version 1.0\ncrtc_select {crtc}\nreset\nwait 1000000\n"),
            ".",
        )
        .unwrap();
    amsrust::snapshot::load_sna(runner.motherboard_mut(), &sna).expect("load sna");
    runner
        .run_text(&format!("csl_version 1.0\nwait {us}\n"), ".")
        .unwrap();
    let mb = runner.motherboard_mut();
    let bytes: Vec<String> = (0..len)
        .map(|i| format!("{:02X}", mb.read_byte(((addr + i) & 0xffff) as u16)))
        .collect();
    println!("{} pc={:04X}", bytes.join(" "), mb.pc());
}
