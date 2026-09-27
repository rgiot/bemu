//! Differential test of the Z80 core: 10 000 single instructions executed
//! by a reference core (kosarev's `z80`, see `tests/z80_reference/generate.py`)
//! from random states, compared register by register - flags with their
//! undocumented bits included - and write by write.
//!
//! Instructions doing I/O or touching the interrupt logic are not part of
//! the set, and neither is the internal MEMPTR register, which the reference
//! does not expose (the flags leaking it, like BIT n,(HL), are kept out of
//! the comparison through `ignored_flag_bits`).

use amsrust::motherboard::{Motherboard, MotherboardBuilder};
use amsrust::z80::{HasValue, Reg16, Reg8};
use std::collections::BTreeMap;

const VECTORS: &str = include_str!("z80_reference/vectors.txt");

/// The bits of F not compared for an instruction: BIT n,(HL) and
/// BIT n,(IX+d) take their X and Y flags from MEMPTR, which the reference
/// does not expose.
fn ignored_flag_bits(code: &[u8]) -> u8 {
    let is_bit = |op: u8| (0x40..0x80).contains(&op);
    match code {
        [0xcb, op, ..] if is_bit(*op) && op & 7 == 6 => 0x28,
        [0xdd | 0xfd, 0xcb, _, op, ..] if is_bit(*op) => 0x28,
        _ => 0
    }
}

/// Whether the refresh register is compared.
const COMPARE_R: bool = true;

fn fresh_motherboard() -> Motherboard {
    MotherboardBuilder::default()
        .disable_lower_rom()
        .disable_upper_rom()
        .build()
}

fn memory_byte(address: u16, seed: u32) -> u8 {
    ((u32::from(address) ^ seed).wrapping_mul(0x9E37_79B1) >> 13) as u8
}

/// AF BC DE HL AF' BC' DE' HL' IX IY SP PC I R IFF1 IFF2
type State = [u16; 16];

const REG16: [Reg16; 12] = [
    Reg16::Af, Reg16::Bc, Reg16::De, Reg16::Hl,
    Reg16::Af2, Reg16::Bc2, Reg16::De2, Reg16::Hl2,
    Reg16::Ix, Reg16::Iy, Reg16::Sp, Reg16::Pc
];

fn parse_state<'a>(fields: &mut impl Iterator<Item = &'a str>) -> State {
    let mut state = [0; 16];
    for value in state.iter_mut() {
        *value = u16::from_str_radix(fields.next().unwrap(), 16).unwrap();
    }
    state
}

fn load(motherboard: &mut Motherboard, code: &[u8], seed: u32, state: &State) {
    let memory: Vec<u8> = (0..=0xffffu16).map(|address| memory_byte(address, seed)).collect();
    motherboard.write_buffer(0, &memory[..0x8000]);
    motherboard.write_buffer(0x8000, &memory[0x8000..]);
    motherboard.write_buffer(state[11], code);

    let cpu = motherboard.cpu_mut();
    for (register, value) in REG16.iter().zip(state) {
        cpu.reg16_mut(*register).set(*value);
    }
    cpu.reg16_mut(Reg16::Wz).set(0);
    cpu.reg8_mut(Reg8::I).set(state[12] as u8);
    cpu.reg8_mut(Reg8::R).set(state[13] as u8);
    cpu.set_iffs(state[14] != 0, state[15] != 0);
}

fn read_state(motherboard: &Motherboard) -> State {
    let cpu = motherboard.cpu();
    let mut state = [0; 16];
    for (value, register) in state.iter_mut().zip(REG16) {
        *value = cpu.reg16(register).value();
    }
    state[12] = u16::from(cpu.reg8(Reg8::I).value());
    state[13] = u16::from(cpu.reg8(Reg8::R).value());
    state[14] = u16::from(cpu.iff1());
    state[15] = u16::from(cpu.iff2());
    state
}

