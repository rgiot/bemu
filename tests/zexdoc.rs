use amsrust::motherboard::{MotherboardBuilder, Motherboard};
use amsrust::z80::{HasValue, Reg16, Reg8, Z80};
use amsrust::memory::*;
use amsrust::debug::Monitor;

/// Took inspiration from https://floooh.github.io/2016/07/12/z80-rust-ms1.html
#[test]
fn zexdoc() {
    const verbose : bool = false;
    let mut motherboard = MotherboardBuilder::default().build();
    motherboard.force_rom_config(
        RomConfig{
            lower_rom: Flag::Disable,
            upper_rom: Flag::Disable,
            rom_number: 0
        }
    );
    
    
    let prg = include_bytes!("zexdoc.com");

    assert_eq!(
        0xc3,
        prg[0]
    );

    motherboard.write_buffer(0x100, prg);
    motherboard.write_byte(5, 0xc9); // need to return from cpm call
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x100);
    motherboard.cpu_mut().reg16_mut(Reg16::Sp).set(0xF000);
    motherboard.write_word(6, 0xF000); // need to return from cpm call


    assert_eq!(
        0xc3,
        motherboard.read_byte(0x100)
    );
    assert_eq!(
        0xc9,
        motherboard.read_byte(0x5)
    );

    let mut nb_instructions = 0;

    loop {
      //  dbg!(motherboard.cpu().reg16(Reg16::Pc));
        motherboard.execute_next_opcode();
        if verbose {
            Monitor::monitor(&motherboard);
        }

        assert_eq!(
            0xc9,
            motherboard.read_byte(0x5)
        );
        
        let cpu = motherboard.cpu();

        nb_instructions += 1;
        if nb_instructions == 50 {
   //         break;
        } // execute few just to test

        match cpu.reg16(Reg16::Pc).value() {
            0x0005 => { cpm_bdos(&motherboard); },
            0x0000 => { break; },
            _ => { },
        }
    }


}

// Code to execute when a call is made to CPM
fn cpm_bdos(motherboard: & Motherboard) {
    let cpu = motherboard.cpu();
    let c_value = cpu.reg8(Reg8::C).value();
    match  c_value {
        2 => {
            // output character in register E
            print!("{}", cpu.reg8(Reg8::E).value() as u8 as char);
        },
        9 => {
            // output a string at register DE until '$'
            let mut addr = cpu.reg16(Reg16::De).value();
            loop {
                let c = motherboard.read_byte(addr) as char;
                addr = addr.wrapping_add(1);
                if c != '$' {
                    print!("{}", c);
                }
                else {
                    break;
                }
            }
        },
        _ => {
            panic!("Unknown CP/M call {}!", c_value);
        }
    }
}