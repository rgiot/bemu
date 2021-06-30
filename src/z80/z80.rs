use crate::chip::*;

use crate::z80::arithmetic::rla;
use crate::z80::arithmetic::rlc;
use crate::z80::arithmetic::rr;
use crate::z80::arithmetic::rrc;
use crate::z80::arithmetic::set_bit;
use crate::z80::arithmetic::sl1;
use crate::z80::arithmetic::sla;
use crate::z80::arithmetic::sra;
use crate::z80::arithmetic::srl;
use crate::z80::registers::*;
use crate::z80::registers_identifiers::*;
use crate::z80::states::*;

use std::fmt::Debug;

use cpclib_asm::preamble::{
    DataAccess, ExprEvaluationExt, FlagTest, Mnemonic, SymbolsTableCaseDependent,
};

macro_rules! inner_8bits_sub {
    ($a:ident,$b:ident,$f:ident) => {{
        let (res, ovf) = $a.overflowing_sub($b);
        
        Z80Flags::auto_update(
            &[Z80Flags::S, Z80Flags::Z, Z80Flags::X, Z80Flags::Y],
            $f,
            res,
        );
        
        Z80Flags::C.set(ovf, $f);
        Z80Flags::N.set_off($f);
        // TODO manage signed stuff
        res
    }};
}

// http://www.z80.info/zinout.htm#Z80CPU
#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u64)]
pub enum Z80Pin {
    A11 = 1 << 0,
    A12 = 1 << 1,
    A13 = 1 << 2,
    A14 = 1 << 3,
    A15 = 1 << 4,
    Neg_CLK = 1 << 5,
    /// Data bus in/out
    D4 = 1 << 6,
    /// Data bus in/out
    D3 = 1 << 7,
    /// Data bus in/out
    D5 = 1 << 8,
    /// Data bus in/out
    D6 = 1 << 9,
    VCC = 1 << 10,
    /// Data bus in/out
    D2 = 1 << 11,
    /// Data bus in/out
    D7 = 1 << 12,
    /// Data bus in/out
    D0 = 1 << 13,
    /// Data bus in/out
    D1 = 1 << 14,
    Neg_INT = 1 << 15,
    Neg_NMI = 1 << 16,
    Neg_HALT = 1 << 17,
    Neg_MREQ = 1 << 18,
    Neg_IORQ = 1 << 19,
    A10 = 1 << 20,
    A9 = 1 << 21,
    A8 = 1 << 22,
    A7 = 1 << 23,
    A6 = 1 << 24,
    A5 = 1 << 25,
    A4 = 1 << 26,
    A3 = 1 << 27,
    A2 = 1 << 28,
    A1 = 1 << 29,
    A0 = 1 << 30,
    GND = 1 << 31,
    Neg_RFSH = 1 << 32,
    Neg_M1 = 1 << 33,
    Neg_RESET = 1 << 34,
    Neg_BUSRQ = 1 << 35,
    Neg_WAIT = 1 << 36,
    Neg_BUSACK = 1 << 37,
    Neg_WR = 1 << 38,
    Neg_RD = 1 << 39,
}

impl Into<u64> for &Z80Pin {
    fn into(self) -> u64 {
        *self as u64
    }
}

impl Into<u64> for Z80Pin {
    fn into(self) -> u64 {
        self as u64
    }
}

/// Set of Z80Pin for the data bus
const Z80_PIN_DATA_BUS: [Z80Pin; 8] = [
Z80Pin::D7,
Z80Pin::D6,
Z80Pin::D5,
Z80Pin::D4,
Z80Pin::D3,
Z80Pin::D2,
Z80Pin::D1,
Z80Pin::D0,
];

/// Set of Z80Pin for the address bus
const PIN_ADDRESS_BUS: [Z80Pin; 16] = [
Z80Pin::A15,
Z80Pin::A14,
Z80Pin::A13,
Z80Pin::A12,
Z80Pin::A11,
Z80Pin::A10,
Z80Pin::A9,
Z80Pin::A8,
Z80Pin::A7,
Z80Pin::A6,
Z80Pin::A5,
Z80Pin::A4,
Z80Pin::A3,
Z80Pin::A2,
Z80Pin::A1,
Z80Pin::A0,
];

const PIN_SYSTEM_CONTROL: [Z80Pin; 5] = [
Z80Pin::Neg_M1,
Z80Pin::Neg_MREQ,
Z80Pin::Neg_IORQ,
Z80Pin::Neg_RD,
Z80Pin::Neg_WR,
];

const PIN_CPU_CONTROL: [Z80Pin; 5] = [
Z80Pin::Neg_HALT,
Z80Pin::Neg_WAIT,
Z80Pin::Neg_INT,
Z80Pin::Neg_NMI,
Z80Pin::Neg_RESET,
];

const PIN_CPU_BUS_CONTROL: [Z80Pin; 2] = [Z80Pin::Neg_BUSRQ, Z80Pin::Neg_BUSACK];

#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Z80Flags {
    // Set if the 2-complement value is negative. It’s simply a copy of the most significant bit.
    S = 1 << 7,
    // Set if the result is zero.
    Z = 1 << 6,
    // A copy of bit 5 of the result
    Y = 1 << 5,
    // The half-carry of an addition/subtraction (from bit 3 to 4). Needed for BCD correctionwithDAA.
    H = 1 << 4,
    // A copy of bit 3 of the result
    X = 1 << 3,
    // This flag can either be the parity of the result (PF), or the2-compliment signed overflow (VF): set if 2-compliment value doesn’t fit in the register
    P = 1 << 2,
    // Shows whether the last operation was an addition (0) or an subtraction (1).  Thisinformation is needed forDAA.
    N = 1 << 1,
    // The carry flag, set if there was a carry after the most significant bit.
    C = 1 << 0,
}

impl Z80Flags {
    /// Properly set the flag to the given byte
    #[inline]
    pub fn set(&self, value: bool, byte: &mut u8) {
        let mask = *self as u8;
        
        if value {
            *byte = *byte | mask;
        } else {
            *byte = *byte & !mask;
        }
    }
    
    /// activate the flag
    #[inline]
    pub fn set_on(&self, byte: &mut u8) {
        self.set(true, byte);
    }
    
    /// deactivate the flag
    #[inline]
    pub fn set_off(&self, byte: &mut u8) {
        self.set(false, byte);
    }
    
    /// Check if the flag is set
    #[inline]
    pub fn is_on(&self, byte: u8) -> bool {
        let mask = *self as u8;
        
        byte & mask != 0
    }
    
    #[inline]
    pub fn is_off(&self, byte: u8) -> bool {
        !self.is_on(byte)
    }
    
    #[inline]
    /// Update using the standard rule for the given flags
    pub fn auto_update_from(&self, byte: &mut u8, res: u8) {
        match self {
            Self::S => self.set(res & 1 << 7 != 0, byte),
            Self::X => self.set(res & 1 << 3 != 0, byte),
            Self::Y => self.set(res & 1 << 5 != 0, byte),
            Self::Z => self.set(res == 0, byte),
            Self::P => self.set(res.count_ones() % 2 == 0, byte),
            
            _ => unreachable!(),
        }
    }
    
    
    #[inline]
    /// Update using the standard rule for the given flags
    pub fn auto_update16_from(&self, byte: &mut u8, res: u16) {
        match self {
            Self::S => self.set(res & 1 << 15 != 0, byte),
            Self::X => self.set(res & 1 << 3 != 0, byte), // XXX probably wrong
            Self::Y => self.set(res & 1 << 5 != 0, byte), // XXX probably wrong
            Self::Z => self.set(res == 0, byte),
            Self::P => self.set(res.count_ones() % 2 == 0, byte),
            
            _ => unreachable!(),
        }
    }
    
    
    #[inline]
    /// Update using the standard rule of each provided flag
    pub fn auto_update(flags: &[Self], byte: &mut u8, res: u8) {
        flags.iter().for_each(|f| f.auto_update_from(byte, res));
    }
    
    pub fn auto_update16(flags: &[Self], byte: &mut u8, res: u16) {
        flags.iter().for_each(|f| f.auto_update16_from(byte, res));
    }
}

#[derive(Debug, PartialEq)]
pub enum InterruptMode {
    Mode0,
    Mode1,
    Mode2,
}
impl Default for InterruptMode {
    fn default() -> Self {
        Self::Mode1
    }
}

#[derive(Default, Debug)]
/// https://gist.github.com/drhelius/8497817
pub struct Z80 {
    /// Encoding of the 40 Z80Pin
    pins: PinsBuffer,
    /// State of the Z80 state-machine
    pub(crate) next_state: MCycle,
    
    pub(crate) pc: Register16,
    pub(crate) sp: Register16,
    
    pub(crate) af: Register16,
    pub(crate) af2: Register16,
    
    pub(crate) hl: Register16,
    pub(crate) bc: Register16,
    pub(crate) de: Register16,
    
    pub(crate) hl2: Register16,
    pub(crate) bc2: Register16,
    pub(crate) de2: Register16,
    
    pub(crate) ix: Register16,
    pub(crate) iy: Register16,
    pub(crate) wz: Register16,
    
    pub(crate) i: Register8,
    pub(crate) r: Register8,
    
    /// Instruction Register / contains the opcode of interest
    pub(crate) ir: Register8,
    
    pub(crate) iff1: bool,
    pub(crate) iff2: bool,
    pub(crate) im: InterruptMode,
    
    
    /// Refresh adr
    /// XXX Not handled at all by the emulator
    pub(crate) refresh_addr: u16,
    
    /// Information for the current opcode - Setup after fetching the very first byte
    pub(crate) opcode_information: Option<&'static OpcodeInformation>,

    /// Some instruction using ix and read & write need to have a correction of timing
    /// Note that it is un unrealistic workaround as the lost timing appears BEFORE the read
    expect_additional_fake_cycle: u8
}

/// Flags related code
impl Z80 {
    /// Update qll the flags after an add/substract operation
    #[inline]
    fn update_all_flags(
        &mut self,
        res: u8,
        half_carry: bool,
        pv: bool,
        overflow: bool,
        substraction: bool,
    ) {
        let f = &mut self.af.low_mut().val;
        
        Z80Flags::S.set((res & 1 << 7) != 0, f);
        Z80Flags::Z.set(res == 0, f);
        Z80Flags::Y.set((res & 1 << 5) != 0, f);
        Z80Flags::H.set(half_carry, f);
        Z80Flags::Y.set((res & 1 << 3) != 0, f);
        Z80Flags::P.set(pv, f);
        Z80Flags::N.set(substraction, f);
        Z80Flags::C.set(overflow, f);
    }
    
    /// Check if the flag test is successfull
    #[inline]
    pub fn test_flag(&self, flag_test: FlagTest) -> bool {
        let flag = self.reg8(Reg8::F).value();
        match flag_test {
            FlagTest::NZ => Z80Flags::Z.is_off(flag),
            FlagTest::Z => Z80Flags::Z.is_on(flag),
            
            FlagTest::NC => Z80Flags::C.is_off(flag),
            FlagTest::C => Z80Flags::C.is_on(flag),
            
            FlagTest::PO => Z80Flags::P.is_off(flag),
            FlagTest::PE => Z80Flags::P.is_on(flag),
            
            FlagTest::P => Z80Flags::S.is_off(flag),
            FlagTest::M => Z80Flags::S.is_on(flag),
        }
    }
    
    #[inline]
    fn set_flag(&mut self, flag: Z80Flags, val: bool) {
        let f = &mut self.reg8_mut(Reg8::F).val;
        flag.set(val, f);
    }
    
    #[inline]
    fn set_flag_on(&mut self, flag: Z80Flags) {
        self.set_flag(flag, true);
    }
    
    #[inline]
    fn set_flag_off(&mut self, flag: Z80Flags) {
        self.set_flag(flag, false);
    }
    
