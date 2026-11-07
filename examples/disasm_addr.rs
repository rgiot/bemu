//! Disassemble a range of a 64K memory dump, with addresses (diagnostics).
//! `disasm_addr <file> <start_hex> <len_hex>`; `start` must be an instruction boundary.
use cpclib_asm::TokenExt;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("file");
    let start = usize::from_str_radix(&a.next().expect("start"), 16).unwrap();
    let len = usize::from_str_radix(&a.next().expect("len"), 16).unwrap();
    let data = std::fs::read(path).unwrap();
    let listing = cpclib_asm::disass::disassemble(&data[start..start + len]);
    let mut address = start;
    for token in listing.listing() {
        let size = token.number_of_bytes().unwrap_or(0);
        let bytes: String = data[address..address + size]
            .iter()
            .map(|b| format!("{b:02X} "))
            .collect();
        println!("{address:04X}  {bytes:<12}{token}");
        address += size;
    }
}
