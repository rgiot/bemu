//! Structural Z80 instruction decoder.
//!
//! Builds the `Token` of an instruction directly from the bits of its
//! opcode, without going through assembly text: the opcode byte is split
//! into its x (bits 7-6), y (bits 5-3), z (bits 2-0), p (bits 5-4) and q
//! (bit 3) fields, which select the instruction family and its register,
//! condition, ALU operation, rotation or bit operands (Zilog Z80 CPU User
//! Manual UM0080, instruction tables; the field layout is the documented
//! encoding of every opcode group).
//!
//! The tokens are exactly the ones the emulator used to obtain by parsing
//! the `cpclib_tokens::opcode_table` strings (`decoder_matches_the_string_
//! tables` checks every opcode of every prefix against them), with one
//! deliberate exception: `RST p` carries its real target address (`RST
//! 0x10` is `Value(16)`) where the tables' "RST 10" parsed as decimal 10.
//!
//! Operands come from an [`Operands`] source, so one decoding serves the
//! three uses:
//! - [`Template`]: placeholder labels "nn"/"nnnn", for the per-opcode
//!   lookup tables built once (`states.rs`);
//! - [`Bytes`]: the operand bytes actually read, for execution and
//!   disassembly;
//! - a counter, for the number of operand bytes an opcode needs.
//!
//! Opcodes with no instruction of their own (holes of the ED page, and
//! the prefix bytes themselves) decode as `NOP`. A DD/FD-prefixed opcode
//! that touches neither HL, H, L nor (HL) executes as its unprefixed form.

use std::cell::Cell;

use cpclib_asm::preamble::{
    BinaryOperation, DataAccess, Expr, FlagTest, IndexRegister16, IndexRegister8, Mnemonic,
    Register16, Register8, Token,
};

use super::OpcodePrefix;

/// Where the operands of an instruction come from.
pub trait Operands {
    /// An 8-bit operand: immediate value, port number or index
    /// displacement. `index` is its position among the operand bytes.
    fn byte(&self, index: usize) -> Expr;
    /// A 16-bit operand (little endian, operand bytes 0 and 1).
    fn word(&self) -> Expr;
    /// The offset of JR/DJNZ, relative to the instruction's own address.
    fn relative(&self) -> Expr;
}

/// Placeholder operands, as in the assembly tables: "nn" and "nnnn".
pub struct Template;

impl Operands for Template {
    fn byte(&self, _index: usize) -> Expr {
        Expr::Label("nn".into())
    }

    fn word(&self) -> Expr {
        Expr::Label("nnnn".into())
    }

    fn relative(&self) -> Expr {
        Expr::Label("nn".into())
    }
}

/// The operand bytes following the opcode (for DD CB / FD CB, the
/// displacement in byte 0).
pub struct Bytes(pub [u8; 2]);

impl Operands for Bytes {
    fn byte(&self, index: usize) -> Expr {
        // Unsigned, displacements included, as the tables' "0x85" was.
        Expr::Value(self.0[index] as i32)
    }

    fn word(&self) -> Expr {
        Expr::Value(u16::from_le_bytes(self.0) as i32)
    }

    fn relative(&self) -> Expr {
        // The displacement counts from the next instruction, 2 bytes on.
        match self.0[0] as i8 as i32 + 2 {
            0 => Expr::Label("$".into()),
            offset => Expr::Value(offset)
        }
    }
}

/// Counts the operand bytes the decoding asks for.
#[derive(Default)]
struct Counter(Cell<u8>);

impl Counter {
    fn need(&self, nb: u8) {
        self.0.set(self.0.get().max(nb));
    }
}

impl Operands for Counter {
    fn byte(&self, index: usize) -> Expr {
        self.need(index as u8 + 1);
        Expr::Value(0)
    }

    fn word(&self) -> Expr {
        self.need(2);
        Expr::Value(0)
    }

    fn relative(&self) -> Expr {
        self.need(1);
        Expr::Value(0)
    }
}

/// The instruction `opcode` encodes after `prefix`, operands from `operands`.
pub fn decode(prefix: OpcodePrefix, opcode: u8, operands: &impl Operands) -> Token {
    let decoded = match prefix {
        OpcodePrefix::None => unprefixed(opcode, operands),
        OpcodePrefix::Cb => Some(cb(opcode)),
        OpcodePrefix::Ed => ed(opcode, operands),
        OpcodePrefix::Dd => indexed(opcode, IndexRegister16::Ix, operands),
        OpcodePrefix::Fd => indexed(opcode, IndexRegister16::Iy, operands),
        OpcodePrefix::DdCb => Some(indexed_cb(opcode, IndexRegister16::Ix, operands)),
        OpcodePrefix::FdCb => Some(indexed_cb(opcode, IndexRegister16::Iy, operands))
    };
    decoded.unwrap_or_else(nop)
}