    #[inline]
    pub fn flag(&self, flag: Z80Flags) -> bool {
        let flags = self.reg8(Reg8::F).value();
        flag.is_on(flags)
    }
    
    #[inline]
    fn auto_update_flags(&mut self, val: u8) {
        let f = &mut self.reg8_mut(Reg8::F).val;
        Z80Flags::auto_update(&[
            Z80Flags::S,
            Z80Flags::Z,
            Z80Flags::P,
            Z80Flags::X,
            Z80Flags::Y,
            ], f, val);
        }
    }
    
    /// Conditionnaly related code
    impl Z80 {
        /// instruction is allowed if it is not conditional or if condition is met
        fn is_condition_met(&self) -> bool {
            let info = self.opcode_information().unwrap();
            match info.flag_test {
                Some(f) => self.test_flag(f),
                None => true,
            }
        }
    }
    
    impl Z80 {
        /// Check if at the next tick the CPU will start the very beginning of an instruction
        pub fn is_aligned_with_new_instruction(&self) -> bool {
            self.next_state.is_new_opcode()
        }
        
        /// Check if some operations has to be handled in the middle of the instructions (i.e. read / operation / write)
        fn must_execute_instruction_in_middle(&self) -> bool {
            self.opcode_information().unwrap()
            .compute_between_read_write()
            
        }
        
        /// Provide the information for the current instruction to execute
        fn opcode_information(&self) -> Option<&OpcodeInformation> {
            self.opcode_information.clone()
        }
        /// Get the adress where to write based on what as been collected at fetch step
        fn get_write_address_from_opcode_information(&self) -> u16 {
            self.get_address_from_address_source(
                &self.opcode_information.as_ref().unwrap().write_address,
            )
        }
        
        /// During the fetch cycle, the CPU collected the information to obtain the address to read/write
        /// This method obtains it
        fn get_read_address_from_opcode_information(&self) -> u16 {
            self.get_address_from_address_source(
                &self.opcode_information.as_ref().unwrap().read_address,
            )
        }
        
        /// src encodes the place where to find an address. This fonction directly returns the corresponding address
        fn get_address_from_address_source(&self, src: &AddressSource) -> u16 {
            match src {
                // Impossible case
                AddressSource::None => unreachable!(),
                // As already been loaded in Wz
                AddressSource::Immediate => self.reg16(Reg16::Wz).get(),
                // Is stored withing a 16 bits register
                AddressSource::Register16(ref reg) | AddressSource::IndexRegister16(ref reg) => {
                    self.reg16(*reg).get()
                }
                // Corresponds to an indexed address
                AddressSource::Indexed(ref reg) => {
                    // by definition, delta is within the first byte
                    let delta = self.reg8(Reg8::Z).get();
                    let base = self.reg16(*reg).get();
                    
                    let addr = if delta & 1 << 7 != 0 {
                        let delta = (!delta).wrapping_add(1); // get positive delta
                        base.wrapping_sub(delta as _)
                    }
                    else  {
                        base.wrapping_add(delta as _)
                    };
                    addr
                }
                // We want to make an operation on the stack
                AddressSource::Stack(delta) => {
                    let stack = self.sp.value();
                    match delta {
                        StackDelta::Zero => stack,
                        StackDelta::MinusTwo => stack.wrapping_sub(2),
                    }
                }
            }
        }
        
        /// Returns the byte that needs to be written
        fn get_byte_to_write_from_opcode_information(&self, step: u8) -> u8 {
            self.get_byte_from_data_source(&self.opcode_information.as_ref().unwrap().write_data, step)
        }
        
        /// Returns the appropriate byte
        fn get_byte_from_data_source(&self, src: &DataSource, step: u8) -> u8 {
            // Retreive the 16 bits value
            let value = match src {
                DataSource::Register8(reg) => {
                    assert_eq!(step, 0);
                    self.reg8(*reg).get() as u16
                }
                DataSource::Pins => {
                    assert_eq!(step, 0);
                    self.get_data_from_pins() as u16
                }
                DataSource::Register16(reg) | DataSource::IndexRegister16(reg) => {
                    self.reg16(*reg).value()
                }
                DataSource::Immediate => self.reg16(Reg16::Wz).value(),
                _ => unimplemented!("{:?} / {:?}", src, self.opcode_information),
            };
            
            let mut first = (value & 0xff) as u8;
            let mut second = ((value & 0xff00) >> 8) as u8;
            
            if self.opcode_information().unwrap().write_on_stack() {
                std::mem::swap(&mut first, &mut second);
            }
            // And output the appropriate 8 bits one
            if step == 0 {
                first
            } else if step == 1 {
                second
            } else {
                unreachable!()
            }
        }
        
        /// CPU has read a value and must store it directly in the right registers
        /// Warning when the recipients are PC or SP, Wz is used instead
        fn store_currently_read_byte(&mut self, byte: u8, nb: u8) {
            let dst = self.opcode_information().unwrap().read_data;
            
            match dst {
                DataSource::Register8(reg) | DataSource::IndexRegister8(reg) => {
                    assert!(nb == 0);
                    self.reg8_mut(reg).set(byte);
                    
                    //      dbg!(reg, self.reg8_mut(reg));
                }
                DataSource::Register16(reg) | DataSource::IndexRegister16(reg) => {
                    let reg = if reg == Reg16::Pc || reg == Reg16::Sp {
                        Reg16::Wz
                    } else {
                        reg
                    };
                    let reg = if nb == 0 { reg.low() } else { reg.high() };
                    self.reg8_mut(reg).set(byte);
                }
                DataSource::Pins => {
                    assert_eq!(nb, 0);
                    self.reg8_mut(Reg8::Z).set(byte); // not sure if it is the case in real world, but at least it make work some instructions
                }
                _ => {
                    unimplemented!("{:?}/ {:?}", dst, self.opcode_information)
                }
            }
        }
        
        /// Execute the instruction ; all necessary stuffs has already been collected
        fn execute_step(&mut self) {
            assert!(
                !matches!(
                    self.next_state,
                    MCycle::Fake(_, _)
                ),
                "We never call execute_state after fake timing"
            );
            
            
            {
                // this is the normal behavior
                // we may add additional duration
                let prefix = &self.opcode_information.as_ref().unwrap().prefix;
                
                // Get the two additional bytes needed for the instruction
                let memory = [self.reg8(Reg8::Z).get(), self.reg8(Reg8::W).get()];
                
                // By construction, it cannot fail
                let (opcode, rest) = cpclib_asm::disass::disassemble_with_potential_argument(
                    self.ir.get(),
                    &prefix.disassembling_table(),
                    &memory,
                )
                .unwrap();
                assert!(opcode.is_opcode());
                self.execute_opcode(
                    opcode.mnemonic().unwrap(),
                    opcode.mnemonic_arg1(),
                    opcode.mnemonic_arg2(),
                );
                
                // reset the various buffers
                if !self.must_execute_instruction_in_middle() {
                    self.opcode_information = None;
                }
            }
            
            if matches!(
                self.next_state,
                MCycle::Fake(TState::T1, _)
            ) {
                // we do nothing because it is not the end of an instruction
                // indeed we have artificially added time
            }
            else {
                // here it is the normal behavior
                // so we can test if there is an interruption
                self.instruction_finished();
            }
            
            
        }
        
        /// Check how many bytes have to be read yet and set up appropriately the next state
        fn decode_step(&mut self) -> bool {
            // Check if current opcode is a prefix or a real one
            // In case it is a prefix, we redo a Fetch state from scratch
            let current_prefix = self.next_state.prefix();
            match self.ir.get() {
                // For prefixed instructions, we have to restart the process from scratch
                0xdd => {
                    self.next_state = MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::Dd);
                    assert!(current_prefix == OpcodePrefix::None, "impossible");
                    return false;
                }
                0xfd => {
                    self.next_state = MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::Fd);
                    assert!(current_prefix == OpcodePrefix::None, "impossible");
                    
                    return false;
                }
                0xcb => {
                    match current_prefix {
                        OpcodePrefix::None => {
                            self.next_state = MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::Cb);
                        },
                        OpcodePrefix::Dd => {
                            self.next_state = MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::DdCb);
                        },
                        OpcodePrefix::Fd => {
                            self.next_state = MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::FdCb);
                        },
                        _ => unreachable!()
                    };
                    return false;
                }
                0xed => {
                    self.next_state = MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::Ed);
                    assert!(current_prefix == OpcodePrefix::None, "Not implemented yet");
                    return false;
                }
                
                _ => {
                    // Indexed opcodes  need to be exchanged
                    if current_prefix.is_for_indexed_opcode() {
                        let z = self.reg8(Reg8::Z).value();
                        let ir = self.ir.value();
                        self.reg8_mut(Reg8::Z).set(ir);
                        self.ir.set(z);
                    }

                    // Get the number of additional bytes to read as well as the information for the next cycles
                    self.opcode_information = Some(current_prefix.opcode_information(self.ir.get()));
                    let opcode_information = self.opcode_information.as_ref().unwrap();
                    
                   //dbg!(opcode_information);


                    if opcode_information.fetch_finished() || current_prefix.is_for_indexed_opcode() {
                        if opcode_information.expect_read() {
                            if opcode_information.read_from_pins {
                                self.next_state = MCycle::Input(TState::T1, 0);
                            } else {
                                self.next_state = MCycle::Read(TState::T1, 0);
                            }
                            return false;
                        } else if opcode_information.expect_write() {
                            if opcode_information.write_to_pins {
                                self.next_state = MCycle::Output(TState::T1, 0);
                            } else {
                                self.next_state = MCycle::Write(TState::T1, 0);
                            }
                            return false;
                        } else {
                            // Start a new opcode
                            self.next_state = MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None);
                            return true;
                        }
                    } {
                        // continue to read the bytes for the current opcode
                        self.next_state = MCycle::FetchOpcode(TState::T1, 1, current_prefix);
                        return false;
                    } 
                }
            }
        }
        
        /// Execute the end of the instruction  (everything related to memory or ports has already been handled for most instruction except for thouse that read and write from memory).
        /// Here we take care of registers modifications and flags configuration
        /// We can consider that here we play the latest t-state
        fn execute_opcode(
            &mut self,
            mnemonic: &Mnemonic,
            arg1: Option<&DataAccess>,
            arg2: Option<&DataAccess>,
        ) {
            match mnemonic {
                Mnemonic::Nop | Mnemonic::Out | Mnemonic::In => {}
                
                Mnemonic::Adc => {
                    match arg1.unwrap() {
                        DataAccess::Register8(cpclib_asm::preamble::Register8::A) => {
                            // this MUST be A register
                            // 8bits version
                            let mut right = self.get_available_8bits_value(arg2.unwrap());
                            if self.flag(Z80Flags::C) {
                                right += 1;
                            }
                            let ovf = self.reg8_mut(Reg8::A).add(right); 
                            // TODO manage the flags
                            self.set_flag(Z80Flags::S, self.a() & 1 << 7 != 0);
                            self.set_flag(Z80Flags::Z, self.a() == 0);
                            self.set_flag(Z80Flags::C, ovf);
                            self.set_flag_off(Z80Flags::N);
                        },
                        
                        DataAccess::Register16(cpclib_asm::preamble::Register16::Hl)=> {
                            let mut right = self.get_available_16bits_value(arg2.unwrap());
                            if self.flag(Z80Flags::C) {
                                right += 1;
                            }
                            let ovf = self.reg16_mut(Reg16::Hl).add(right);
                            self.set_flag(Z80Flags::S, self.hl() & 1 << 15 != 0);
                            self.set_flag(Z80Flags::Z, self.hl() == 0);
                            self.set_flag(Z80Flags::C, ovf);
                            self.set_flag_off(Z80Flags::N);

                            let extra_cycles = 2;
                            self.next_state = MCycle::new_fake(extra_cycles);
                        }
                        
                        _ =>  unimplemented!()
                    }
                }
                
                
                
                Mnemonic::Add => {
                    let involve_indexes = matches!(arg1, Some(DataAccess::IndexRegister16WithIndex(_, _))) | matches!(arg2, Some(DataAccess::IndexRegister16WithIndex(_, _))
                    );

                    match arg1.unwrap() {
                        DataAccess::Register8(cpclib_asm::preamble::Register8::A) => {
                            // this MUST be A register
                            // 8bits version
                            let right = self.get_available_8bits_value(arg2.unwrap());
                            let ovf = self.reg8_mut(Reg8::A).add(right); 
                            // TODO manage the flags
                            self.set_flag(Z80Flags::C, ovf);
                        }
                        
                        DataAccess::Register16(_) | DataAccess::IndexRegister16(_)  => {
                            let reg: Reg16 = match arg1.unwrap() {
                                DataAccess::Register16(reg) => reg.into(),
                                DataAccess::IndexRegister16(reg) => reg.into(),
                                _ => unimplemented!()
                            };
                            let right = self.get_available_16bits_value(arg2.unwrap());
                            let mut left = self.reg16_mut(reg);
                            
                            let ovf = left.add(right); 
                            // TODO manage the other flags
                            self.set_flag(Z80Flags::C, ovf);

                            let extra_cycles = 2;
                            self.next_state = MCycle::new_fake(extra_cycles);
                        }
                        
                        _ => unimplemented!(),
                    }

                    if involve_indexes {
                        self.next_state = MCycle::new_fake(1);
                    }
                }
                
                Mnemonic::And => {
                    let mut  val = self.get_available_8bits_value(arg1.unwrap());
                    val = val & self.reg8(Reg8::A).value();
                    self.reg8_mut(Reg8::A).set(val);
                    
                    self.auto_update_flags(val);
                    self.set_flag_on(Z80Flags::H);
                    self.set_flag_off(Z80Flags::N);
                    self.set_flag_off(Z80Flags::C);
                }
                
                Mnemonic::Bit => {
                    let val = self.get_available_8bits_value(arg2.unwrap());
                    let bit = val & 1 << arg1.unwrap().expr().unwrap().eval().unwrap() != 0;
                    
                    self.set_flag_on(Z80Flags::H);
                    self.set_flag(Z80Flags::Z, bit);
                    self.set_flag_on(Z80Flags::N);
                }
                
                Mnemonic::Call => {
                    #[cfg(test)]
                    {
                        // conditional call where condition is not met MUST not arrive there
                        if arg1.is_some() {
                            assert!(self.is_condition_met());
                        }
                    }
                    
                    let addr = match arg2 {
                        Some(DataAccess::Expression(exp)) => exp.eval().unwrap() as u16,
                        
                        _ => unreachable!(),
                    };
                    
                    self.pc.set(addr);
                    self.sp.dec();
                    self.sp.dec();
                }
                
                
                Mnemonic::Ccf => {
                    let c = self.flag(Z80Flags::C);
                    self.set_flag(Z80Flags::C, !c);
                    self.set_flag_off(Z80Flags::N);
                }

                Mnemonic::Cp => {
                    let left = self.reg8(Reg8::A).value();
                    let right = self.get_available_8bits_value(arg1.unwrap());
                    let f = &mut self.reg8_mut(Reg8::F).val;
                    inner_8bits_sub!(left, right, f);
                    // override bit 5 and 3
                    Z80Flags::X.auto_update_from(f, right);
                    Z80Flags::Y.auto_update_from(f, right);
                }
                
                Mnemonic::Cpi | Mnemonic::Cpd => {
                    let left = self.reg8(Reg8::A).value();
                    let right = self.get_data_from_pins();
                    let f = &mut self.reg8_mut(Reg8::F).val;
                    inner_8bits_sub!(left, right, f);
                    Z80Flags::X.auto_update_from(f, right);
                    Z80Flags::Y.auto_update_from(f, right);
                    
                    if *mnemonic == Mnemonic::Cpi || *mnemonic == Mnemonic::Cpir {
                        self.hl.inc();
                    }
                    else {
                        self.hl.dec();
                    }
                    self.bc.dec();
                    self.set_flag(Z80Flags::P, self.bc() == 0);
                    
                    let jmp = if *mnemonic == Mnemonic::Cpi || *mnemonic == Mnemonic::Cpd {
                        false
                    } else {
                        self.bc() != 0 || self.flag(Z80Flags::Z)
                    };
                    
                    if jmp {
                        self.next_state = MCycle::new_fake(2);
                        self.pc.dec();
                        self.pc.dec();
                    }
                    else {
                        self.next_state = MCycle::new_fake(1);
                    }
                }
                
                
                Mnemonic::Cpl => {
                    self.reg8_mut(Reg8::A).cpl();
                    self.set_flag_on(Z80Flags::H);
                    self.set_flag_on(Z80Flags::N);
                    
                }

                Mnemonic::Daa => {
                    let low = self.a()&0x0f;
                    let high = (self.a()&0xf0)>>4;


                    let (diff, c) = match (
                        self.flag(Z80Flags::N),
                        self.flag(Z80Flags::C),
                        high,
                        self.flag(Z80Flags::H),
                        low
                    ) {
                        //1
                        (
                            false,
                            false,
                            0..=9,
                            false,
                            0..=9
                        ) => (0x00, false),

                        // 2
                        (
                            false,
                            false,
                            0..=8,
                            false,
                            0xa..=0xf
                        ) => (0x06, false),

                        // 3
                        (
                            false,
                            false,
                            0..=9,
                            true,
                            0..=3
                        ) => (0x06, false),
                        
                        // 4
                        (
                            false,
                            false,
                            0xa..=0xf,
                            false,
                            0x0..=0x9
                        ) => (0x60, true),

                        // 5
                        (
                            false,
                            false,
                            0x9..=0xf,
                            false,
                            0x1..0xf
                        ) => (0x66, true),

                        // 6
                        (
                            false,
                            false,
                            0xa..=0xf,
                            true,
                            0x0..=0x3
                        ) => (0x66, true),

                        //7
                        (
                            false,
                            true,
                            0x0..=0x2,
                            false,
                            0x0..=0x9
                        ) => (0x60, true),

                        //8
                        (
                            false,
                            true,
                            0x0..=0x2,
                            false,
                            0xa..=0xf
                        ) => (0x66, true),

                        // 9
                        (
                            false,
                            true,
                            0x0..=0x3,
                            true,
                            0x0..=0x3
                        ) => (0x66, true),

                        //1
                        (
                            true,
                            false,
                            0x0..=0x9,
                            false,
                            0x0..=0x9
                        ) => (0x00, false),

                        //2
                        (
                            true,
                            false,
                            0x0..=0x8,
                            true,
                            0x6..=0xf
                        ) => (0xFA, false),

                        //3
                        (
                            true,
                            true,
                            0x7..=0xf,
                            false,
                            0x0..=0x9
                        ) => (0xa0, true),

                        //4
                        (
                            true,
                            true,
                            0x6..=0xf,
                            true,
                            0x6..=0xf
                        ) => (0x9a, true),
                        _ => unreachable!()
                    };


                    self.reg8_mut(Reg8::A).add(diff);
                    self.set_flag(Z80Flags::C, c);
                    self.auto_update_flags(self.a());
                }

                Mnemonic::Dec => {
                    let mut update_flags = true; // disable only for 16 bits
                    let mut res = 0;
                    match arg1.unwrap() {
                        DataAccess::Register8(reg) => {
                            let left: Reg8 = reg.into();
                            self.reg8_mut(left).dec();
                            res = self.reg8(left).value();
                        }
                        DataAccess::IndexRegister8(reg) => {
                            let left: Reg8 = reg.into();
                            self.reg8_mut(left).dec();
                            res = self.reg8(left).value();
                        }
                        DataAccess::Register16(reg) => {
                            let left: Reg16 = reg.into();
                            self.reg16_mut(left).dec();
                            update_flags = false;
                        }
                        DataAccess::IndexRegister16(reg) => {
                            let left: Reg16 = reg.into();
                            self.reg16_mut(left).dec();
                            update_flags = false;
                        }
                        DataAccess::MemoryRegister16(_) | DataAccess::IndexRegister16WithIndex(_,_) => {
                            // executed in the middle
                            let  left = self.get_data_from_pins();
                            res = left.wrapping_sub(1);
                            self.set_data_on_pins(res);
                        }
                        _ => unreachable!()
                    }
                    
                    if matches!(arg1, Some(DataAccess::IndexRegister16WithIndex(_, _))) {
                        self.expect_additional_fake_cycle = 1;
                    }
                    else if matches!(arg1, Some(DataAccess::Register16(_) | DataAccess::IndexRegister16(_))) {
                        self.next_state = MCycle::new_fake(1);
                    }

                    if update_flags {
                        self.auto_update_flags(res);
                        self.set_flag_off(Z80Flags::H);//false
                        self.set_flag_on(Z80Flags::N);
                        
                    }
                }
                
                Mnemonic::Di => {
                    self.iff1 = false;
                    self.iff2 = false;
                }
                
                Mnemonic::Djnz => {
                    self.reg8_mut(Reg8::B).dec();
                    if self.reg8(Reg8::B).value() != 0 {
                        let mut sym = SymbolsTableCaseDependent::default();
                        sym.set_symbol_to_value("$", -2);
                        
                        let expr = arg1.unwrap().expr().unwrap();
                        let displacement = expr.relative_delta().unwrap();
                        if displacement > 0 {
                            self.pc.add(displacement as u16);
                        } else {
                            self.pc.sub(-displacement as u16);
                        }
                    }
                }
                
                Mnemonic::Ei => {
                    self.iff1 = true;
                    self.iff2 = true;
                }
                
                Mnemonic::ExAf => {
                    std::mem::swap(&mut self.af, &mut self.af2);
                }
                
                Mnemonic::ExHlDe => {
                    std::mem::swap(&mut self.hl, &mut self.de);
                }
                
                Mnemonic::ExMemSp => {
                    let reg = match arg1 {
                        Some(DataAccess::Register16(reg)) => reg.into(),
                        Some(DataAccess::IndexRegister16(reg)) => reg.into(),
                        _ => unimplemented!()
                    };
                    let val = self.wz.value();
                    self.reg16_mut(reg).set(val);
                    self.next_state = MCycle::new_fake(1);
                }
                
                Mnemonic::Exx => {
                    std::mem::swap(&mut self.bc, &mut self.bc2);
                    std::mem::swap(&mut self.de, &mut self.de2);
                    std::mem::swap(&mut self.hl, &mut self.hl2);
                }

                Mnemonic::Halt => {
                    if self.get_pin(Z80Pin::Neg_INT) && self.iff1 {
                        // do nothing we hit the interruption
                    } else {
                        self.pc.dec(); // wait
                    }
                }
                
                Mnemonic::Inc => {
                    let mut update_flags = true; // disable only for 16 bits
                    let mut res = 0;
                    match arg1.unwrap() {
                        DataAccess::MemoryRegister16(_) | DataAccess::IndexRegister16WithIndex(_, _)=> {
                            // read and write directly in pins (shoudl be launched before writting the results)
                            let left = self.get_data_from_pins();
                            let left = left.wrapping_add(1);
                            self.set_data_on_pins(left);
                            
                            assert_eq!(
                                self.opcode_information().unwrap().read_data,
                                DataDestination::Pins
                            );
                            assert_eq!(
                                self.opcode_information().unwrap().write_data,
                                DataDestination::Pins
                            );
                        }
                        DataAccess::Register8(reg) => {
                            let left: Reg8 = reg.into();
                            self.reg8_mut(left).inc();
                            res = self.reg8(left).value();
                        }
                        DataAccess::IndexRegister8(reg) => {
                            let left: Reg8 = reg.into();
                            self.reg8_mut(left).inc();
                            res = self.reg8(left).value();
                        }
                        DataAccess::Register16(reg) => {
                            let left: Reg16 = reg.into();
                            self.reg16_mut(left).inc();
                            update_flags = false;

                        }
                        DataAccess::IndexRegister16(reg) => {
                            let left: Reg16 = reg.into();
                            self.reg16_mut(left).inc();
                            update_flags = false;
                        }
                        _ => unimplemented!(),
                    }

                    if matches!(arg1, Some(DataAccess::IndexRegister16WithIndex(_, _))) {
                        self.expect_additional_fake_cycle = 1;
                    }
                    else if matches!(arg1, Some(DataAccess::Register16(_) | DataAccess::IndexRegister16(_))) {
                        self.next_state = MCycle::new_fake(1);
                    }
                    
                    if update_flags {
                        if update_flags {
                            self.auto_update_flags(res);
                            self.set_flag_off(Z80Flags::H);//false
                            self.set_flag_off(Z80Flags::N);
                            
                        }
                    }
                }
                
                Mnemonic::Jp => {
                    let do_jump = match arg1 {
                        None => true,
                        Some(DataAccess::FlagTest(flag_test)) => self.test_flag(*flag_test),
                        _ => unreachable!(),
                    };
                    
                    if do_jump {
                        let adr = match arg2.unwrap() {
                            DataAccess::Expression(expr) => expr.eval().unwrap() as u16,
                            
                            DataAccess::MemoryRegister16(reg) => self.reg16(reg.into()).value(),
                            
                            DataAccess::MemoryIndexRegister16(reg) => self.reg16(reg.into()).value(),
                            
                            _ => unreachable!(),
                        };
                        
                        self.pc.set(adr);
                    }
                }
                
                Mnemonic::Jr => {
                    let do_jump = match arg1 {
                        None => true,
                        Some(DataAccess::FlagTest(flag_test)) => self.test_flag(*flag_test),
                        _ => unreachable!(),
                    };
                    
                    if do_jump {
                        //        dbg!("The implementation is incorrect. An additional mcycle must handle the operation");
                        
                        let mut sym = SymbolsTableCaseDependent::default();
                        sym.set_symbol_to_value("$", -2);
                        
                        let expr = arg2.unwrap().expr().unwrap();
                        let displacement = expr.relative_delta().unwrap();
                        
                        //             dbg!(displacement);
                        
                        // the very same value is also here
                        //   let displacement = self.wz.low().value() as i8 as i16 as u16;
                        
                        if displacement > 0 {
                            self.pc.add(displacement as u16);
                        } else {
                            self.pc.sub(-displacement as u16);
                        }
                        /*
                        // convert the 8bit number in a 16 bits value
                        let full = if displacement & 1<<7 != 0 {
                            0xff00
                        } else {
                            0x0000
                        } | displacement as u16;
                        self.pc.add(full);
                        */
                    }
                }
                
                Mnemonic::Im => {
                    match arg1.unwrap().expr().unwrap().eval().unwrap() {
                        0 => self.im = InterruptMode::Mode0,
                        1 => self.im = InterruptMode::Mode1,
                        2 => self.im = InterruptMode::Mode2,
                        _ => unreachable!()
                    }
                }
                
                Mnemonic::Ld => {
                    let arg1 = arg1.unwrap();
                    let arg2 = arg2.unwrap();
                    
                    let from_memory = matches!(
                        arg2, 
                        DataAccess::Memory(_) |  
                        DataAccess::MemoryIndexRegister16(_) | 
                        DataAccess::IndexRegister16WithIndex(_, _) |
                        DataAccess::MemoryRegister16(_)
                    );
                    let to_memory = matches!(
                        arg1, 
                        DataAccess::Memory(_) |
                        DataAccess::MemoryIndexRegister16(_) |
                        DataAccess::IndexRegister16WithIndex(_, _) |
                        DataAccess::MemoryRegister16(_)
                    );
                    
                    
                    let involve_indexes = matches!(arg1, DataAccess::IndexRegister16WithIndex(_, _)) | matches!(arg2, DataAccess::IndexRegister16WithIndex(_, _)
                    );

                    let involve_special_reg = matches!(arg1, DataAccess::SpecialRegisterI | DataAccess::SpecialRegisterR) | matches!(arg2, DataAccess::SpecialRegisterI | DataAccess::SpecialRegisterR
                    );

                    let involve_sp = matches!(arg1, DataAccess::Register16(cpclib_asm::preamble::Register16::Sp));
                
                if !(from_memory | to_memory) {
                    match arg1 {
                        DataAccess::Register8(reg) => {
                            self.inner_ld_reg8_access(reg.into(), arg2);
                        }
                        DataAccess::IndexRegister8(reg) => {
                            self.inner_ld_reg8_access(reg.into(), arg2);
                        }                                   
                        DataAccess::SpecialRegisterI => {
                            self.inner_ld_reg8_access(Reg8::I, arg2);
                        }
                        DataAccess::SpecialRegisterR => {
                            self.inner_ld_reg8_access(Reg8::R, arg2);
                        }
                        
                        
                        DataAccess::Register16(reg) => {
                            self.inner_ld_reg16_access(reg.into(), arg2);
                        }      
                        DataAccess::IndexRegister16(reg) => {
                            self.inner_ld_reg16_access(reg.into(), arg2);
                        }
                        
                        _ => unimplemented!("{:?}", arg1),
                    }
                }
                else if matches!(
                    (arg1,arg2),
                    (
                        DataAccess::Register16(cpclib_asm::preamble::Register16::Sp),
                        DataAccess::Register16(cpclib_asm::preamble::Register16::Hl) | DataAccess::IndexRegister16(_)
                    )
                ) {
                    self.next_state = MCycle::new_fake(1);
                }
                
                
                // needed for the addition ?
                if involve_indexes | involve_special_reg | involve_sp{ 
                    self.next_state = MCycle::new_fake(1);
                }
            }
            
            Mnemonic::Ldd => {
                self.inner_ldd();
                self.next_state = MCycle::new_fake(1);
            } 
            Mnemonic::Ldi => {
                self.inner_ldi();
                self.next_state = MCycle::new_fake(1);
            }
            Mnemonic::Lddr => {
                self.inner_ldd();
                let jmp = self.inner_bc_loop();
                self.next_state = MCycle::new_fake(if jmp {2} else {1});
            }
            Mnemonic::Ldir => {
                self.inner_ldi();
                let jmp = self.inner_bc_loop();
                self.next_state = MCycle::new_fake(if jmp {2} else {1});
            }

            Mnemonic::Neg=> {
                let a = self.a();
                self.reg8_mut(Reg8::A).set(
                    0u8.wrapping_sub(a)
                );
          
                self.auto_update_flags(self.a());
                self.set_flag(Z80Flags::C, a == 0);
                self.set_flag(Z80Flags::P, a == 0x80);
                self.set_flag_off(Z80Flags::N);
                
            },
            
            Mnemonic::Or => {
                let mut  val = self.get_available_8bits_value(arg1.unwrap());
                val = val | self.reg8(Reg8::A).value();                
                self.reg8_mut(Reg8::A).set(val);
                
                self.auto_update_flags(val);
                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag_off(Z80Flags::C);
            }
            
            Mnemonic::Outd => {
                let b_is_zero = self.reg8(Reg8::B).value() == 0;
                let f = &mut self.reg8_mut(Reg8::F).val;
                Z80Flags::Z.set(b_is_zero, f);
                Z80Flags::N.set_on(f);
            }
            
            Mnemonic::Push => {
                self.sp.dec();
                self.sp.dec();
                self.next_state = MCycle::new_fake(1);
            }
            
            Mnemonic::Pop => {
                self.sp.inc();
                self.sp.inc();
            }
            
            Mnemonic::Res => {
                let mut  val = self.get_available_8bits_value(arg2.unwrap());
                val |= 1 << arg1.unwrap().expr().unwrap().eval().unwrap();
                self.store_computed_8bits_value(arg2.unwrap(), val);
            }
            
            
            Mnemonic::Ret => {
                self.pc = self.wz;
                self.sp.inc();
                self.sp.inc();
            }
            
            
            Mnemonic::Rl => {
                let prev_c = self.test_flag(FlagTest::C);
                
                let mut  val = self.get_available_8bits_value(arg1.unwrap());
                let c = rla(&mut val, prev_c);
                self.store_computed_8bits_value(arg1.unwrap(), val);
                
                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag(Z80Flags::C, c);
            }
            
            Mnemonic::Rla => {
                let prev_c = self.test_flag(FlagTest::C);
                
                let mut  val = self.reg8(Reg8::A).value();
                let c = rla(&mut val, prev_c);
                self.reg8_mut(Reg8::A).set(val);
                
                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag(Z80Flags::C, c);
            }
            
            Mnemonic::Rlc => {
                let mut  val = self.get_available_8bits_value(arg1.unwrap());
                let c = rlc(&mut val);
                self.store_computed_8bits_value(arg1.unwrap(), val);
                
                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag(Z80Flags::C, c);

                if matches!(arg1,
                    Some(DataAccess::IndexRegister16WithIndex(_,_))) {
                        self.expect_additional_fake_cycle = 1;
                }
            }
            
            Mnemonic::Rlca => {
                let c = self.reg8_mut(Reg8::A).rlc();
                
                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag(Z80Flags::C, c);
            }
            Mnemonic::Rld => {
                let mut  val = self.get_available_8bits_value(arg1.unwrap());
                unimplemented!();
                self.store_computed_8bits_value(arg1.unwrap(), val);
            }
            Mnemonic::Rr => {
                let prev_c = self.test_flag(FlagTest::C);
                
                let mut  val = self.get_available_8bits_value(arg1.unwrap());
                let c = rr(&mut val, prev_c);
                self.store_computed_8bits_value(arg1.unwrap(), val);
                
                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag(Z80Flags::C, c);
            }
            
            Mnemonic::Rra => {
                let prev_c = self.test_flag(FlagTest::C);
                let c = self.reg8_mut(Reg8::A).rr(prev_c);
                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag(Z80Flags::C, c);
            }
            
            Mnemonic::Rrc => {
                
                let mut  val = self.get_available_8bits_value(arg1.unwrap());
                let c = rrc(&mut val);
                self.store_computed_8bits_value(arg1.unwrap(), val);
                
                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag(Z80Flags::C, c);
            }
            
            
            Mnemonic::Rrca => {
                let c = self.reg8_mut(Reg8::A).rrc();
                
                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag(Z80Flags::C, c);
            }
            
            Mnemonic::Rrd => {
                let mut  val = self.get_available_8bits_value(arg1.unwrap());
                unimplemented!();
                self.store_computed_8bits_value(arg1.unwrap(), val);
            }
            Mnemonic::Scf => {
                self.set_flag_on(Z80Flags::C);
            }
            
            Mnemonic::Rst => {
                self.sp.dec();
                self.sp.dec();
                
                let addr =   match arg1.unwrap().expr().unwrap().eval().unwrap(){
                    0 => 0,
                    8 => 0x8,
                    10 => 0x10,
                    18 => 0x18,
                    20 => 0x20,
                    28 => 0x28,
                    30 => 0x30,
                    38 => 0x38,
                    _ => unreachable!()
                };
                self.pc.set(addr);
                self.next_state = MCycle::new_fake(1);
            }
            
            
            Mnemonic::Set => {
                let bit = arg1.unwrap().expr().unwrap().eval().unwrap() as u8;
                
                match arg2.unwrap() {
                    DataAccess::Register8(reg8) => {
                        self.reg8_mut(reg8.into()).set_bit(bit);
                    }
                    
                    DataAccess::MemoryRegister16(_)
                    | DataAccess::IndexRegister16WithIndex(_, _) => {
                        // data is on the pins
                        let mut data = self.get_data_from_pins();
                        set_bit(&mut data, bit);
                        self.set_data_on_pins(data);
                    }
                    
                    other => {
                        unimplemented!("{:?}", other);
                    }
                }
            }
            
            Mnemonic::Sbc => {
                match (arg1, arg2) {
                    (
                        Some(DataAccess::Register16(reg1)), // hl by definition
                        Some(DataAccess::Register16(reg2))
                    ) => {
                        let reg1: Reg16 = reg1.into();
                        let reg2: Reg16 = reg2.into();
                        
                        let other = {
                            let mut other = self.reg16(reg2).value();
                            if self.flag(Z80Flags::C){
                                other = other.wrapping_add(1);
                            }        
                            other
                        };
                        let c = self.reg16_mut(reg1).sub(other);
                        let value = self.reg16(reg1).value();
                        
                        // TODO manage H if borrow from bit 12
                        Z80Flags::auto_update16(
                            &[Z80Flags::S, Z80Flags::Z, Z80Flags::X, Z80Flags::Y],
                            &mut self.reg8_mut(Reg8::F).val,
                            value,
                        );
                        self.set_flag_on(Z80Flags::N);
                        self.set_flag(Z80Flags::C, c);

                        let extra_cycles = 2;
                        self.next_state = MCycle::new_fake(extra_cycles);
                        
                    },
                    
                    (
                        Some(DataAccess::Register8(cpclib_asm::preamble::Register8::A)),
                        _
                    ) => {
                        let left = self.reg8(Reg8::A).value();
                        let right = {
                            let mut other = self.get_available_8bits_value(arg2.unwrap());
                            if self.flag(Z80Flags::C){
                                other = other.wrapping_add(1);
                            }        
                            other
                        };
                        
                        let (value, c) = left.overflowing_sub(right);
                        self.reg8_mut(Reg8::A).set(value);
                        
                        // TODO manage H if borrow from bit 12
                        Z80Flags::auto_update(
                            &[Z80Flags::S, Z80Flags::Z, Z80Flags::X, Z80Flags::Y],
                            &mut self.reg8_mut(Reg8::F).val,
                            value,
                        );
                        self.set_flag_on(Z80Flags::N);
                        self.set_flag(Z80Flags::C, c);
                    }
                    
                    _ => unimplemented!("{:?}, {:?}", arg1, arg2)
                }
            }
            
            Mnemonic::Sub => match arg1 {
                Some(DataAccess::Register8(_) | DataAccess::Expression(_)) => {
                    let left = self.reg8(Reg8::A).value();
                    let right = self.get_available_8bits_value(arg1.unwrap());
                    let f = &mut self.reg8_mut(Reg8::F).val;
                    let res = inner_8bits_sub!(left, right, f);
                    self.reg8_mut(Reg8::A).set(res);
                }
                
                _ => unimplemented!("{:?}, {:?}", arg1, arg2),
            },
  
            Mnemonic::Sl1 => {
                let mut  val = self.get_available_8bits_value(arg1.unwrap());
                let c = sl1(&mut val);
                self.store_computed_8bits_value(arg1.unwrap(), val);
                
                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag(Z80Flags::C, c);
            }


            Mnemonic::Sla => {
                let mut  val = self.get_available_8bits_value(arg1.unwrap());
                let c = sla(&mut val);
                self.store_computed_8bits_value(arg1.unwrap(), val);
                
                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag(Z80Flags::C, c);
            }
            
            Mnemonic::Sra => {
                let mut  val = self.get_available_8bits_value(arg1.unwrap());
                let c = sra(&mut val);
                self.store_computed_8bits_value(arg1.unwrap(), val);
                
                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag(Z80Flags::C, c);
            }
            
            
            Mnemonic::Srl => {
                let mut  val = self.get_available_8bits_value(arg1.unwrap());
                let c = srl(&mut val);
                self.store_computed_8bits_value(arg1.unwrap(), val);
                
                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag(Z80Flags::C, c);
            }
            
            Mnemonic::Xor => {
                let mut  val = self.get_available_8bits_value(arg1.unwrap());
                val = val ^ self.reg8(Reg8::A).value();
                self.reg8_mut(Reg8::A).set(val);
                
                self.auto_update_flags(val);
                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag_off(Z80Flags::C);
            }
            _ => unimplemented!("{:?}, {:?}, {:?}", mnemonic, arg1, arg2),
        }
    }
    /// Return the value for this data access
    /// - Inside a register / straightworard to obtain
    /// - In memory / the cpu has read it and stored it in XXX
    /// - In chip / the cpu has read it and stored it in XXX
    pub fn get_available_8bits_value(&self, data: &DataAccess) -> u8 {
        match data {
            DataAccess::Register8(reg) => {
                let reg: Reg8 = reg.into();
                return self.reg8(reg).get();
            }
            DataAccess::SpecialRegisterI => {
                return self.reg8(Reg8::I).get();
            }
            DataAccess::SpecialRegisterR => {
                return self.reg8(Reg8::R).get();
            }
            DataAccess::IndexRegister8(reg) => {
                let reg: Reg8 = reg.into();
                return self.reg8(reg).get();
            }
            DataAccess::MemoryRegister16(_) | DataAccess::IndexRegister16WithIndex(_, _)=> {
                // Here we do not care at all of which register is used for memory reading
                // Because the CPU has already read the information
                return self.reg8(Reg8::Z).get();
            }
            DataAccess::Expression(expr) => {
                return expr.eval().unwrap() as u8; // cannot fail as the value has been disassembled
            }
            DataAccess::Memory(_) => return self.reg8(Reg8::Z).get(),
            _ => {
                panic!("{:?} unhandled", data);
            }
        }
    }
    
    /// "Store" the currently compted value. When it is somewhere in memory, it is stored in the data pins
    /// This function is mainly usefull for the 
    pub fn store_computed_8bits_value(&mut self, data:&DataAccess, val: u8) {
        match data {
            DataAccess::Register8(reg) => {
                let reg: Reg8 = reg.into();
                self.reg8_mut(reg).set(val);
            }
            DataAccess::IndexRegister8(reg) => {
                let reg: Reg8 = reg.into();
                self.reg8_mut(reg).set(val);
            }
            DataAccess::MemoryRegister16(_)  | DataAccess::IndexRegister16WithIndex(_, _) => {
                self.set_data_on_pins(val);
            }
            DataAccess::Memory(_) => {
                self.set_data_on_pins(val);
            }
            _ => {
                panic!("{:?} unhandled", data);
            }
        }
    }
    
    /// Retreive the 16 bits that corresponds to the data choice
    pub fn get_available_16bits_value(&self, data: &DataAccess) -> u16 {
        match data {
            DataAccess::Register16(reg) => {
                let reg: Reg16 = reg.into();
                return self.reg16(reg).get();
            }
            DataAccess::IndexRegister16(reg) => {
                let reg: Reg16 = reg.into();
                return self.reg16(reg).get();
            }
            DataAccess::MemoryRegister16(_reg) => {
                panic!("Cannot be used / it is done directly during cpu execution");
                // Here we do not care at all of which register is used for memory reading
                // Because the CPU has already read the information
                //  return self.reg16(Reg16::Wz).get();
            }
            DataAccess::Expression(expr) => {
                return expr.eval().unwrap() as u16; // cannot fail as the value has been disassembled
            }
            DataAccess::Memory(_) => return self.reg16(Reg16::Wz).get(),
            _ => {
                panic!("Cannot be used / it is done directly during cpu execution");
            }
        }
    }
    
    /// Get the appropriate (inner) 16bits register in a mutable state
    pub fn reg16_mut(&mut self, reg: Reg16) -> &mut Register16 {
        match reg {
            Reg16::Af => &mut self.af,
            Reg16::Hl => &mut self.hl,
            Reg16::De => &mut self.de,
            Reg16::Bc => &mut self.bc,
            Reg16::Wz => &mut self.wz,
            
            Reg16::Af2 => &mut self.af2,
            Reg16::Hl2 => &mut self.hl2,
            Reg16::De2 => &mut self.de2,
            Reg16::Bc2 => &mut self.bc2,
            
            Reg16::Ix => &mut self.ix,
            Reg16::Iy => &mut self.iy,
            Reg16::Pc => &mut self.pc,
            Reg16::Sp => &mut self.sp,
        }
    }
    
    pub fn reg16(&self, reg: Reg16) -> &Register16 {
        match reg {
            Reg16::Af => &self.af,
            Reg16::Hl => &self.hl,
            Reg16::De => &self.de,
            Reg16::Bc => &self.bc,
            Reg16::Wz => &self.wz,
            
            Reg16::Af2 => &self.af2,
            Reg16::Hl2 => &self.hl2,
            Reg16::De2 => &self.de2,
            Reg16::Bc2 => &self.bc2,
            
            Reg16::Ix => &self.ix,
            Reg16::Iy => &self.iy,
            Reg16::Pc => &self.pc,
            Reg16::Sp => &self.sp,
        }
    }
    
    pub fn reg8_mut(&mut self, reg: Reg8) -> &mut Register8 {
        match reg {
            Reg8::I => &mut self.i,
            Reg8::R => &mut self.r,
            _ => {
                let r16 = self.reg16_mut(reg.full());
                if reg.is_high() {
                    r16.high_mut()
                } else {
                    r16.low_mut()
                }
            }
        }
    }
    
    pub fn reg8(&self, reg: Reg8) -> &Register8 {
        match reg {
            Reg8::I => & self.i,
            Reg8::R => & self.r,
            _ => {
                
                let r16 = self.reg16(reg.full());
                if reg.is_high() {
                    r16.high()
                } else {
                    r16.low()
                }
            }
        }
    }
}

