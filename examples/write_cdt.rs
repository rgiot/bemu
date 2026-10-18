//! Writes a CDT holding a binary file, as the CPC firmware would save it.
//!
//! `cargo run --release --example write_cdt -- <out.cdt> <name> <load> <entry> <file.bin> [baud]`
//! (addresses in hex).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [out, name, load, entry, input, rest @ ..] = args.as_slice() else {
        eprintln!("usage: write_cdt <out.cdt> <name> <load hex> <entry hex> <file.bin> [baud]");
        std::process::exit(2);
    };
    let hex = |s: &str| u16::from_str_radix(s.trim_start_matches("0x").trim_start_matches('&'), 16).expect("hex address");
    let baud = rest.first().map_or(2000, |b| b.parse().expect("baud"));
    let data = std::fs::read(input).expect("input");
    std::fs::write(out, amsrust::tape::cdt_of_binary(name, hex(load), hex(entry), &data, baud)).expect("output");
}
