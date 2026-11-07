//! Disassemble a range of a binary file (diagnostics).
//! `disasm_bin <file> <start_hex> <len_hex>`
fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("file");
    let start = usize::from_str_radix(&a.next().expect("start"), 16).unwrap();
    let len = usize::from_str_radix(&a.next().expect("len"), 16).unwrap();
    let data = std::fs::read(path).unwrap();
    let listing = cpclib_asm::disass::disassemble(&data[start..start + len]);
    for t in listing.listing() {
        println!("{}", t);
    }
}