/// Opcode specific code
impl Z80 {
    /// Manage the specific treatment after the read_instruction
    /// TODO check if it does not belong to another Cycle (I'm pretty sure of that) !!!
    pub fn post_read_action(&mut self) {
        let opcode_info = self.opcode_information.as_ref().unwrap();
        
        match opcode_info.token.mnemonic() {
            Some(Mnemonic::Outd) => {
                self.reg16_mut(Reg16::Hl).dec();
                self.reg8_mut(Reg8::B).dec();
            }
            
            
            Some(Mnemonic::Ld) => {
                if let Some(DataAccess::Register16(cpclib_asm::preamble::Register16::Sp)) =
                opcode_info.token.mnemonic_arg1()
                {
                    // arg2 is a memory read operation by definition
                    self.sp = self.wz.clone();
                    self.wz.inc();
                }
            }
            
            _ => {
                // by default, there is nothing to
            }
        }
    }
}

impl Z80 {
    fn inner_ld_reg8_access(&mut self, reg: Reg8, access: &DataAccess) {
        let val = self.get_available_8bits_value(access);
        self.reg8_mut(reg).set(val);
    }
    
    fn inner_ld_reg16_access(&mut self, reg: Reg16, access: &DataAccess) {
        let val = self.get_available_16bits_value(access);
        self.reg16_mut(reg).set(val);
    }
    
