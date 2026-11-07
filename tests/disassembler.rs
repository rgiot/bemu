//! The debugger's disassembler (`Motherboard::disassemble_instruction_at`):
//! instruction boundaries and tokens for each prefix page, decoded from
//! the opcode bits (`amsrust::z80::decoder`).

use amsrust::motherboard::MotherboardBuilder;
use cpclib_asm::preamble::{
    BinaryOperation, DataAccess, Expr, IndexRegister16, Mnemonic, Register16, Register8, Token,
};

fn op(mnemonic: Mnemonic, arg1: Option<DataAccess>, arg2: Option<DataAccess>) -> Token {
    Token::OpCode(mnemonic, arg1, arg2, None)
}

fn value(v: i32) -> Option<DataAccess> {
    Some(DataAccess::Expression(Expr::Value(v)))
}

fn reg8(register: Register8) -> Option<DataAccess> {
    Some(DataAccess::Register8(register))
}

fn ix_plus(d: i32) -> Option<DataAccess> {
    Some(DataAccess::IndexRegister16WithIndex(
        IndexRegister16::Ix,
        BinaryOperation::Add,
        Expr::Value(d),
    ))
}

#[test]
fn disassembles_every_prefix_page_with_its_operands() {
    let program: &[(&[u8], Token)] = &[
        (
            &[0x3e, 0x12],
            op(Mnemonic::Ld, reg8(Register8::A), value(0x12)),
        ),
        (
            &[0x21, 0x34, 0x12],
            op(
                Mnemonic::Ld,
                Some(DataAccess::Register16(Register16::Hl)),
                value(0x1234),
            ),
        ),
        (&[0x18, 0x05], op(Mnemonic::Jr, None, value(7))),
        (
            &[0xcb, 0x7e],
            op(
                Mnemonic::Bit,
                value(7),
                Some(DataAccess::MemoryRegister16(Register16::Hl)),
            ),
        ),
        (&[0xed, 0xb0], op(Mnemonic::Ldir, None, None)),
        (
            &[0xed, 0x43, 0x00, 0xc0],
            op(
                Mnemonic::Ld,
                Some(DataAccess::Memory(Expr::Value(0xc000))),
                Some(DataAccess::Register16(Register16::Bc)),
            ),
        ),
        (
            &[0xdd, 0x36, 0x05, 0x99],
            op(Mnemonic::Ld, ix_plus(5), value(0x99)),
        ),
        (
            &[0xdd, 0x41],
            op(Mnemonic::Ld, reg8(Register8::B), reg8(Register8::C)),
        ),
        (
            &[0xdd, 0xcb, 0x03, 0xc6],
            Token::OpCode(Mnemonic::Set, value(0), ix_plus(3), None),
        ),
        (&[0xd7], op(Mnemonic::Rst, value(0x10), None)),
    ];

    let mut motherboard = MotherboardBuilder::default()
        .disable_lower_rom()
        .disable_upper_rom()
        .build();
    let bytes: Vec<u8> = program
        .iter()
        .flat_map(|(bytes, _)| bytes.iter().copied())
        .collect();
    motherboard.write_buffer(0x4000, &bytes);

    let disassembled = motherboard.disassemble_instructions_from(0x4000, program.len());
    let mut address = 0x4000u16;
    for (instruction, (bytes, token)) in disassembled.iter().zip(program) {
        assert_eq!(instruction.address, address);
        assert_eq!(&instruction.memory[..], *bytes, "{}", instruction);
        assert_eq!(&instruction.token, token, "{}", instruction);
        address += bytes.len() as u16;
    }
}
