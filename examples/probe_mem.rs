//! After a CSL script: `<len>` bytes of memory from `<addr>` as hex, to read what a test program left
//! there (RAM above #4000: below it the lower ROM answers).
//! `probe_mem <script.csl> <addr_hex> <len>`
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let addr = u16::from_str_radix(&a.next().expect("addr"), 16).unwrap();
    let len: usize = a.next().unwrap().parse().unwrap();
    let mut runner = CslRunner::new("/tmp/probe_mem_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let bytes: Vec<String> = (0..len)
        .map(|i| format!("{:02x}", mb.read_byte(addr.wrapping_add(i as u16))))
        .collect();
    println!("{}", bytes.join(""));
}