    /// Handle the registers for LDD/LDDR
    fn inner_ldd(&mut self) {
        self.reg16_mut(Reg16::Hl).dec();
        self.reg16_mut(Reg16::De).dec();
        self.reg16_mut(Reg16::Bc).dec();
        
        self.set_flag_off(Z80Flags::H);
        self.set_flag_off(Z80Flags::N);
        self.set_flag(Z80Flags::P, self.bc()!=0);
    }
    
    /// Handle the registers for LDI/LDIR
    fn inner_ldi(&mut self) {
        self.reg16_mut(Reg16::Hl).inc();
        self.reg16_mut(Reg16::De).inc();
        self.reg16_mut(Reg16::Bc).dec();
        
        self.set_flag_off(Z80Flags::H);
        self.set_flag_off(Z80Flags::N);
        self.set_flag(Z80Flags::P, self.bc()!=0);
    }
    
    /// Handle the loop for LDDR/LDIR
    /// Return true when pc is modified
    fn inner_bc_loop(&mut self) -> bool {
        if self.bc() != 0 {
            self.reg16_mut(Reg16::Pc).dec();
            self.reg16_mut(Reg16::Pc).dec();
            true
        }
        else {
            false
        }
        
    }
}

/// Everything that is related to interruptions
impl Z80 {
    /// Handle interruption if any
    // must be called only at the end of an instruction
    fn instruction_finished(&mut self) {
        // manage the flipflip for interruption
        if !self.get_pin(Z80Pin::Neg_INT)
        && self.iff1
        && !matches!(
            self.next_state,
            MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None)
        )
        {
            
            self.iff1 = false;
            self.iff2 = false;
            
            assert_eq!(
                self.im,
                InterruptMode::Mode1,
                "Interruption {:?} unimplemented",
                self.im
            );
            
            self.simulate_call_0x38_after_fetch();
        } else {
            self.next_state = Default::default();
            return;
        }
    }
    
    /// Handcraft a call to let the CPU jump in 0x38
    /// TODO manage a Fake M1 cycle
    fn simulate_call_0x38_after_fetch(&mut self) {
        self.opcode_information = Some(&OPCODE_PREFIX[0xcd]);
        self.next_state = MCycle::Write(TState::T1, 0);
    }
}