/// The instruction with placeholder operands (for the lookup tables).
pub fn template(prefix: OpcodePrefix, opcode: u8) -> Token {
    decode(prefix, opcode, &Template)
}

/// The instruction with its actual operand bytes.
pub fn with_operands(prefix: OpcodePrefix, opcode: u8, operands: [u8; 2]) -> Token {
    decode(prefix, opcode, &Bytes(operands))
}

/// How many operand bytes follow `opcode` (DD CB / FD CB: the displacement
/// and the opcode, which come after the prefixes).
pub fn nb_operand_bytes(prefix: OpcodePrefix, opcode: u8) -> u8 {
    if matches!(prefix, OpcodePrefix::DdCb | OpcodePrefix::FdCb) {
        return 2;
    }
    let counter = Counter::default();
    decode(prefix, opcode, &counter);
    counter.0.get()
}

// ------------------------------------------------------------- fields

fn nop() -> Token {
    op(Mnemonic::Nop, None, None)
}

fn op(mnemonic: Mnemonic, arg1: Option<DataAccess>, arg2: Option<DataAccess>) -> Token {
    Token::OpCode(mnemonic, arg1, arg2, None)
}

/// x, y, z, p, q fields of an opcode.
fn fields(opcode: u8) -> (u8, u8, u8, u8, u8) {
    let y = (opcode >> 3) & 7;
    (opcode >> 6, y, opcode & 7, y >> 1, y & 1)
}

/// r[i]: B, C, D, E, H, L, (HL), A.
fn r(index: u8) -> DataAccess {
    match index {
        0 => DataAccess::Register8(Register8::B),
        1 => DataAccess::Register8(Register8::C),
        2 => DataAccess::Register8(Register8::D),
        3 => DataAccess::Register8(Register8::E),
        4 => DataAccess::Register8(Register8::H),
        5 => DataAccess::Register8(Register8::L),
        6 => DataAccess::MemoryRegister16(Register16::Hl),
        _ => DataAccess::Register8(Register8::A)
    }
}

/// The register r[i] designates (none for (HL)).
fn register8(index: u8) -> Option<Register8> {
    match r(index) {
        DataAccess::Register8(register) => Some(register),
        _ => None
    }
}

/// rp[i]: BC, DE, HL, SP.
fn rp(index: u8) -> Register16 {
    [Register16::Bc, Register16::De, Register16::Hl, Register16::Sp][index as usize]
}

/// rp2[i]: BC, DE, HL, AF.
fn rp2(index: u8) -> Register16 {
    [Register16::Bc, Register16::De, Register16::Hl, Register16::Af][index as usize]
}

/// cc[i]: NZ, Z, NC, C, PO, PE, P, M.
fn cc(index: u8) -> DataAccess {
    DataAccess::FlagTest(
        [
            FlagTest::NZ,
            FlagTest::Z,
            FlagTest::NC,
            FlagTest::C,
            FlagTest::PO,
            FlagTest::PE,
            FlagTest::P,
            FlagTest::M
        ][index as usize]
    )
}

/// alu[y] with operand `operand`. SBC keeps the explicit accumulator
/// ("SBC A,r") the others omit ("ADD r"), like the tables.
fn alu(y: u8, operand: DataAccess) -> Token {
    let mnemonic = [
        Mnemonic::Add,
        Mnemonic::Adc,
        Mnemonic::Sub,
        Mnemonic::Sbc,
        Mnemonic::And,
        Mnemonic::Xor,
        Mnemonic::Or,
        Mnemonic::Cp
    ][y as usize];
    let accumulator = (mnemonic == Mnemonic::Sbc).then_some(DataAccess::Register8(Register8::A));
    op(mnemonic, accumulator, Some(operand))
}

/// rot[y]: RLC, RRC, RL, RR, SLA, SRA, SL1 (undocumented SLL), SRL.
fn rot(y: u8) -> Mnemonic {
    [
        Mnemonic::Rlc,
        Mnemonic::Rrc,
        Mnemonic::Rl,
        Mnemonic::Rr,
        Mnemonic::Sla,
        Mnemonic::Sra,
        Mnemonic::Sl1,
        Mnemonic::Srl
    ][y as usize]
}

