//! Execute a few bytes of Z80 code at &4000 (ROMs off) and print PC and the main registers after
//! each instruction: `probe_opcodes FD DD 21 34 12 ED FC 76`.
fn main() {
    let bytes: Vec<u8> = std::env::args().skip(1).map(|b| u8::from_str_radix(&b, 16).expect("hex bytes")).collect();
    let mut mb = amsrust::motherboard::MotherboardBuilder::default();
    let mut mb = mb.disable_lower_rom().disable_upper_rom().build();
    for (offset, byte) in bytes.iter().enumerate() {
        mb.write_byte(0x4000 + offset as u16, *byte);
    }
    mb.cpu_mut().set_pc(0x4000);
    for _ in 0..bytes.len() {
        if mb.pc() as usize >= 0x4000 + bytes.len() {
            break;
        }
        let pc = mb.pc();
        mb.execute_next_opcode_no_video();
        println!(
            "{pc:04X} -> PC={:04X} AF={:04X} BC={:04X} DE={:04X} HL={:04X} IX={:04X} IY={:04X} SP={:04X}",
            mb.pc(), mb.af(), mb.bc(), mb.de(), mb.hl(), mb.ix(), mb.iy(), mb.sp()
        );
    }
}