impl Z80 {
    pub fn next_state(&self) -> &MCycle {
        &self.next_state
    }
}
/// Get registers value
impl Z80 {
    pub fn pc(&self) -> u16 {
        self.reg16(Reg16::Pc).value()
    }
    pub fn sp(&self) -> u16 {
        self.reg16(Reg16::Sp).value()
    }
    pub fn af(&self) -> u16 {
        self.reg16(Reg16::Af).value()
    }
    pub fn bc(&self) -> u16 {
        self.reg16(Reg16::Bc).value()
    }
    pub fn de(&self) -> u16 {
        self.reg16(Reg16::De).value()
    }
    pub fn hl(&self) -> u16 {
        self.reg16(Reg16::Hl).value()
    }
    pub fn ix(&self) -> u16 {
        self.reg16(Reg16::Ix).value()
    }
    pub fn iy(&self) -> u16 {
        self.reg16(Reg16::Iy).value()
    }
    
    pub fn a(&self) -> u8 {
        self.reg8(Reg8::A).value()
    }
    pub fn b(&self) -> u8 {
        self.reg8(Reg8::B).value()
    }
    pub fn d(&self) -> u8 {
        self.reg8(Reg8::D).value()
    }
    pub fn h(&self) -> u8 {
        self.reg8(Reg8::H).value()
    }
    pub fn ixh(&self) -> u8 {
        self.reg8(Reg8::Ixh).value()
    }
    pub fn iyh(&self) -> u8 {
        self.reg8(Reg8::Iyh).value()
    }
    