const NAMES: [&str; 16] = [
    "AF", "BC", "DE", "HL", "AF'", "BC'", "DE'", "HL'", "IX", "IY", "SP", "PC", "I", "R", "IFF1", "IFF2"
];

#[test]
fn every_instruction_matches_the_reference_core() {
    // panics are reported with the failures, not one by one
    std::panic::set_hook(Box::new(|_| {}));
    let mut motherboard = fresh_motherboard();

    // failures per instruction opcode (without operands): count, first description
    let mut failures: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut total = 0;

    for line in VECTORS.lines() {
        let mut fields = line.split_whitespace();
        let code_hex = fields.next().unwrap();
        let code: Vec<u8> = (0..code_hex.len() / 2)
            .map(|i| u8::from_str_radix(&code_hex[2 * i..2 * i + 2], 16).unwrap())
            .collect();
        let seed = u32::from_str_radix(fields.next().unwrap(), 16).unwrap();
        let before = parse_state(&mut fields);
        let mut expected = parse_state(&mut fields);
        let writes: Vec<(u16, u8)> = fields
            .map(|w| {
                let (address, value) = w.split_once('=').unwrap();
                (u16::from_str_radix(address, 16).unwrap(), u8::from_str_radix(value, 16).unwrap())
            })
            .collect();

        load(&mut motherboard, &code, seed, &before);
        // a core that panics on an instruction is a failure like another
        let executed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            motherboard.execute_next_opcode_no_video()
        }));
        if executed.is_err() {
            motherboard = fresh_motherboard();
        }
        let mut actual = read_state(&motherboard);

        // where the instruction is (its bytes are not part of memory_byte)
        let mut expected_memory: BTreeMap<u16, u8> = BTreeMap::new();
        for (offset, byte) in code.iter().enumerate() {
            expected_memory.insert(before[11].wrapping_add(offset as u16), *byte);
        }
        for (address, value) in &writes {
            expected_memory.insert(*address, *value);
        }

        let mask = u16::from(ignored_flag_bits(&code));
        expected[0] &= !mask;
        actual[0] &= !mask;

        let mut problems = Vec::new();
        if executed.is_err() {
            problems.push("the core panicked".to_owned());
        }
        for (index, name) in NAMES.iter().enumerate() {
            if index == 13 && !COMPARE_R {
                continue;
            }
            if actual[index] != expected[index] {
                problems.push(format!(
                    "{name}: {:04x}, reference {:04x}",
                    actual[index], expected[index]
                ));
            }
        }
        for (address, value) in &expected_memory {
            let found = motherboard.read_byte(*address);
            if found != *value {
                problems.push(format!("[{address:04x}]: {found:02x}, reference {value:02x}"));
            }
        }
        total += 1;

        if !problems.is_empty() {
            let key = if code[0] == 0xdd || code[0] == 0xfd {
                if code[1] == 0xcb { format!("{:02x}{:02x}..{:02x}", code[0], code[1], code[3]) } else { format!("{:02x}{:02x}", code[0], code[1]) }
            }
            else if code[0] == 0xcb || code[0] == 0xed {
                format!("{:02x}{:02x}", code[0], code[1])
            }
            else {
                format!("{:02x}", code[0])
            };
            let entry = failures.entry(key).or_insert((0, String::new()));
            entry.0 += 1;
            if entry.1.is_empty() {
                entry.1 = format!("{code_hex} from AF={:04x} BC={:04x} HL={:04x}: {}", before[0], before[1], before[3], problems.join(", "));
            }
        }
    }

    let _ = std::panic::take_hook();
    if !failures.is_empty() {
        let report: Vec<String> = failures
            .iter()
            .map(|(opcode, (count, example))| format!("{opcode}: {count} failures, e.g. {example}"))
            .collect();
        panic!("{} of {total} vectors differ, {} instructions:\n{}", failures.values().map(|f| f.0).sum::<usize>(), failures.len(), report.join("\n"));
    }
}