fn value(v: i32) -> DataAccess {
    DataAccess::Expression(Expr::Value(v))
}

fn reg16(register: Register16) -> DataAccess {
    DataAccess::Register16(register)
}

// -------------------------------------------------------------- pages

/// The unprefixed page. `None` for the prefix bytes CB, DD, ED, FD.
fn unprefixed(opcode: u8, operands: &impl Operands) -> Option<Token> {
    use DataAccess::{Expression, Memory, MemoryRegister16, PortN};
    let (x, y, z, p, q) = fields(opcode);
    let a = || DataAccess::Register8(Register8::A);
    Some(match (x, z) {
        (0, 0) => match y {
            0 => nop(),
            1 => op(Mnemonic::ExAf, None, None),
            2 => op(Mnemonic::Djnz, Some(Expression(operands.relative())), None),
            3 => op(Mnemonic::Jr, None, Some(Expression(operands.relative()))),
            _ => op(Mnemonic::Jr, Some(cc(y - 4)), Some(Expression(operands.relative())))
        },
        (0, 1) if q == 0 => op(Mnemonic::Ld, Some(reg16(rp(p))), Some(Expression(operands.word()))),
        (0, 1) => op(Mnemonic::Add, Some(reg16(Register16::Hl)), Some(reg16(rp(p)))),
        (0, 2) => {
            let (memory, register) = match p {
                0 => (MemoryRegister16(Register16::Bc), a()),
                1 => (MemoryRegister16(Register16::De), a()),
                2 => (Memory(operands.word()), reg16(Register16::Hl)),
                _ => (Memory(operands.word()), a())
            };
            if q == 0 {
                op(Mnemonic::Ld, Some(memory), Some(register))
            }
            else {
                op(Mnemonic::Ld, Some(register), Some(memory))
            }
        },
        (0, 3) => op([Mnemonic::Inc, Mnemonic::Dec][q as usize], Some(reg16(rp(p))), None),
        (0, 4) => op(Mnemonic::Inc, Some(r(y)), None),
        (0, 5) => op(Mnemonic::Dec, Some(r(y)), None),
        (0, 6) => op(Mnemonic::Ld, Some(r(y)), Some(Expression(operands.byte(0)))),
        (0, _) => op(
            [
                Mnemonic::Rlca,
                Mnemonic::Rrca,
                Mnemonic::Rla,
                Mnemonic::Rra,
                Mnemonic::Daa,
                Mnemonic::Cpl,
                Mnemonic::Scf,
                Mnemonic::Ccf
            ][y as usize],
            None,
            None
        ),
        (1, 6) if y == 6 => op(Mnemonic::Halt, None, None),
        (1, _) => op(Mnemonic::Ld, Some(r(y)), Some(r(z))),
        (2, _) => alu(y, r(z)),
        (_, 0) => op(Mnemonic::Ret, Some(cc(y)), None),
        (_, 1) => match (q, p) {
            (0, _) => op(Mnemonic::Pop, Some(reg16(rp2(p))), None),
            (_, 0) => op(Mnemonic::Ret, None, None),
            (_, 1) => op(Mnemonic::Exx, None, None),
            (_, 2) => op(Mnemonic::Jp, None, Some(MemoryRegister16(Register16::Hl))),
            _ => op(Mnemonic::Ld, Some(reg16(Register16::Sp)), Some(reg16(Register16::Hl)))
        },
        (_, 2) => op(Mnemonic::Jp, Some(cc(y)), Some(Expression(operands.word()))),
        (_, 3) => match y {
            0 => op(Mnemonic::Jp, None, Some(Expression(operands.word()))),
            1 => return None, // CB prefix
            2 => op(Mnemonic::Out, Some(PortN(operands.byte(0))), Some(a())),
            3 => op(Mnemonic::In, Some(a()), Some(PortN(operands.byte(0)))),
            4 => op(Mnemonic::ExMemSp, Some(reg16(Register16::Hl)), None),
            5 => op(Mnemonic::ExHlDe, None, None),
            6 => op(Mnemonic::Di, None, None),
            _ => op(Mnemonic::Ei, None, None)
        },
        (_, 4) => op(Mnemonic::Call, Some(cc(y)), Some(Expression(operands.word()))),
        (_, 5) => match (q, p) {
            (0, _) => op(Mnemonic::Push, Some(reg16(rp2(p))), None),
            (_, 0) => op(Mnemonic::Call, None, Some(Expression(operands.word()))),
            _ => return None // DD, ED, FD prefixes
        },
        (_, 6) => alu(y, Expression(operands.byte(0))),
        _ => op(Mnemonic::Rst, Some(value(8 * y as i32)), None)
    })
}