    pub fn c(&self) -> u8 {
        self.reg8(Reg8::C).value()
    }
    pub fn e(&self) -> u8 {
        self.reg8(Reg8::E).value()
    }
    pub fn l(&self) -> u8 {
        self.reg8(Reg8::L).value()
    }
    pub fn x(&self) -> u8 {
        self.reg8(Reg8::Ixl).value()
    }
    pub fn y(&self) -> u8 {
        self.reg8(Reg8::Iyl).value()
    }
}

/// Set registers value
impl Z80 {
    pub fn set_pc(&mut self, value: u16) {
        self.reg16_mut(Reg16::Pc).set(value)
    }
    pub fn set_sp(&mut self, value: u16) {
        self.reg16_mut(Reg16::Sp).set(value)
    }
    pub fn set_af(&mut self, value: u16) {
        self.reg16_mut(Reg16::Af).set(value)
    }
    pub fn set_bc(&mut self, value: u16) {
        self.reg16_mut(Reg16::Bc).set(value)
    }
    pub fn set_de(&mut self, value: u16) {
        self.reg16_mut(Reg16::De).set(value)
    }
    pub fn set_hl(&mut self, value: u16) {
        self.reg16_mut(Reg16::Hl).set(value)
    }
    pub fn set_ix(&mut self, value: u16) {
        self.reg16_mut(Reg16::Ix).set(value)
    }
    pub fn set_iy(&mut self, value: u16) {
        self.reg16_mut(Reg16::Iy).set(value)
    }
}

impl AddressChip for Z80 {
    fn pins_address() -> &'static [Self::PinType; 16] {
        return &PIN_ADDRESS_BUS;
    }
}

impl DataChip for Z80 {
    fn pins_data() -> &'static [Self::PinType; 8] {
        return &Z80_PIN_DATA_BUS;
    }
}

impl WriteAddressChip for Z80 {}

impl ReadAddressChip for Z80 {}

impl WriteDataChip for Z80 {}

impl ReadDataChip for Z80 {}

impl Chip for Z80 {
    type PinType = Z80Pin;
    
    fn pins_buffer_mut(&mut self) -> &mut PinsBuffer {
        &mut self.pins
    }
    fn pins_buffer(&self) -> &PinsBuffer {
        &self.pins
    }
    
    fn must_execute_tick(&self) -> bool {
        self.get_pin(Z80Pin::Neg_CLK) && self.get_pin(Z80Pin::Neg_WAIT) // TODO do that only for Fetch/T2
    }
    
    fn execute_tick(&mut self) {
        //dbg!(&self.next_state);
        match self.next_state {
            // First state of the fetch process
            MCycle::FetchOpcode(TState::T1, _, _) => {
                self.expect_additional_fake_cycle = 0;

                self.set_address_on_pins(self.pc.get());
                // pc is incremented in T2 in a real CPU
                self.pc.inc();
                
                self.set_pin(Z80Pin::Neg_MREQ, false);
                self.set_pin(Z80Pin::Neg_RD, false);
                self.set_pin(Z80Pin::Neg_M1, false);
                self.set_pin(Z80Pin::Neg_RFSH, true);
                
                self.next_state.inc_state();
            }
            
            // Second step / may be delayed if wait is activated
            MCycle::FetchOpcode(TState::T2, nb, _) => {
                self.set_pin(Z80Pin::Neg_RD, true);
                self.set_pin(Z80Pin::Neg_MREQ, true);
                self.set_pin(Z80Pin::Neg_M1, true);
                
                let byte = dbg!(self.get_data_from_pins());
                match nb {
                    0 => {
                        self.ir.set(byte);
                    }
                    1 => {
                        self.reg8_mut(Reg8::Z).set(byte);
                    }
                    2 => {
                        self.reg8_mut(Reg8::W).set(byte);
                    }
                    _ => {
                        unreachable!()
                    }
                }
                self.set_pin(Z80Pin::Neg_RFSH, false);
                self.set_address_on_pins(self.refresh_addr);
                self.next_state.inc_state();
            }
            
            MCycle::FetchOpcode(TState::T3, _, _) => {
                // we do nothing here
                // but, IRL, the decoding appears there
                self.next_state.inc_state();
                self.set_pin(Z80Pin::Neg_MREQ, false);
            }
            
            MCycle::FetchOpcode(TState::T4, nb, prefix) => {
                self.set_pin(Z80Pin::Neg_MREQ, true);
                self.set_pin(Z80Pin::Neg_M1, false);
                self.set_pin(Z80Pin::Neg_RFSH, true);
                
                if  (nb == 0 && !prefix.is_for_indexed_opcode()) || 
                    (nb ==1 &&  prefix.is_for_indexed_opcode()) {
                    let execute = self.decode_step(); // setup next state
                    // by definition there is no conditional instruction there
                    if execute {
                        // nothing is required for the execution
                        // TODO check if input or memory cycle is required
                        self.execute_step();
                    } else {
                        // nothing to do ; the cpu is already in the right state
                    }
                } 
                else if prefix.is_for_indexed_opcode() {
                    self.next_state = MCycle::FetchOpcode(TState::T1, nb + 1, prefix);
                }
                else {
                    let opcode_information = self.opcode_information.as_ref().unwrap();
                    
                    if nb == opcode_information.nb_additional_opcode_bytes {
                        // We have finished to read the opcode
                        if !self.is_condition_met() {
                            // nothing happens there we can read the next instruction without executing anything
                            // serve for call/jp/jr
                            // TODO check if it must serve for loop based instruction
                            self.instruction_finished();
                        } else if  opcode_information.expect_read() {
                            if opcode_information.read_from_pins {
                                self.next_state = MCycle::Input(TState::T1, 0);
                            } else {
                                self.next_state = MCycle::Read(TState::T1, 0);
                            }
                        } else if opcode_information.expect_write() {
                            if opcode_information.read_from_pins {
                                self.next_state = MCycle::Output(TState::T1, 0);
                            } else {
                                self.next_state = MCycle::Write(TState::T1, 0);
                            }
                        } else {
                            // go back for a new instruction
                            self.execute_step();
                        }
                    } else {
                        // we still need to read something
                        self.next_state = MCycle::FetchOpcode(TState::T1, nb + 1, prefix);
                    }
                }
            }
            
            MCycle::Read(TState::T1, nb) | MCycle::Input(TState::T1, nb) => {
                let address = self.get_read_address_from_opcode_information().wrapping_add(nb as u16);
                self.set_address_on_pins(address);
                
                eprintln!("READ ADDR={:x}", address);
                
                assert!(self.get_pin(self.next_state.read_pin_type().unwrap()));
                assert!(self.get_pin(Z80Pin::Neg_RD));
                assert!(self.get_pin(Z80Pin::Neg_WR));
                
                self.off(self.next_state.read_pin_type().unwrap());
                self.off(Z80Pin::Neg_RD);
                self.next_state.inc_state();
            }
            MCycle::Read(TState::T2, nb) | MCycle::Input(TState::T2, nb) => {
                let byte = self.get_data_from_pins();
                //eprintln!("DATA={:x}", byte);
                self.store_currently_read_byte(byte, nb);
                
                self.on(self.next_state.read_pin_type().unwrap());
                self.on(Z80Pin::Neg_RD);
                self.next_state.inc_state();
            }
            /// simulate wait
            MCycle::Read(TState::T3, nb) | MCycle::Input(TState::T3, nb) => {
                self.next_state.inc_state();
            }
            MCycle::Read(TState::T4, nb) | MCycle::Input(TState::T4, nb) => {
                self.post_read_action();
                
                let opcode_info = self.opcode_information.as_ref().unwrap().clone();
                if nb + 1 == opcode_info.nb_bytes_to_read {
                    if 0 != opcode_info.nb_bytes_to_write {
                        if self.must_execute_instruction_in_middle() {
                            self.execute_step();
                        }
                        if opcode_info.write_to_pins {
                            self.next_state = MCycle::Output(TState::T1, 0);
                        } else {
                            self.next_state = MCycle::Write(TState::T1, 0);
                        }
                    } else {
                        self.execute_step();
                        
                        if !matches!(
                            self.next_state,
                            MCycle::Fake(TState::T1, _)
                        ){                      
                            self.next_state = MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None);
                            // go back for a new instruction
                        }
                    }
                } else {
                    // go to read the next byte
                    if self.next_state.is_input() {
                        self.next_state = MCycle::Input(TState::T1, nb + 1);
                    } else {
                        self.next_state = MCycle::Read(TState::T1, nb + 1);
                    }
                }
            }
            
            MCycle::Write(TState::T1, nb) | MCycle::Output(TState::T1, nb) => {
                let opcode_information = self.opcode_information().unwrap();

                if !opcode_information.compute_between_read_write() {
                    // Get the real address to put on the pins
                    let address = if opcode_information.write_on_stack()  {
                        let base = self.get_write_address_from_opcode_information();
                        match nb {
                            0 => base + 1,
                            1 => base,
                            _ => unreachable!(),
                        }
                    } else {
                        self.get_write_address_from_opcode_information().wrapping_add(nb as u16)
                    };
                    //eprintln!("Write in 0x{:x}", address);
                    self.set_address_on_pins(address);
                } else {
                    // address is already at the right place
                    // to compute it once again will provide false information
                }
                    
                
                eprintln!("WRITE ADDR={:x}", self.get_address_from_pins());
                assert!(self.get_pin(self.next_state.write_pin_type().unwrap()));
                assert!(self.get_pin(Z80Pin::Neg_RD));
                assert!(self.get_pin(Z80Pin::Neg_WR));
                
                self.next_state.inc_state();
            }
            MCycle::Write(TState::T2, nb) | MCycle::Output(TState::T2, nb) => {
                self.off(self.next_state.write_pin_type().unwrap());
                self.off(Z80Pin::Neg_WR); // or is it the writer that does that ?
                
                let byte = self.get_byte_to_write_from_opcode_information(nb);
                self.set_data_on_pins(byte);
                
                self.next_state.inc_state();
            }
            
            // simulates a wait state
            MCycle::Write(TState::T3, nb) | MCycle::Output(TState::T3, nb) => {
                self.next_state.inc_state();
            }
            
            MCycle::Write(TState::T4, nb) | MCycle::Output(TState::T4, nb) => {
                self.on(self.next_state.write_pin_type().unwrap());
                self.on(Z80Pin::Neg_WR);
                
                if nb == self.opcode_information.as_ref().unwrap().nb_bytes_to_write - 1 {
                    // TODO take into account those that do a write after the read (if that exists)
                    if !self.must_execute_instruction_in_middle() {
                        self.execute_step();
                    } else {
                        self.opcode_information = None;
                    }
                    if !matches!(
                        self.next_state,
                        MCycle::Fake(TState::T1, _)
                    ){
                        if self.expect_additional_fake_cycle > 0 {
                            self.next_state = MCycle::new_fake(self.expect_additional_fake_cycle);
                        }
                        else {
                            self.next_state = MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None);
                        }
                    }
                    
                    // go back for a new instruction
                } else {
                    // go to write the next byte
                    if self.next_state.is_output() {
                        self.next_state = MCycle::Output(TState::T1, nb + 1);
                    } else {
                        self.next_state = MCycle::Write(TState::T1, nb + 1);
                    }
                }
            }
            
            MCycle::Fake(TState::T4, 1) => {
                self.instruction_finished();
            },
            MCycle::Fake(_, _) => {
                self.next_state.inc_state();
            }
            
            
            _ => unimplemented!(),
        }
    }
    
    fn reset(&mut self) {
        self.pins = (0, 0).into();
        
        self.set_pin(Z80Pin::Neg_MREQ, true);
        self.set_pin(Z80Pin::Neg_IORQ, true);
        self.set_pin(Z80Pin::Neg_WR, true);
        self.set_pin(Z80Pin::Neg_RD, true);
        self.set_pin(Z80Pin::Neg_M1, true);
        self.set_pin(Z80Pin::Neg_RFSH, true);
        
        self.next_state = MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None);
        
        self.sp.set(0xffff);
        self.pc.set(0x0000);
        self.af.set(0xffff);
        self.iff1 = false;
        self.iff2 = false;
        self.im = InterruptMode::Mode1;
    }
}

impl Activable for Z80 {
    fn clock_pin() -> Self::PinType {
        Z80Pin::Neg_CLK
    }
}

#[cfg(test)]
mod test {
    use super::*;
    
    #[test]
    fn test_pins_indices() {
        assert_eq!(Z80Pin::A11 as u64, 1 << 0);
        assert_eq!(Z80Pin::A12 as u64, 1 << 1);
        assert_eq!(Z80Pin::A13 as u64, 1 << 2);
    }
    
    #[test]
    fn test_pins() {
        let mut cpu = Z80::default();
        cpu.set_address_on_pins(0x7F00);
        assert_eq!(cpu.get_address_from_pins(), 0x7F00);
    }
    
    #[test]
    fn test_registers() {
        let mut a = crate::z80::Register8::default();
        let mut b = crate::z80::Register8::default();
        
        a.set(1);
        b.set(2);
        
        assert_eq!(1, a.get());
        assert_eq!(2, b.get());
        
        a.add(b.get());
        assert_eq!(3, a.get());
    }
    
    #[test]
    fn test_registers_in_cpu() {
        let mut cpu = Z80::default();
        cpu.reg8_mut(Reg8::A).set(1);
        cpu.reg8_mut(Reg8::B).set(2);
        
        assert_eq!(cpu.reg8(Reg8::A).get(), 1);
        assert_eq!(cpu.reg8(Reg8::B).get(), 2);
        
        let val = cpu.reg8(Reg8::B).get();
        cpu.reg8_mut(Reg8::A).add(val);
        assert_eq!(cpu.reg8(Reg8::A).get(), 3);
    }
    
    #[test]
    fn test_nop_FetchOpcode() {
        let mut cpu = Z80::default();
        cpu.pc.set(0x1234);
        test_byte_FetchOpcode(&mut cpu, 0x00);
    }
    
    #[test]
    fn test_xora_FetchOpcode() {
        let mut cpu = Z80::default();
        cpu.pc.set(0x1234);
        cpu.reg8_mut(Reg8::A).set(0xff);
        
        assert_eq!(cpu.reg8(Reg8::A).get(), 0xff);
        test_byte_FetchOpcode(&mut cpu, 0xaf);
        assert_eq!(cpu.reg8(Reg8::A).get(), 0x00);
    }
    
    #[test]
    fn test_add_e_FetchOpcode() {
        let mut cpu = Z80::default();
        cpu.pc.set(0x1234);
        cpu.reg8_mut(Reg8::A).set(0x01);
        cpu.reg8_mut(Reg8::E).set(0x10);
        
        assert_eq!(cpu.reg8(Reg8::A).get(), 0x01);
        assert_eq!(cpu.reg8(Reg8::E).get(), 0x10);
        test_byte_FetchOpcode(&mut cpu, 0x83);
        assert_eq!(cpu.reg8(Reg8::A).get(), 0x11);
    }
    
    #[test]
    fn test_ld_l_1() {
        let mut cpu = Z80::default();
        cpu.pc.set(0x1234);
        cpu.reg8_mut(Reg8::L).set(0xff);
        test_byte_FetchOpcode2(&mut cpu, 0x2e, 0x01);
        assert_eq!(cpu.reg8(Reg8::L).get(), 0x01);
    }
    