/// The CB page: rotations/shifts, BIT, RES, SET on r[z].
fn cb(opcode: u8) -> Token {
    let (x, y, z, _, _) = fields(opcode);
    match x {
        0 => op(rot(y), Some(r(z)), None),
        _ => op(
            [Mnemonic::Bit, Mnemonic::Res, Mnemonic::Set][x as usize - 1],
            Some(value(y as i32)),
            Some(r(z))
        )
    }
}

/// The ED page. `None` for the opcodes with no instruction of their own.
fn ed(opcode: u8, operands: &impl Operands) -> Option<Token> {
    use DataAccess::{Memory, PortC};
    let (x, y, z, p, q) = fields(opcode);
    let a = || DataAccess::Register8(Register8::A);
    Some(match (x, z) {
        (1, 0) => {
            // y = 6: the undocumented IN (C), which only sets the flags.
            let destination = register8(y).map_or_else(|| value(0), DataAccess::Register8);
            op(Mnemonic::In, Some(destination), Some(PortC))
        },
        (1, 1) => {
            // y = 6: the undocumented OUT (C),0.
            let source = register8(y).map_or_else(|| value(0), DataAccess::Register8);
            op(Mnemonic::Out, Some(PortC), Some(source))
        },
        (1, 2) => op(
            [Mnemonic::Sbc, Mnemonic::Adc][q as usize],
            Some(reg16(Register16::Hl)),
            Some(reg16(rp(p)))
        ),
        (1, 3) if q == 0 => op(Mnemonic::Ld, Some(Memory(operands.word())), Some(reg16(rp(p)))),
        (1, 3) => op(Mnemonic::Ld, Some(reg16(rp(p))), Some(Memory(operands.word()))),
        (1, 4) => op(Mnemonic::Neg, None, None),
        (1, 5) if y == 1 => op(Mnemonic::Reti, None, None),
        (1, 5) => op(Mnemonic::Retn, None, None),
        (1, 6) => op(Mnemonic::Im, Some(value([0, 0, 1, 2][y as usize & 3])), None),
        (1, 7) => match y {
            0 => op(Mnemonic::Ld, Some(DataAccess::SpecialRegisterI), Some(a())),
            1 => op(Mnemonic::Ld, Some(DataAccess::SpecialRegisterR), Some(a())),
            2 => op(Mnemonic::Ld, Some(a()), Some(DataAccess::SpecialRegisterI)),
            3 => op(Mnemonic::Ld, Some(a()), Some(DataAccess::SpecialRegisterR)),
            4 => op(Mnemonic::Rrd, None, None),
            5 => op(Mnemonic::Rld, None, None),
            _ => return None
        },
        // Block instructions: y = 4..7 (increment, decrement, repeat
        // increment, repeat decrement), z = 0..3 (LD, CP, IN, OUT).
        (2, 0..=3) if y >= 4 => {
            let block = [
                [Mnemonic::Ldi, Mnemonic::Cpi, Mnemonic::Ini, Mnemonic::Outi],
                [Mnemonic::Ldd, Mnemonic::Cpd, Mnemonic::Ind, Mnemonic::Outd],
                [Mnemonic::Ldir, Mnemonic::Cpir, Mnemonic::Inir, Mnemonic::Otir],
                [Mnemonic::Lddr, Mnemonic::Cpdr, Mnemonic::Indr, Mnemonic::Otdr]
            ];
            op(block[y as usize - 4][z as usize], None, None)
        },
        _ => return None
    })
}

fn index_half(index: IndexRegister16, register: Register8) -> IndexRegister8 {
    match (index, register == Register8::H) {
        (IndexRegister16::Ix, true) => IndexRegister8::Ixh,
        (IndexRegister16::Ix, false) => IndexRegister8::Ixl,
        (IndexRegister16::Iy, true) => IndexRegister8::Iyh,
        (IndexRegister16::Iy, false) => IndexRegister8::Iyl
    }
}

/// The DD (IX) / FD (IY) page: the unprefixed instruction with HL
/// replaced by the index register. With (HL) it becomes (IX+d) and H/L
/// stay themselves; otherwise H and L become IXh and IXl. An instruction
/// using none of them executes unchanged, as its unprefixed form.
fn indexed(opcode: u8, index: IndexRegister16, operands: &impl Operands) -> Option<Token> {
    use DataAccess::{Expression, IndexRegister16WithIndex, MemoryIndexRegister16, MemoryRegister16};
    let displaced = || IndexRegister16WithIndex(index, BinaryOperation::Add, operands.byte(0));

    let token = match opcode {
        // LD (IX+d),n: the displacement comes first, the value second.
        0x36 => op(Mnemonic::Ld, Some(displaced()), Some(Expression(operands.byte(1)))),
        // JP (HL) jumps to HL itself: JP (IX), no displacement.
        0xe9 => op(Mnemonic::Jp, None, Some(MemoryIndexRegister16(index))),
        _ => unprefixed(opcode, operands)?
    };
    let Token::OpCode(mnemonic, arg1, arg2, arg3) = token
    else {
        unreachable!()
    };
    if opcode == 0x36 || opcode == 0xe9 {
        return Some(Token::OpCode(mnemonic, arg1, arg2, arg3));
    }

    let uses_memory = [&arg1, &arg2]
        .iter()
        .any(|arg| matches!(arg, Some(MemoryRegister16(Register16::Hl))));
    let substitute = |arg: Option<DataAccess>| -> Option<DataAccess> {
        Some(match arg? {
            MemoryRegister16(Register16::Hl) => displaced(),
            DataAccess::Register16(Register16::Hl) => DataAccess::IndexRegister16(index),
            DataAccess::Register8(register @ (Register8::H | Register8::L)) if !uses_memory => {
                DataAccess::IndexRegister8(index_half(index, register))
            },
            other => other
        })
    };
    let (new1, new2) = (substitute(arg1.clone()), substitute(arg2.clone()));
    if new1 == arg1 && new2 == arg2 {
        // Nothing to do with HL: same instruction as without the prefix.
        return Some(Token::OpCode(mnemonic, arg1, arg2, arg3));
    }
    Some(Token::OpCode(mnemonic, new1, new2, arg3))
}