    #[test]
    fn test_ld_nn_a() {
        let mut cpu = Z80::default();
        cpu.pc.set(0x1234);
        cpu.reg8_mut(Reg8::A).set(0xff);
        
        cpu.set_pin(Z80Pin::Neg_MREQ, true);
        cpu.set_pin(Z80Pin::Neg_RD, true);
        cpu.set_pin(Z80Pin::Neg_WAIT, true);
        cpu.set_pin(Z80Pin::Neg_M1, true);
        cpu.set_pin(Z80Pin::Neg_RFSH, true);
        cpu.set_pin(Z80Pin::Neg_IORQ, true);
        cpu.set_pin(Z80Pin::Neg_WR, true);
        
        // Get opcode
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None)
        );
        cpu.tick();
        cpu.set_data_on_pins(0x32);
        cpu.tick();
        cpu.tick();
        cpu.tick();
        
        assert_eq!(
            &OpcodePrefix::None.opcode_information(0x32),
            cpu.opcode_information.as_ref().unwrap()
        );
        
        assert_eq!(
            cpu.opcode_information
            .as_ref()
            .unwrap()
            .nb_additional_opcode_bytes,
            2
        );
        
        assert_eq!(
            cpu.opcode_information.as_ref().unwrap().nb_bytes_to_write,
            1
        );
        
        // Get address
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T1, 1, OpcodePrefix::None)
        );
        cpu.tick();
        cpu.set_data_on_pins(0x12);
        cpu.tick();
        cpu.tick();
        cpu.tick();
        
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T1, 2, OpcodePrefix::None)
        );
        cpu.tick();
        cpu.set_data_on_pins(0x34);
        cpu.tick();
        cpu.tick();
        cpu.tick();
        
        // Write address
        assert_eq!(cpu.next_state, MCycle::Write(TState::T1, 0));
        cpu.tick();
        cpu.tick();
        cpu.tick();
        assert_eq!(cpu.next_state, MCycle::Write(TState::T1, 1));
        cpu.tick();
        cpu.tick();
        cpu.tick();
        
        // Really write byte
        assert_eq!(cpu.next_state, MCycle::Write(TState::T1, 2));
        cpu.tick();
        cpu.tick();
        cpu.tick();
        
        //next opcode
        
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None)
        );
    }
    
    #[test]
    fn test_inc_ixl() {
        let mut cpu = Z80::default();
        cpu.pc.set(0x1234);
        cpu.reg8_mut(Reg8::Ixl).set(0xff);
        test_byte_FetchOpcode_prefix(&mut cpu, 0xdd, 0x2c);
        assert_eq!(cpu.reg8(Reg8::Ixl).get(), 0x00);
    }
    
    #[test]
    fn test_xor_mem_hl_FetchOpcode() {
        let mut cpu = Z80::default();
        cpu.pc.set(0x1234);
        cpu.reg8_mut(Reg8::A).set(0xff);
        
        assert_eq!(cpu.reg8(Reg8::A).get(), 0xff);
        test_byte_FetchOpcodeRead(&mut cpu, 0xae, 0xff);
        assert_eq!(cpu.reg8(Reg8::A).get(), 0x00);
    }
    
    #[test]
    fn test_lut() {
        assert_eq!(
            OpcodePrefix::None
            .opcode_information(0x00)
            .nb_additional_opcode_bytes,
            0
        );
        
        // ld a, (nnnn)
        let desc = dbg!(OpcodePrefix::None.opcode_information(0b00111010));
        assert_eq!(desc.nb_additional_opcode_bytes, 2);
        assert_eq!(desc.nb_bytes_to_read, 1);
        
        // ld l, nn
        let desc = dbg!(OpcodePrefix::None.opcode_information(0x2e));
        assert_eq!(desc.nb_additional_opcode_bytes, 1);
        
        // ld (nn), a
        let desc = dbg!(OpcodePrefix::None.opcode_information(0x32));
        assert_eq!(desc.nb_additional_opcode_bytes, 2);
        assert_eq!(desc.nb_bytes_to_write, 1);
    }
    
    #[test]
    fn test_ld_a_mem_FetchOpcode() {
        let mut cpu = Z80::default();
        cpu.pc.set(0x1234);
        
        cpu.set_pin(Z80Pin::Neg_MREQ, true);
        cpu.set_pin(Z80Pin::Neg_RD, true);
        cpu.set_pin(Z80Pin::Neg_WAIT, true);
        cpu.set_pin(Z80Pin::Neg_M1, true);
        cpu.set_pin(Z80Pin::Neg_RFSH, true);
        cpu.set_pin(Z80Pin::Neg_IORQ, true);
        cpu.set_pin(Z80Pin::Neg_WR, true);
        
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None)
        );
        
        cpu.reg8_mut(Reg8::A).set(0x12);
        assert_eq!(cpu.reg8(Reg8::A).get(), 0x12);
        
        cpu.tick();
        cpu.set_data_on_pins(0b00111010);
        cpu.tick();
        cpu.tick();
        cpu.tick();
        
        cpu.tick();
        cpu.set_data_on_pins(0xff);
        cpu.tick();
        cpu.tick();
        cpu.tick();
        
        cpu.tick();
        cpu.set_data_on_pins(0xff);
        cpu.tick();
        cpu.tick();
        cpu.tick();
        
        cpu.tick();
        cpu.set_data_on_pins(0x34);
        cpu.tick();
        cpu.tick();
        cpu.tick();
        
        assert_eq!(cpu.reg8(Reg8::A).get(), 0x34);
    }
    
    fn force_front(cpu: &mut Z80) {
        cpu.set_pin(Z80Pin::Neg_CLK, false);
        cpu.save_pins_state();
        cpu.set_pin(Z80Pin::Neg_CLK, true);
    }
    
    fn test_byte_FetchOpcode(cpu: &mut Z80, byte: u8) {
        let pc_base = cpu.reg16(Reg16::Pc).get();
        force_front(cpu);
        
        cpu.set_pin(Z80Pin::Neg_MREQ, true);
        cpu.set_pin(Z80Pin::Neg_RD, true);
        cpu.set_pin(Z80Pin::Neg_WAIT, true);
        cpu.set_pin(Z80Pin::Neg_M1, true);
        cpu.set_pin(Z80Pin::Neg_RFSH, true);
        cpu.set_pin(Z80Pin::Neg_IORQ, true);
        cpu.set_pin(Z80Pin::Neg_WR, true);
        
        assert!(cpu.is_activated());
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None)
        );
        cpu.tick();
        let addr = cpu.get_address_from_pins();
        assert_eq!(addr, pc_base);
        assert_eq!(cpu.pc.get(), pc_base + 1);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_MREQ), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_M1), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RD), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_IORQ), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_WR), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RFSH), true);
        
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T2, 0, OpcodePrefix::None)
        );
        
        // GA set wait
        cpu.set_pin(Z80Pin::Neg_WAIT, false);
        cpu.tick();
        assert_eq!(cpu.pc.get(), pc_base + 1);
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T2, 0, OpcodePrefix::None)
        );
        assert_eq!(cpu.get_pin(Z80Pin::Neg_MREQ), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_M1), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RD), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_IORQ), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_WR), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RFSH), true);
        
        // Byte is available
        cpu.set_pin(Z80Pin::Neg_WAIT, true);
        cpu.set_data_on_pins(byte);
        cpu.tick();
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T3, 0, OpcodePrefix::None)
        );
        
        cpu.tick();
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T4, 0, OpcodePrefix::None)
        );
        
        cpu.tick();
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None)
        );
    }
    
    fn test_byte_FetchOpcodeRead(cpu: &mut Z80, byte: u8, data: u8) {
        let pc_base = cpu.reg16(Reg16::Pc).get();
        force_front(cpu);
        
        cpu.set_pin(Z80Pin::Neg_MREQ, true);
        cpu.set_pin(Z80Pin::Neg_RD, true);
        cpu.set_pin(Z80Pin::Neg_WAIT, true);
        cpu.set_pin(Z80Pin::Neg_M1, true);
        cpu.set_pin(Z80Pin::Neg_RFSH, true);
        cpu.set_pin(Z80Pin::Neg_IORQ, true);
        cpu.set_pin(Z80Pin::Neg_WR, true);
        
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None)
        );
        cpu.tick();
        let addr = cpu.get_address_from_pins();
        assert_eq!(addr, pc_base);
        assert_eq!(cpu.pc.get(), pc_base + 1);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_MREQ), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_M1), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RD), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_IORQ), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_WR), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RFSH), true);
        
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T2, 0, OpcodePrefix::None)
        );
        
        // GA set wait
        cpu.set_pin(Z80Pin::Neg_WAIT, false);
        cpu.tick();
        assert_eq!(cpu.pc.get(), pc_base + 1);
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T2, 0, OpcodePrefix::None)
        );
        assert_eq!(cpu.get_pin(Z80Pin::Neg_MREQ), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_M1), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RD), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_IORQ), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_WR), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RFSH), true);
        
        // Byte is available
        cpu.set_pin(Z80Pin::Neg_WAIT, true);
        cpu.set_data_on_pins(byte);
        cpu.tick();
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T3, 0, OpcodePrefix::None)
        );
        
        cpu.tick();
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T4, 0, OpcodePrefix::None)
        );
        
        cpu.tick(); // execute
        assert_eq!(cpu.next_state, MCycle::Read(TState::T1, 0));
        
        ///TODO add assertions there
        cpu.tick();
        cpu.set_data_on_pins(data);
        cpu.tick();
        cpu.tick();
    }
    
    fn test_byte_FetchOpcodeRead2(cpu: &mut Z80, byte: u8, data: u8, data2: u8) {
        let pc_base = cpu.reg16(Reg16::Pc).get();
        force_front(cpu);
        
        cpu.set_pin(Z80Pin::Neg_MREQ, true);
        cpu.set_pin(Z80Pin::Neg_RD, true);
        cpu.set_pin(Z80Pin::Neg_WAIT, true);
        cpu.set_pin(Z80Pin::Neg_M1, true);
        cpu.set_pin(Z80Pin::Neg_RFSH, true);
        cpu.set_pin(Z80Pin::Neg_IORQ, true);
        cpu.set_pin(Z80Pin::Neg_WR, true);
        
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None)
        );
        cpu.tick();
        let addr = cpu.get_address_from_pins();
        assert_eq!(addr, pc_base);
        assert_eq!(cpu.pc.get(), pc_base + 1);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_MREQ), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_M1), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RD), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_IORQ), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_WR), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RFSH), true);
        
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T2, 0, OpcodePrefix::None)
        );
        
        // GA set wait
        cpu.set_pin(Z80Pin::Neg_WAIT, false);
        cpu.tick();
        assert_eq!(cpu.pc.get(), pc_base + 1);
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T2, 0, OpcodePrefix::None)
        );
        assert_eq!(cpu.get_pin(Z80Pin::Neg_MREQ), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_M1), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RD), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_IORQ), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_WR), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RFSH), true);
        
        // Byte is available
        cpu.set_pin(Z80Pin::Neg_WAIT, true);
        cpu.set_data_on_pins(byte);
        cpu.tick();
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T3, 0, OpcodePrefix::None)
        );
        
        cpu.tick();
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T4, 0, OpcodePrefix::None)
        );
        
        cpu.tick(); // execute
        assert_eq!(cpu.next_state, MCycle::Read(TState::T1, 0));
        
        ///TODO add assertions there
        cpu.tick();
        cpu.set_data_on_pins(data);
        cpu.tick();
        cpu.tick();
        
        ///TODO add assertions there
        cpu.tick();
        cpu.set_data_on_pins(data2);
        cpu.tick();
        cpu.tick();
    }
    
    fn test_byte_FetchOpcode2(cpu: &mut Z80, byte1: u8, byte2: u8) {
        let pc_base = cpu.reg16(Reg16::Pc).get();
        force_front(cpu);
        
        cpu.set_pin(Z80Pin::Neg_MREQ, true);
        cpu.set_pin(Z80Pin::Neg_RD, true);
        cpu.set_pin(Z80Pin::Neg_WAIT, true);
        cpu.set_pin(Z80Pin::Neg_M1, true);
        cpu.set_pin(Z80Pin::Neg_RFSH, true);
        cpu.set_pin(Z80Pin::Neg_IORQ, true);
        cpu.set_pin(Z80Pin::Neg_WR, true);
        
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None)
        );
        cpu.tick();
        let addr = cpu.get_address_from_pins();
        assert_eq!(addr, pc_base);
        assert_eq!(cpu.pc.get(), pc_base + 1);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_MREQ), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_M1), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RD), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_IORQ), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_WR), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RFSH), true);
        
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T2, 0, OpcodePrefix::None)
        );
        
        // GA set wait
        cpu.set_pin(Z80Pin::Neg_WAIT, false);
        cpu.tick();
        assert_eq!(cpu.pc.get(), pc_base + 1);
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T2, 0, OpcodePrefix::None)
        );
        assert_eq!(cpu.get_pin(Z80Pin::Neg_MREQ), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_M1), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RD), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_IORQ), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_WR), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RFSH), true);
        
        // Byte is available
        cpu.set_pin(Z80Pin::Neg_WAIT, true);
        cpu.set_data_on_pins(byte1);
        cpu.tick();
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T3, 0, OpcodePrefix::None)
        );
        
        cpu.tick();
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T4, 0, OpcodePrefix::None)
        );
        
        cpu.tick();
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T1, 1, OpcodePrefix::None)
        );
        
        cpu.tick();
        let addr = cpu.get_address_from_pins();
        assert_eq!(addr, pc_base + 1);
        assert_eq!(cpu.pc.get(), pc_base + 2);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_MREQ), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_M1), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RD), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_IORQ), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_WR), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RFSH), true);
        
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T2, 1, OpcodePrefix::None)
        );
        
        // GA set wait
        cpu.set_pin(Z80Pin::Neg_WAIT, false);
        cpu.tick();
        assert_eq!(cpu.pc.get(), pc_base + 2);
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T2, 1, OpcodePrefix::None)
        );
        assert_eq!(cpu.get_pin(Z80Pin::Neg_MREQ), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_M1), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RD), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_IORQ), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_WR), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RFSH), true);
        
        // Byte is available
        cpu.set_pin(Z80Pin::Neg_WAIT, true);
        cpu.set_data_on_pins(byte2);
        cpu.tick();
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T3, 1, OpcodePrefix::None)
        );
        
        cpu.tick();
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T4, 1, OpcodePrefix::None)
        );
        
        cpu.tick();
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None)
        );
    }
    
    fn test_byte_FetchOpcode_prefix(cpu: &mut Z80, byte1: u8, byte2: u8) {
        let prefix = match byte1 {
            0xdd => OpcodePrefix::Dd,
            _ => unimplemented!(),
        };
        
        let pc_base = cpu.reg16(Reg16::Pc).get();
        force_front(cpu);
        
        cpu.set_pin(Z80Pin::Neg_MREQ, true);
        cpu.set_pin(Z80Pin::Neg_RD, true);
        cpu.set_pin(Z80Pin::Neg_WAIT, true);
        cpu.set_pin(Z80Pin::Neg_M1, true);
        cpu.set_pin(Z80Pin::Neg_RFSH, true);
        cpu.set_pin(Z80Pin::Neg_IORQ, true);
        cpu.set_pin(Z80Pin::Neg_WR, true);
        
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None)
        );
        cpu.tick();
        let addr = cpu.get_address_from_pins();
        assert_eq!(addr, pc_base);
        assert_eq!(cpu.pc.get(), pc_base + 1);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_MREQ), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_M1), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RD), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_IORQ), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_WR), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RFSH), true);
        
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T2, 0, OpcodePrefix::None)
        );
        
        // GA set wait
        cpu.set_pin(Z80Pin::Neg_WAIT, false);
        cpu.tick();
        assert_eq!(cpu.pc.get(), pc_base + 1);
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T2, 0, OpcodePrefix::None)
        );
        assert_eq!(cpu.get_pin(Z80Pin::Neg_MREQ), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_M1), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RD), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_IORQ), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_WR), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RFSH), true);
        
        // Byte is available
        cpu.set_pin(Z80Pin::Neg_WAIT, true);
        cpu.set_data_on_pins(byte1);
        cpu.tick();
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T3, 0, OpcodePrefix::None)
        );
        
        cpu.tick();
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T4, 0, OpcodePrefix::None)
        );
        
        cpu.tick();
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T1, 0, prefix));
        
        cpu.tick();
        let addr = cpu.get_address_from_pins();
        assert_eq!(addr, pc_base + 1);
        assert_eq!(cpu.pc.get(), pc_base + 2);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_MREQ), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_M1), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RD), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_IORQ), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_WR), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RFSH), true);
        
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T2, 0, prefix));
        
        // GA set wait
        cpu.set_pin(Z80Pin::Neg_WAIT, false);
        cpu.tick();
        assert_eq!(cpu.pc.get(), pc_base + 2);
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T2, 0, prefix));
        assert_eq!(cpu.get_pin(Z80Pin::Neg_MREQ), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_M1), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RD), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_IORQ), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_WR), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RFSH), true);
        
        // Byte is available
        cpu.set_pin(Z80Pin::Neg_WAIT, true);
        cpu.set_data_on_pins(byte2);
        cpu.tick();
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T3, 0, prefix));
        
        cpu.tick();
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T4, 0, prefix));
        
        cpu.tick();
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None)
        );
    }
    
    #[test]
    fn registers_operation() {
        let mut r16 = crate::z80::Register16::default();
        assert_eq!(r16.value(), 0);
        
        r16.set(0xffff);
        assert_eq!(r16.value(), 0xffff);
        
        r16.add(1);
        assert_eq!(r16.value(), 0);
        
        r16.inc();
        assert_eq!(r16.value(), 1);
    }
}