/// DD CB d op / FD CB d op: the CB page on (IX+d). With z != 6 the
/// undocumented forms also copy the result to r[z] - for rotations as a
/// second argument, for RES/SET as the extra register of the token (BIT
/// only reads).
fn indexed_cb(opcode: u8, index: IndexRegister16, operands: &impl Operands) -> Token {
    let (x, y, z, _, _) = fields(opcode);
    let displaced =
        DataAccess::IndexRegister16WithIndex(index, BinaryOperation::Add, operands.byte(0));
    let copy = register8(z);
    match x {
        0 => op(rot(y), Some(displaced), copy.map(DataAccess::Register8)),
        1 => op(Mnemonic::Bit, Some(value(y as i32)), Some(displaced)),
        _ => Token::OpCode(
            [Mnemonic::Res, Mnemonic::Set][x as usize - 2],
            Some(value(y as i32)),
            Some(displaced),
            copy
        )
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use cpclib_asm::preamble::ParseToken;
    use cpclib_tokens::opcode_table::*;

    const PREFIXES: [OpcodePrefix; 7] = [
        OpcodePrefix::None,
        OpcodePrefix::Cb,
        OpcodePrefix::Dd,
        OpcodePrefix::Ed,
        OpcodePrefix::Fd,
        OpcodePrefix::DdCb,
        OpcodePrefix::FdCb
    ];

    fn table(prefix: OpcodePrefix) -> &'static [&'static str; 256] {
        match prefix {
            OpcodePrefix::None => &TABINSTR,
            OpcodePrefix::Cb => &TABINSTRCB,
            OpcodePrefix::Dd => &TABINSTRDD,
            OpcodePrefix::Ed => &TABINSTRED,
            OpcodePrefix::Fd => &TABINSTRFD,
            OpcodePrefix::DdCb => &TABINSTRDDCB,
            OpcodePrefix::FdCb => &TABINSTRFDCB
        }
    }

    /// What the emulator used to get from the assembly-text tables: an
    /// empty DD/FD entry falls back to the unprefixed one, any other
    /// empty entry is a NOP, warnings are unwrapped.
    fn reference(prefix: OpcodePrefix, opcode: u8, operands: Option<[u8; 2]>) -> Token {
        let mut lut = table(prefix);
        if lut[opcode as usize].is_empty() && matches!(prefix, OpcodePrefix::Dd | OpcodePrefix::Fd) {
            lut = &TABINSTR;
        }
        let parsed = match operands {
            None if lut[opcode as usize].is_empty() => Err(String::new()),
            None => Token::parse_token(lut[opcode as usize]),
            Some(bytes) => {
                cpclib_asm::disass::disassemble_with_potential_argument(opcode, lut, &bytes)
                    .map(|(token, _)| token)
            },
        };
        let token = parsed.map(|t| t.unwrapped().clone()).unwrap_or_else(|_| nop());
        // RST: the tables' "RST 10" parses as decimal 10.
        match token {
            Token::OpCode(Mnemonic::Rst, Some(DataAccess::Expression(Expr::Value(v))), None, None) => {
                let address = i32::from_str_radix(&v.to_string(), 16).unwrap();
                op(Mnemonic::Rst, Some(value(address)), None)
            },
            token => token
        }
    }

    #[test]
    fn templates_match_the_string_tables() {
        for prefix in PREFIXES {
            for opcode in 0..=255u8 {
                assert_eq!(
                    template(prefix, opcode),
                    reference(prefix, opcode, None),
                    "{prefix:?} {opcode:#04x}"
                );
            }
        }
    }

    #[test]
    fn decoder_matches_the_string_tables() {
        // Every opcode with every value of its first operand byte, and a
        // spread of second bytes where one is used (16-bit operands,
        // LD (IX+d),n).
        for prefix in PREFIXES {
            for opcode in 0..=255u8 {
                let nb = nb_operand_bytes(prefix, opcode);
                let first_bytes: Vec<u8> = if nb == 0 { vec![0x5a] } else { (0..=255).collect() };
                // DD CB d op only uses its displacement byte.
                let second_bytes: &[u8] = if nb == 2 && !matches!(prefix, OpcodePrefix::DdCb | OpcodePrefix::FdCb) {
                    &[0x00, 0x01, 0x7f, 0x80, 0xc3, 0xff]
                }
                else {
                    &[0x00, 0xff]
                };
                for &low in &first_bytes {
                    for &high in second_bytes {
                        assert_eq!(
                            with_operands(prefix, opcode, [low, high]),
                            reference(prefix, opcode, Some([low, high])),
                            "{prefix:?} {opcode:#04x} {low:#04x} {high:#04x}"
                        );
                    }
                }
            }
        }
    }

    /// The count the string tables gave: one per "nn", two per "nnnn".
    fn reference_nb_operand_bytes(prefix: OpcodePrefix, opcode: u8) -> u8 {
        if matches!(prefix, OpcodePrefix::DdCb | OpcodePrefix::FdCb) {
            return 2;
        }
        let mut repr = table(prefix)[opcode as usize];
        if repr.is_empty() && matches!(prefix, OpcodePrefix::Dd | OpcodePrefix::Fd) {
            repr = TABINSTR[opcode as usize];
        }
        // "nnnn" holds two non-overlapping "nn".
        repr.matches("nn").count() as u8
    }

    #[test]
    fn operand_byte_counts_match_the_string_tables() {
        for prefix in PREFIXES {
            for opcode in 0..=255u8 {
                assert_eq!(
                    nb_operand_bytes(prefix, opcode),
                    reference_nb_operand_bytes(prefix, opcode),
                    "{prefix:?} {opcode:#04x}"
                );
            }
        }
    }

    #[test]
    fn rst_carries_its_target_address() {
        for (opcode, address) in [(0xc7, 0x00), (0xd7, 0x10), (0xff, 0x38)] {
            assert_eq!(template(OpcodePrefix::None, opcode), op(Mnemonic::Rst, Some(value(address)), None));
        }
    }

    #[test]
    fn relative_jumps() {
        let jr = |d: u8| with_operands(OpcodePrefix::None, 0x18, [d, 0]);
        assert_eq!(jr(0xfe), op(Mnemonic::Jr, None, Some(DataAccess::Expression(Expr::Label("$".into())))));
        assert_eq!(jr(0x05), op(Mnemonic::Jr, None, Some(value(7))));
        assert_eq!(jr(0x80), op(Mnemonic::Jr, None, Some(value(-126))));
    }
}
