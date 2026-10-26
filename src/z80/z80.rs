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
use std::ops::BitAnd;

use cpclib_asm::preamble::Token;
use super::alu;
use cpclib_asm::preamble::{
    DataAccess, DataAccessElem, Expr, ExprElement, ExprEvaluationExt, FlagTest, ListingElement,
    Mnemonic,
};

fn eval_expr_to_i32(expr: &Expr) -> i32 {
    match expr {
        // What the decoder produces for operands: read it directly.
        // `Expr::eval` builds a whole default assembler environment (a
        // zeroed memory image included) on every call.
        Expr::Value(value) => *value,
        _ => expr.eval().unwrap().int_value().unwrap()
    }
}

fn eval_data_access_to_i32(access: &DataAccess) -> i32 {
    eval_expr_to_i32(access.get_expression().unwrap())
}

macro_rules! inner_8bits_sub {
    ($a:ident,$b:ident,$f:ident) => {{
        let (res, ovf) = $a.overflowing_sub($b);
        
        Z80Flags::auto_update(
            &[Z80Flags::S, Z80Flags::Z, Z80Flags::X, Z80Flags::Y],
            $f,
            res,
        );

        Z80Flags::H.set($b & 0x0f > $a & 0x0f, $f);
        Z80Flags::C.set(ovf, $f);
        Z80Flags::N.set_on($f);
        Z80Flags::P.set((($b ^ $a) & ($a ^ res) & 0x80) != 0, $f);

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

impl BitAnd<Z80Flags> for u8 {
    type Output = u8;
    fn bitand(self, rhs: Z80Flags) -> Self::Output {
        self & rhs as u8
    }
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
            Self::X => self.set(res & 1 << (8+3) != 0, byte), // XXX probably wrong
            Self::Y => self.set(res & 1 << (8+5) != 0, byte), // XXX probably wrong
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

#[derive(Debug, PartialEq, Clone, Copy)]
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

/// See [`Z80::io_window`].
#[derive(Default, Debug, Clone, Copy)]
struct IoWindow {
    start: u8,
    end: u8,
    t: u8,
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
    expect_additional_fake_cycle: u8,

    /// CPU has received an interruption (remember GA send it only during 1.4ms but an instruction can take 6 nops)
    interruption_received: bool,

    /// An interrupt cannot be taken right after EI (ACCC 27.3.3): the
    /// instruction following it always runs. Counts the instruction ends
    /// still shadowed.
    ei_shadow: u8,

    /// IM 2 response: 0 = none, 1 = the return address is being pushed, 2 = the
    /// vector is being read (address `I<<8 | #FF`, the data bus being pulled up).
    im2_stage: u8,

    /// A non-maskable interrupt is waiting for the end of the instruction.
    nmi_pending: bool,
    /// The interrupt being answered is an NMI: its M1 is an ordinary read (no acknowledge), and
    /// it calls &0066.
    nmi_in_progress: bool,
    /// The low byte of the IM 2 table entry, put on the bus by the device that interrupts (a
    /// Z80 CTC); none, the data bus floats at &FF as on a bare CPC.
    interrupt_vector: Option<u8>,

    /// Where the instruction being executed started, and what it is - to know
    /// where its last T-state falls (see `tstates`).
    instr_start_pc: u16,
    instr_prefix: OpcodePrefix,
    instr_op: u8,
    /// T-states left idle at the end of the last microsecond of the last
    /// executed instruction (0 to 3).
    last_idle_t: u8,
    /// The output being written: IORQ and /WR active from T-state `start` to `end` (T2 to T3 of
    /// the real I/O cycle once stretched by the Gate Array's /WAIT, see `tstates`), counted from
    /// the T1 of the Output M-cycle; `t` is the T-state of the next tick.
    io_window: Option<IoWindow>,

    /// Low byte of a not-yet-completed SSM (ScreenShot Management) pair:
    /// set when an "ED <neutral>" instruction is executed and we're
    /// waiting to see whether the very next instruction is also
    /// "ED <neutral>" (see SSM-STANDARD-EN.pdf).
    pending_ssm_low: Option<u8>,

    /// SSM codes detected since the last call to `take_ssm_codes`.
    ssm_codes: Vec<u16>
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
    pub fn get_flag(&self, flag: Z80Flags) -> bool {
        let f = self.reg8(Reg8::F).value();
        flag.is_on(f)
    }

    #[inline]
    pub fn set_flag(&mut self, flag: Z80Flags, val: bool) {
        let f = &mut self.reg8_mut(Reg8::F).val;
        flag.set(val, f);
    }
    
    #[inline]
    pub fn set_flag_on(&mut self, flag: Z80Flags) {
        self.set_flag(flag, true);
    }
    
    #[inline]
    pub fn set_flag_off(&mut self, flag: Z80Flags) {
        self.set_flag(flag, false);
    }
    
    #[inline]
    pub fn flag(&self, flag: Z80Flags) -> bool {
        let flags = self.reg8(Reg8::F).value();
        flag.is_on(flags)
    }
    
    /// The F register.
    #[inline]
    fn flags(&self) -> u8 {
        self.reg8(Reg8::F).value()
    }

    #[inline]
    fn set_flags(&mut self, flags: u8) {
        self.reg8_mut(Reg8::F).set(flags);
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
    

    #[inline]
    fn auto_update_flags_x_y(&mut self, val: u8) {
        let f = &mut self.reg8_mut(Reg8::F).val;
        Z80Flags::auto_update(&[
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
            let info = self.opcode_information().unwrap();

            info.compute_between_read_write() &&
                !info.token.is_output_opcode()
            
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
            if self.im2_stage == 2 {
                return (u16::from(self.reg8(Reg8::I).get()) << 8) | u16::from(self.interrupt_vector.unwrap_or(0xff));
            }
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
                AddressSource::PortN => ((self.a() as u16) << 8) + self.reg8(Reg8::Z).value() as u16,
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
                    debug_assert_eq!(step, 0);
                    self.reg8(*reg).get() as u16
                }
                DataSource::Pins => {
                    debug_assert_eq!(step, 0);
                    self.get_data_from_pins() as u16
                }
                DataSource::Zero => 0,
                DataSource::Register16(reg) | DataSource::IndexRegister16(reg) => {
                    self.reg16(*reg).value()
                }
                DataSource::Immediate => {
                    if matches!(self.opcode_information().unwrap().prefix, OpcodePrefix::Dd | OpcodePrefix::Fd) {
                        // LD (IX+d),n / LD (IY+d),n: the first operand byte
                        // (Z) is the displacement, the value to store is the
                        // second one (W). Using Z stored the displacement.
                        u16::from(self.reg8(Reg8::W).get())
                    }
                    else {
                        self.reg16(Reg16::Wz).value()
                    }
                }
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
                    debug_assert!(nb == 0);
                    self.reg8_mut(reg).set(byte);
                    
                    //      dbg!(reg, self.reg8_mut(reg));
                }
                DataSource::Register16(reg) | DataSource::IndexRegister16(reg) => {
                    let reg = if reg == Reg16::Pc /*|| reg == Reg16::Sp*/ {
                        Reg16::Wz
                    } else {
                        reg
                    };
                    let reg = self.reg16_mut(reg);
                    let reg = if nb == 0 { reg.low_mut() } else { reg.high_mut() };
                    reg.set(byte);
                }
                DataSource::Pins => {
                    debug_assert_eq!(nb, 0);
                    self.reg8_mut(Reg8::Z).set(byte); // not sure if it is the case in real world, but at least it make work some instructions
                }

                _ => {
                    unimplemented!("{:?}/ {:?}", dst, self.opcode_information)
                }
            }
        }
        
        /// The state starting the port read of the current instruction.
        /// ACCC 4.4.3: the read of IN r,(C) and INI/IND lands on the 4th
        /// microsecond (IN A,(n): the 3rd), so those wait one microsecond first.
        fn input_start_state(&self) -> MCycle {
            let late = match self.opcode_information().map(|i| &i.token) {
                Some(Token::OpCode(Mnemonic::In, _, Some(DataAccess::PortC), _)) => true,
                Some(Token::OpCode(Mnemonic::Ini | Mnemonic::Ind | Mnemonic::Inir | Mnemonic::Indr, _, _, _)) => true,
                _ => false,
            };
            if late {
                MCycle::IoWait(TState::T1, false)
            } else {
                MCycle::Input(TState::T1, 0)
            }
        }

        /// Where IORQ and /WR of the output being started are active, in T-states from the T1 of
        /// this model's Output M-cycle (which sits in the microsecond of the real I/O cycle's T2).
        fn output_window(&self) -> IoWindow {
            let prefix = match self.instr_prefix {
                OpcodePrefix::None => 0x00,
                OpcodePrefix::Cb => 0xcb,
                OpcodePrefix::Ed => 0xed,
                OpcodePrefix::Dd => 0xdd,
                OpcodePrefix::Fd => 0xfd,
                OpcodePrefix::DdCb => 0xdc,
                OpcodePrefix::FdCb => 0xfc,
            };
            let timing = super::tstates::cpc_bus_timing(prefix, self.instr_op, false, false);
            match (timing.iorq_t, timing.iorq_end_t) {
                (Some(start), Some(end)) => {
                    let base = start / 4 * 4;
                    IoWindow { start: (start - base) as u8, end: (end - base) as u8, t: 0 }
                }
                _ => IoWindow { start: 0, end: 2, t: 0 },
            }
        }

        /// OUTI/OUTD/OTIR/OTDR
        fn is_block_output(&self) -> bool {
            matches!(
                self.opcode_information().and_then(|i| i.token.mnemonic()),
                Some(Mnemonic::Outi | Mnemonic::Outd | Mnemonic::Otir | Mnemonic::Otdr)
            )
        }


        /// Execute the instruction ; all necessary stuffs has already been collected
        fn execute_step(&mut self) {
            debug_assert!(
                !matches!(
                    self.next_state,
                    MCycle::Fake(_, _)
                ),
                "We never call execute_state after fake timing"
            );
            
            
            let write_follows;
            {
                // this is the normal behavior
                // we may add additional duration
                let prefix = &self.opcode_information.as_ref().unwrap().prefix;
                
                // Get the two additional bytes needed for the instruction
                let memory = [self.reg8(Reg8::Z).get(), self.reg8(Reg8::W).get()];
                
                // Decoded from the opcode bits and the operand bytes (see
                // `decoder.rs`): the instruction table and this agree on
                // the same instruction, including the DD/FD opcodes that
                // execute as their unprefixed form and the undefined ED
                // opcodes (e.g. those SSM repurposes as screenshot-request
                // codes, see SSM-STANDARD-EN.pdf), which are NOPs.
                let opcode = super::decoder::with_operands(*prefix, self.ir.get(), memory);
         //       dbg!(&opcode);
                self.execute_opcode(
                    opcode.mnemonic().unwrap(),
                    opcode.mnemonic_arg1(),
                    opcode.mnemonic_arg2(),
                );
                
                // reset the various buffers
                let in_middle = self.must_execute_instruction_in_middle();
                write_follows = in_middle
                    && self.opcode_information().unwrap().nb_bytes_to_write != 0;
                if !in_middle {
                    self.opcode_information = None;
                }
            }

            if write_follows {
                // Read-modify-write (e.g. DEC (HL)): only the compute step
                // happened here, the write M-cycle is still to come. The
                // instruction retires at the end of that write cycle (see
                // the `Write`/`Output` T4 handler) - accepting a pending
                // interrupt now would drop the write.
            }
            else if matches!(
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
                0xdd if current_prefix == OpcodePrefix::None => {
                    // A non-ED first byte breaks a pending SSM pair (see SSM-STANDARD-EN.pdf).
                    self.pending_ssm_low = None;
                    self.next_state = MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::Dd);
                    debug_assert!(current_prefix == OpcodePrefix::None, "impossible");
                    return false;
                }
                0xfd if current_prefix == OpcodePrefix::None => {
                    self.pending_ssm_low = None;
                    self.next_state = MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::Fd);
                    debug_assert!(current_prefix == OpcodePrefix::None, "impossible");

                    return false;
                }
                0xcb if matches!(current_prefix, OpcodePrefix::None | OpcodePrefix::Dd | OpcodePrefix::Fd) => {
                    self.pending_ssm_low = None;
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
                0xed if current_prefix == OpcodePrefix::None => {
                    self.next_state = MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::Ed);
                    return false;
                }
                
                _ => {
                    // SSM (ScreenShot Management) detection: an ED-prefixed
                    // suffix feeds the ED/ED pair detector; any other fully
                    // decoded instruction (no ED prefix at all) breaks a
                    // pending pair. See SSM-STANDARD-EN.pdf.
                    if current_prefix == OpcodePrefix::Ed {
                        self.detect_ssm(self.ir.get());
                    } else {
                        self.pending_ssm_low = None;
                    }

                    // Indexed opcodes  need to be exchanged
                    if current_prefix.is_for_indexed_opcode() {
                        let z = self.reg8(Reg8::Z).value();
                        let ir = self.ir.value();
                        self.reg8_mut(Reg8::Z).set(ir);
                        self.ir.set(z);
                    }

                    // Get the number of additional bytes to read as well as the information for the next cycles
                    self.instr_prefix = current_prefix;
                    self.instr_op = self.ir.get();
                    self.opcode_information = Some(current_prefix.opcode_information(self.ir.get()));
                    let opcode_information = self.opcode_information.as_ref().unwrap();
                    
                   //dbg!(opcode_information);


                    if opcode_information.fetch_finished() || current_prefix.is_for_indexed_opcode() {

                        if let 
                            Token::OpCode(
                                Mnemonic::Ret,
                                Some(DataAccess::FlagTest(test)),
                                None,
                                None
                            ) = opcode_information.token {
                                if !self.test_flag(test) {
                                    self.next_state = MCycle::new_fake(1);
                                    return false;
                                } else {
                                    self.next_state = MCycle::Read(TState::T1, 0);
                                    return false;
                                }
                            }
                           
                        


                        if opcode_information.expect_read() {
                            if opcode_information.read_from_pins {
                                self.next_state = self.input_start_state();
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
                Mnemonic::Nop => {}
                
                Mnemonic::Adc => {
                    match arg1 {
                        Some(DataAccess::Register16(cpclib_asm::preamble::Register16::Hl)) => {
                            let left = self.hl.value();
                            let right = self.get_available_16bits_value(arg2.unwrap());
                            let (result, flags) = alu::adc16(left, right, self.flag(Z80Flags::C));
                            self.wz.set(left.wrapping_add(1));
                            self.hl.set(result);
                            self.set_flags(flags);
                            self.next_state = MCycle::new_fake(2);
                        }
                        _ => {
                            let right = self.get_available_8bits_value(arg2.or(arg1).unwrap());
                            let (result, flags) = alu::add8(self.a(), right, self.flag(Z80Flags::C));
                            self.reg8_mut(Reg8::A).set(result);
                            self.set_flags(flags);
                        }
                    }
                }
                
                
                
                Mnemonic::Add => {
                    let involve_indexes = matches!(arg1, Some(DataAccess::IndexRegister16WithIndex(_, _, _))) | matches!(arg2, Some(DataAccess::IndexRegister16WithIndex(_, _, _))
                    );

                    match arg1 {
                        None | Some(DataAccess::Register8(cpclib_asm::preamble::Register8::A)) => {
                            let right = self.get_available_8bits_value(arg2.or(arg1).unwrap());
                            let (result, flags) = alu::add8(self.a(), right, false);
                            self.reg8_mut(Reg8::A).set(result);
                            self.set_flags(flags);
                        }

                        Some(DataAccess::Register16(_)) | Some(DataAccess::IndexRegister16(_))  => {
                            let reg: Reg16 = match arg1.unwrap() {
                                DataAccess::Register16(reg) => reg.into(),
                                DataAccess::IndexRegister16(reg) => reg.into(),
                                _ => unimplemented!()
                            };
                            let left = self.reg16(reg).value();
                            let right = self.get_available_16bits_value(arg2.unwrap());
                            let (result, flags) = alu::add16(left, right, self.flags());
                            self.wz.set(left.wrapping_add(1));
                            self.reg16_mut(reg).set(result);
                            self.set_flags(flags);

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
                    let mut  val = self.get_available_8bits_value(arg2.or(arg1).unwrap());
                    val = val & self.reg8(Reg8::A).value();
                    self.reg8_mut(Reg8::A).set(val);
                    
                    self.auto_update_flags(val);
                    self.set_flag_on(Z80Flags::H);
                    self.set_flag_off(Z80Flags::N);
                    self.set_flag_off(Z80Flags::C);
                    self.auto_update_flags_x_y(val);
                }
                
                Mnemonic::Bit => {
                    let val = self.get_available_8bits_value(arg2.unwrap());
                    let number = (eval_data_access_to_i32(arg1.unwrap()) & 7) as u8;
                    let bit_set = val & (1 << number) != 0;

                    // Z is set when the tested bit is *clear*; P/V is a copy
                    // of Z; S is set when bit 7 is the tested bit and is set.
                    // (This used to set Z when the bit was set - every BIT
                    // was inverted, e.g. the firmware "is ESC held?" test.)
                    self.set_flag_on(Z80Flags::H);
                    self.set_flag(Z80Flags::Z, !bit_set);
                    self.set_flag(Z80Flags::P, !bit_set);
                    self.set_flag(Z80Flags::S, number == 7 && bit_set);
                    self.set_flag_off(Z80Flags::N);
                    // X/Y come from the tested register; for a memory operand
                    // real hardware takes them from the high byte of the
                    // internal address register (WZ).
                    let xy_source: u8 = if matches!(arg2, Some(DataAccess::Register8(_))) {
                        (val & 0xff) as u8
                    }
                    else {
                        self.reg16(Reg16::Wz).high().get()
                    };
                    self.auto_update_flags_x_y(xy_source);

                    if matches!(arg2, Some(DataAccess::IndexRegister16WithIndex(_, _, _))) {
                        self.next_state = MCycle::new_fake(1);
                    }
                }
                
                Mnemonic::Call => {
                    #[cfg(test)]
                    {
                        // conditional call where condition is not met MUST not arrive there
                        if arg1.is_some() {
                            debug_assert!(self.is_condition_met());
                        }
                    }
                    
                    let addr = match arg2 {
                        Some(DataAccess::Expression(exp)) => eval_expr_to_i32(exp) as u16,
                        
                        _ => unreachable!(),
                    };
                    
                    self.pc.set(addr);
                    self.sp.dec();
                    self.sp.dec();
                }
                
                
                Mnemonic::Ccf => {
                    let flags = alu::ccf(self.a(), self.flags());
                    self.set_flags(flags);
                }

                Mnemonic::Cp => {
                    let right = self.get_available_8bits_value(arg2.or(arg1).unwrap());
                    let flags = alu::cp8(self.a(), right);
                    self.set_flags(flags);
                }
                
                Mnemonic::Cpi | Mnemonic::Cpd | Mnemonic::Cpir | Mnemonic::Cpdr => {
                    let value = self.get_data_from_pins();
                    let (result, compared) = alu::sub8(self.a(), value, false);

                    if matches!(mnemonic, Mnemonic::Cpi | Mnemonic::Cpir) {
                        self.hl.inc();
                    }
                    else {
                        self.hl.dec();
                    }
                    self.bc.dec();

                    // S, Z, H as for CP; N set; C untouched; P/V tells BC is
                    // not 0; X and Y come from the result minus the half carry
                    let adjusted = result.wrapping_sub(u8::from(compared & alu::H != 0));
                    let mut flags = (compared & (alu::S | alu::Z | alu::H | alu::N)) | (self.flags() & alu::C);
                    if self.bc() != 0 {
                        flags |= alu::PV;
                    }
                    if adjusted & 0x08 != 0 {
                        flags |= alu::X;
                    }
                    if adjusted & 0x02 != 0 {
                        flags |= alu::Y;
                    }
                    self.set_flags(flags);

                    let repeat = matches!(mnemonic, Mnemonic::Cpir | Mnemonic::Cpdr)
                        && self.bc() != 0
                        && flags & alu::Z == 0;
                    if repeat {
                        self.next_state = MCycle::new_fake(2);
                        self.pc.dec();
                        self.pc.dec();
                    }
                    else {
                        self.next_state = MCycle::new_fake(1);
                    }
                }
                
                
                Mnemonic::Cpl => {
                    let (result, flags) = alu::cpl(self.a(), self.flags());
                    self.reg8_mut(Reg8::A).set(result);
                    self.set_flags(flags);
                }

                Mnemonic::Daa => {
                    let (result, flags) = alu::daa(self.a(), self.flags());
                    self.reg8_mut(Reg8::A).set(result);
                    self.set_flags(flags);
                }

                Mnemonic::Dec => {
                    match arg1.unwrap() {
                        DataAccess::Register8(reg) => {
                            let left: Reg8 = reg.into();
                            let (result, flags) = alu::dec8(self.reg8(left).value(), self.flags());
                            self.reg8_mut(left).set(result);
                            self.set_flags(flags);
                        }
                        DataAccess::IndexRegister8(reg) => {
                            let left: Reg8 = reg.into();
                            let (result, flags) = alu::dec8(self.reg8(left).value(), self.flags());
                            self.reg8_mut(left).set(result);
                            self.set_flags(flags);
                        }
                        // 16 bits: no flag at all
                        DataAccess::Register16(reg) => {
                            let left: Reg16 = reg.into();
                            self.reg16_mut(left).dec();
                        }
                        DataAccess::IndexRegister16(reg) => {
                            let left: Reg16 = reg.into();
                            self.reg16_mut(left).dec();
                        }
                        DataAccess::MemoryRegister16(_) | DataAccess::IndexRegister16WithIndex(_, _, _) => {
                            // executed in the middle
                            let (result, flags) = alu::dec8(self.get_data_from_pins(), self.flags());
                            self.set_data_on_pins(result);
                            self.set_flags(flags);
                        }
                        _ => unreachable!()
                    }

                    if matches!(arg1, Some(DataAccess::IndexRegister16WithIndex(_, _, _))) {
                        self.expect_additional_fake_cycle = 1;
                    }
                    else if matches!(arg1, Some(DataAccess::Register16(_) | DataAccess::IndexRegister16(_))) {
                        self.next_state = MCycle::new_fake(1);
                    }
                }
                
                Mnemonic::Di => {
                    self.iff1 = false;
                    self.iff2 = false;
                }
                
                Mnemonic::Djnz => {
                    self.reg8_mut(Reg8::B).dec();
                    if self.reg8(Reg8::B).value() != 0 {
                        // Same reasoning as JR above: use the raw fetched
                        // displacement byte rather than re-evaluating the
                        // disassembled expression, which encodes the
                        // displacement biased by +2 for its own "$"-relative
                        // display convention (and, for a self-referencing
                        // jump, as a bare "$" symbol that isn't numerically
                        // evaluable at all without a PC to resolve it
                        // against).
                        let displacement = self.reg8(Reg8::Z).value() as i8;
                        if displacement > 0 {
                            self.pc.add(displacement as u16);
                        } else {
                            self.pc.sub(-displacement as u16);
                        }
                        self.next_state = MCycle::new_fake(2);
                    }
                    else {
                        self.next_state = MCycle::new_fake(1);
                    }
                }
                
                Mnemonic::Ei => {
                    self.iff1 = true;
                    self.iff2 = true;
                    self.ei_shadow = 1;
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
                    if (self.interruption_received && self.iff1) || self.nmi_pending {
                        // Nothing more to do here: instruction_finished(),
                        // called right after this for every instruction
                        // alike, independently checks interruption_received
                        // && iff1 and vectors to RST 38h on its own. PC is
                        // already one past this HALT (from its own fetch),
                        // which is the correct return address for the
                        // RETI that ends the interrupt handler.
                    } else {
                        self.pc.dec(); // wait and restart
                    }
                }

                Mnemonic::In => {
                    match arg1 {
                        // `IN A,(n)` is 3 us on a CPC; every `IN r,(C)` form -
                        // `IN A,(C)` included - is 4 us, the extra microsecond
                        // being waited *before* the port is read (see
                        // `input_start_state`)
                        Some(DataAccess::Register8(_)) | Some(DataAccess::Expression(_)) => {},
                        _ => unreachable!()
                    }

                    // `IN A,(n)` (the fixed-port form) affects no flags at
                    // all. Every `IN r,(C)` form - including `IN A,(C)`,
                    // which the previous code silently skipped entirely -
                    // sets S/Z/P/X/Y from the byte just read, resets H/N,
                    // and leaves C untouched. Leaving `IN A,(C)` a flags
                    // no-op broke the extremely common real-world idiom of
                    // polling a hardware status bit with `IN A,(C)` then
                    // `JR Z`/`JR NZ`, which would branch on a stale flag
                    // from whatever instruction last set one instead.
                    if !matches!(arg2, Some(DataAccess::PortN(_))) {
                        // IN F,(C) (ED 70) reads for the flags only
                        let val = match arg1 {
                            Some(DataAccess::Register8(_)) => self.get_available_8bits_value(arg1.unwrap()),
                            _ => self.get_data_from_pins()
                        };
                        self.auto_update_flags(val);
                        self.auto_update_flags_x_y(val);
                        self.set_flag_off(Z80Flags::H);
                        self.set_flag_off(Z80Flags::N);
                    }
                }
                
                Mnemonic::Inc => {
                    match arg1.unwrap() {
                        DataAccess::MemoryRegister16(_) | DataAccess::IndexRegister16WithIndex(_, _, _)=> {
                            // read and write directly in pins (should be launched before writing the results)
                            debug_assert_eq!(
                                self.opcode_information().unwrap().read_data,
                                DataDestination::Pins
                            );
                            debug_assert_eq!(
                                self.opcode_information().unwrap().write_data,
                                DataDestination::Pins
                            );
                            let (result, flags) = alu::inc8(self.get_data_from_pins(), self.flags());
                            self.set_data_on_pins(result);
                            self.set_flags(flags);
                        }
                        DataAccess::Register8(reg) => {
                            let left: Reg8 = reg.into();
                            let (result, flags) = alu::inc8(self.reg8(left).value(), self.flags());
                            self.reg8_mut(left).set(result);
                            self.set_flags(flags);
                        }
                        DataAccess::IndexRegister8(reg) => {
                            let left: Reg8 = reg.into();
                            let (result, flags) = alu::inc8(self.reg8(left).value(), self.flags());
                            self.reg8_mut(left).set(result);
                            self.set_flags(flags);
                        }
                        // 16 bits: no flag at all
                        DataAccess::Register16(reg) => {
                            let left: Reg16 = reg.into();
                            self.reg16_mut(left).inc();
                        }
                        DataAccess::IndexRegister16(reg) => {
                            let left: Reg16 = reg.into();
                            self.reg16_mut(left).inc();
                        }
                        _ => unimplemented!(),
                    }

                    if matches!(arg1, Some(DataAccess::IndexRegister16WithIndex(_, _, _))) {
                        self.expect_additional_fake_cycle = 1;
                    }
                    else if matches!(arg1, Some(DataAccess::Register16(_) | DataAccess::IndexRegister16(_))) {
                        self.next_state = MCycle::new_fake(1);
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
                            DataAccess::Expression(expr) => eval_expr_to_i32(expr) as u16,
                            
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
                        // JR semantics depend on the fetched immediate displacement byte.
                        // Using the raw byte avoids ambiguities in symbolic disassembly forms.
                        let displacement = self.reg8(Reg8::Z).value() as i8;
             
                      
                        if displacement > 0 {
                            self.pc.add(displacement as u16);
                        } else {
                            self.pc.sub(-displacement as u16);
                        }

                    }

                    self.next_state = MCycle::new_fake(1);
                }
                
                Mnemonic::Im => {
                    match eval_data_access_to_i32(arg1.unwrap()) {
                        0 => self.im = InterruptMode::Mode0,
                        1 => self.im = InterruptMode::Mode1,
                        2 => self.im = InterruptMode::Mode2,
                        _ => unreachable!()
                    }
                }
                
                // HL only moves now (see post_read_action): the write
                // just completed used the address from before this move.
                Mnemonic::Ind => {
                    self.inner_ind_flags();
                    self.hl.dec();
                                    }
                Mnemonic::Ini => {
                    self.inner_ini_flags();
                    self.hl.inc();
                                    }
                Mnemonic::Indr => {
                    self.inner_ind_flags();
                    self.hl.dec();
                    if self.b() != 0 {
                        self.pc.dec();
                        self.pc.dec();
                        self.next_state = MCycle::new_fake(1);
                    }
                }
                Mnemonic::Inir => {
                    self.inner_ini_flags();
                    self.hl.inc();
                    if self.b() != 0 {
                        self.pc.dec();
                        self.pc.dec();
                        self.next_state = MCycle::new_fake(1);
                    }
                }

                Mnemonic::Ld => {
                   // dbg!(self.opcode_information());

                    let arg1 = arg1.unwrap();
                    let arg2 = arg2.unwrap();
                    
                    let from_memory = matches!(
                        arg2, 
                        DataAccess::Memory(_) |  
                        DataAccess::MemoryIndexRegister16(_) | 
                        DataAccess::IndexRegister16WithIndex(_, _, _) |
                        DataAccess::MemoryRegister16(_)
                    );
                    let to_memory = matches!(
                        arg1, 
                        DataAccess::Memory(_) |
                        DataAccess::MemoryIndexRegister16(_) |
                        DataAccess::IndexRegister16WithIndex(_, _, _) |
                        DataAccess::MemoryRegister16(_)
                    );
                    
                    
                    let involve_indexes = matches!(arg1, DataAccess::IndexRegister16WithIndex(_, _, _)) | matches!(arg2, DataAccess::IndexRegister16WithIndex(_, _, _)
                    );

                    let involve_special_reg = matches!(arg1, DataAccess::SpecialRegisterI | DataAccess::SpecialRegisterR) | matches!(arg2, DataAccess::SpecialRegisterI | DataAccess::SpecialRegisterR
                    );

                    // only LD SP,HL/IX/IY has an internal cycle (6 T-states): LD SP,nn
                    // (3 us on a CPC) and LD SP,(nn) (5 us) do not
                    let involve_sp = matches!(
                        (arg1, arg2),
                        (
                            DataAccess::Register16(cpclib_asm::preamble::Register16::Sp),
                            DataAccess::Register16(cpclib_asm::preamble::Register16::Hl) | DataAccess::IndexRegister16(_)
                        )
                    );
                
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


                if matches!(arg2, DataAccess::SpecialRegisterI | DataAccess::SpecialRegisterR) {
                    self.auto_update_flags(self.a());
                    self.set_flag_off(Z80Flags::N);
                    self.set_flag_off(Z80Flags::H);
                    self.set_flag(Z80Flags::P, self.iff2);
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

            Mnemonic::Neg => {
                let (result, flags) = alu::sub8(0, self.a(), false);
                self.reg8_mut(Reg8::A).set(result);
                self.set_flags(flags);
            },
            
            Mnemonic::Or => {
                let mut  val = self.get_available_8bits_value(arg2.or(arg1).unwrap());
                val = val | self.reg8(Reg8::A).value();                
                self.reg8_mut(Reg8::A).set(val);
                
                self.auto_update_flags(val);
                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag_off(Z80Flags::C);
                self.auto_update_flags_x_y(val);
            }

            Mnemonic::Out => {
                if let Some(DataAccess::PortN(_)) =  arg1 {
                
                } else {
                    self.next_state = MCycle::new_fake(1);
                }
            }
  
            // OUTI/OUTD write the port on the 5th microsecond (an IoWait is
            // inserted before the output): nothing is idle afterwards, except
            // the extra microsecond of a repeating iteration
            Mnemonic::Outi => {
                self.inner_outi_flags();
            }

            Mnemonic::Otir => {
                self.inner_outi_flags();
                if self.b() != 0 {
                    self.pc.dec();
                    self.pc.dec();
                    self.next_state = MCycle::new_fake(1);
                }
            }          

            Mnemonic::Outd => {
                self.inner_outd_flags();
            }

            Mnemonic::Otdr => {
                self.inner_outd_flags();
                if self.b() != 0 {
                    self.pc.dec();
                    self.pc.dec();
                    self.next_state = MCycle::new_fake(1);
                }
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
                val &= !(1 << eval_data_access_to_i32(arg1.unwrap()));
                self.store_computed_8bits_value(arg2.unwrap(), val);



                if matches!(arg2, Some(DataAccess::IndexRegister16WithIndex(_, _, _))) {

                    self.expect_additional_fake_cycle = 1;
                }
            }
            
            
            Mnemonic::Ret  => {
                self.pc = self.wz;
                if self.im2_stage == 2 {
                    // end of an IM 2 response: the vector is not on the stack
                    self.im2_stage = 0;
                } else {
                    self.sp.inc();
                    self.sp.inc();
                }

                // conditional ret is far slower
                if arg1.is_some() {
                    self.next_state = MCycle::new_fake(1);
                }
            }

            Mnemonic::Reti => {
                self.pc = self.wz;
                self.sp.inc();
                self.sp.inc();
             
            }

            Mnemonic::Retn => {
                self.pc = self.wz;
                self.sp.inc();
                self.sp.inc();
                // the end of an NMI handler: interrupts as they were before it
                self.iff1 = self.iff2;
            }          
            
            
            Mnemonic::Rl => {
                let prev_c = self.test_flag(FlagTest::C);
                
                let mut  val = self.get_available_8bits_value(arg1.unwrap());
                let c = rla(&mut val, prev_c);
                self.store_computed_8bits_value(arg1.unwrap(), val);

                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag(Z80Flags::C, c);
                self.auto_update_flags(val);
                self.auto_update_flags_x_y(val);
            }

            Mnemonic::Rla => {
                let prev_c = self.test_flag(FlagTest::C);
                
                let mut  val = self.reg8(Reg8::A).value();
                let c = rla(&mut val, prev_c);
                self.reg8_mut(Reg8::A).set(val);
                
                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag(Z80Flags::C, c);
                self.auto_update_flags_x_y(val);
            }
            
            Mnemonic::Rlc => {
                let mut  val = self.get_available_8bits_value(arg1.unwrap());
                let c = rlc(&mut val);
                self.store_computed_8bits_value(arg1.unwrap(), val);

                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag(Z80Flags::C, c);
                self.auto_update_flags(val);
                self.auto_update_flags_x_y(val);

                if matches!(arg1,
                    Some(DataAccess::IndexRegister16WithIndex(_, _, _))) {
                        self.expect_additional_fake_cycle = 1;
                }
            }
            
            Mnemonic::Rlca => {
                let c = self.reg8_mut(Reg8::A).rlc();
                
                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag(Z80Flags::C, c);
                self.auto_update_flags_x_y(self.a());
                
            }
            Mnemonic::Rld => {
                let mut  val = self.get_data_from_pins();
                let a = self.a();

                let a_low = a & 0x0f;
                let pin_low = val & 0x0f;
                let pin_high = (val & 0xf0) >> 4;

                self.set_a( (a & 0xf0) | pin_high);
                self.set_data_on_pins( (pin_low << 4 ) | a_low);

                self.expect_additional_fake_cycle = 1;
                self.auto_update_flags(self.a());
                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
            }
            Mnemonic::Rr => {
                let prev_c = self.test_flag(FlagTest::C);
                
                let mut  val = self.get_available_8bits_value(arg1.unwrap());
                let c = rr(&mut val, prev_c);
                self.store_computed_8bits_value(arg1.unwrap(), val);

                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag(Z80Flags::C, c);
                self.auto_update_flags(val);
                self.auto_update_flags_x_y(val);
            }

            Mnemonic::Rra => {
                let prev_c = self.test_flag(FlagTest::C);
                let c = self.reg8_mut(Reg8::A).rr(prev_c);
                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag(Z80Flags::C, c);
                self.auto_update_flags_x_y(self.a());
            }
            
            Mnemonic::Rrc => {
                
                let mut  val = self.get_available_8bits_value(arg1.unwrap());
                let c = rrc(&mut val);
                self.store_computed_8bits_value(arg1.unwrap(), val);

                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag(Z80Flags::C, c);
                self.auto_update_flags(val);
                self.auto_update_flags_x_y(val);
            }

            
            Mnemonic::Rrca => {
                let c = self.reg8_mut(Reg8::A).rrc();
                
                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag(Z80Flags::C, c);
                self.auto_update_flags_x_y(self.a());
            }
            
            Mnemonic::Rrd => {
                let mut  val = self.get_data_from_pins();
                let a = self.a();

                let a_low = a & 0x0f;
                let pin_low = val & 0x0f;
                let pin_high = (val & 0xf0) >> 4;

                self.set_a( (a & 0xf0) | pin_low);
                self.set_data_on_pins( (a << 4 ) | pin_high);

                self.expect_additional_fake_cycle = 1;
                self.auto_update_flags(self.a());
                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
            }
            Mnemonic::Scf => {
                    let flags = alu::scf(self.a(), self.flags());
                    self.set_flags(flags);
                }
            
            Mnemonic::Rst => {
                self.sp.dec();
                self.sp.dec();
                
                // The decoder gives the target address (opcode bits 5-3 * 8).
                let addr = eval_data_access_to_i32(arg1.unwrap()) as u16;
                debug_assert!(addr & !0x38 == 0, "RST {addr:#x}");
                self.pc.set(addr);
                self.next_state = MCycle::new_fake(1);
            }
            
            
            Mnemonic::Set => {
                let bit = eval_data_access_to_i32(arg1.unwrap()) as u8;
                
                match arg2.unwrap() {
                    DataAccess::Register8(reg8) => {
                        self.reg8_mut(reg8.into()).set_bit(bit);
                    }
                    
                    DataAccess::MemoryRegister16(_)
                    | DataAccess::IndexRegister16WithIndex(_, _, _) => {
                        // data is on the pins
                        let mut data = self.get_data_from_pins();
                        set_bit(&mut data, bit);
                        self.set_data_on_pins(data);
                    }
                    
                    other => {
                        unimplemented!("{:?}", other);
                    }
                }

                if matches!(arg2, Some(DataAccess::IndexRegister16WithIndex(_, _, _))) {
                    self.expect_additional_fake_cycle = 1;
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
                        let left = self.reg16(reg1).value();
                        let right = self.reg16(reg2).value();
                        let (result, flags) = alu::sbc16(left, right, self.flag(Z80Flags::C));
                        self.wz.set(left.wrapping_add(1));
                        self.reg16_mut(reg1).set(result);
                        self.set_flags(flags);

                        let extra_cycles = 2;
                        self.next_state = MCycle::new_fake(extra_cycles);
                    },

                    (None, Some(_)) |
                    (Some(DataAccess::Register8(cpclib_asm::preamble::Register8::A)), _) => {
                        let right = self.get_available_8bits_value(arg2.or(arg1).unwrap());
                        let (result, flags) = alu::sub8(self.a(), right, self.flag(Z80Flags::C));
                        self.reg8_mut(Reg8::A).set(result);
                        self.set_flags(flags);
                    }

                    _ => unimplemented!("{:?}, {:?}", arg1, arg2)
                }
            }
            
                        Mnemonic::Sub => {
                let right = self.get_available_8bits_value(arg2.or(arg1).unwrap());
                let (result, flags) = alu::sub8(self.a(), right, false);
                self.reg8_mut(Reg8::A).set(result);
                self.set_flags(flags);
            },
  
            Mnemonic::Sl1 => {
                let mut  val = self.get_available_8bits_value(arg1.unwrap());
                let c = sl1(&mut val);
                self.store_computed_8bits_value(arg1.unwrap(), val);

                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag(Z80Flags::C, c);
                self.auto_update_flags(val);
                self.auto_update_flags_x_y(val);
            }


            Mnemonic::Sla => {
                let mut  val = self.get_available_8bits_value(arg1.unwrap());
                let c = sla(&mut val);
                self.store_computed_8bits_value(arg1.unwrap(), val);

                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag(Z80Flags::C, c);
                self.auto_update_flags(val);
                self.auto_update_flags_x_y(val);
            }
            
            Mnemonic::Sra => {
                let mut  val = self.get_available_8bits_value(arg1.unwrap());
                let c = sra(&mut val);
                self.store_computed_8bits_value(arg1.unwrap(), val);

                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag(Z80Flags::C, c);
                self.auto_update_flags(val);
                self.auto_update_flags_x_y(val);
            }
            
            
            Mnemonic::Srl => {
                let mut  val = self.get_available_8bits_value(arg1.unwrap());
                let c = srl(&mut val);
                self.store_computed_8bits_value(arg1.unwrap(), val);

                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag(Z80Flags::C, c);
                self.auto_update_flags(val);
                self.auto_update_flags_x_y(val);
            }
            
            Mnemonic::Xor => {
                let mut  val = self.get_available_8bits_value(arg2.or(arg1).unwrap());
                val = val ^ self.reg8(Reg8::A).value();
                self.reg8_mut(Reg8::A).set(val);
                
                self.auto_update_flags(val);
                self.set_flag_off(Z80Flags::H);
                self.set_flag_off(Z80Flags::N);
                self.set_flag_off(Z80Flags::C);
                self.auto_update_flags_x_y(val);
            }
            _ => unimplemented!("{:?}, {:?}, {:?}", mnemonic, arg1, arg2),
        }

        // the internal cycle of the 8 bits ALU instructions on (IX+d)/(IY+d)
        // (ADD has its own above): 19 T-states, 5 us on a CPC
        if matches!(
            mnemonic,
            Mnemonic::Adc | Mnemonic::And | Mnemonic::Cp | Mnemonic::Or | Mnemonic::Sbc | Mnemonic::Sub | Mnemonic::Xor
        ) && matches!(arg2.or(arg1), Some(DataAccess::IndexRegister16WithIndex(_, _, _)))
        {
            self.next_state = MCycle::new_fake(1);
        }

        // rotates and shifts on (IX+d)/(IY+d) take 7 us like RLC, RES and SET
        // (RLC, RES and SET ask for it themselves)
        if matches!(
            mnemonic,
            Mnemonic::Rl | Mnemonic::Rr | Mnemonic::Rrc | Mnemonic::Sla | Mnemonic::Sra | Mnemonic::Srl | Mnemonic::Sl1
        ) && matches!(arg1, Some(DataAccess::IndexRegister16WithIndex(_, _, _)))
        {
            self.expect_additional_fake_cycle = 1;
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
            DataAccess::MemoryRegister16(_) | DataAccess::IndexRegister16WithIndex(_, _, _)=> {
                // Here we do not care at all of which register is used for memory reading
                // Because the CPU has already read the information
                return self.reg8(Reg8::Z).get();
            }
            DataAccess::Expression(expr) => {
                return eval_expr_to_i32(expr) as u8; // cannot fail as the value has been disassembled
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
            DataAccess::MemoryRegister16(_)  | DataAccess::IndexRegister16WithIndex(_, _, _) => {
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
                return eval_expr_to_i32(expr) as u16; // cannot fail as the value has been disassembled
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
            // OUTD/OTDR/OUTI/OTIR read FROM (HL): by the time this runs
            // (right after that read) HL's old value has already served
            // its purpose, so adjusting it immediately is safe.
            Some(Mnemonic::Outd | Mnemonic::Otdr) => {
                self.reg16_mut(Reg16::Hl).dec();
                self.reg8_mut(Reg8::B).dec();
            }
            Some(Mnemonic::Outi | Mnemonic::Otir) => {
                self.reg16_mut(Reg16::Hl).inc();
                self.reg8_mut(Reg8::B).dec();
            }
            // IND/INDR/INI/INIR instead write TO (HL): that write is still
            // to come, as the Read/Input M-cycle's own Write follow-up,
            // and it addresses (HL) using whatever HL holds at that time.
            // Adjusting HL here - before the write - would target the
            // wrong byte (off by one), so only B changes now; HL is
            // adjusted by the Ini/Ind/Inir/Indr handlers in
            // execute_opcode, which run once the write has already
            // latched its address (see MCycle::Write's T1 handler).
            Some(Mnemonic::Ind | Mnemonic::Indr | Mnemonic::Ini | Mnemonic::Inir) => {
                self.reg8_mut(Reg8::B).dec();
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
        
        self.inner_ldi_ldd_flags();
    }
    
    /// Handle the registers for LDI/LDIR
    fn inner_ldi(&mut self) {
        self.reg16_mut(Reg16::Hl).inc();
        self.reg16_mut(Reg16::De).inc();
        self.reg16_mut(Reg16::Bc).dec();
        
        self.inner_ldi_ldd_flags();
    }
    
    fn inner_ldi_ldd_flags(&mut self) {
        self.set_flag_off(Z80Flags::H);
        self.set_flag_off(Z80Flags::N);
        self.set_flag(Z80Flags::P, self.bc()!=0);

        let tmp = self.a().wrapping_add(self.get_data_from_pins());
        // bit 3 of A + data is X, its bit 1 is Y
        self.set_flag(Z80Flags::X, tmp & 1<<3 != 0);
        self.set_flag(Z80Flags::Y, tmp & 1<<1 != 0);
    }


    /// Flags of the block I/O instructions (INI, IND, OUTI, OUTD and their
    /// repeating forms). `data` is the byte transferred, `counter` the value
    /// that is added to it to find the carry and half carry (C+1 for INI,
    /// C-1 for IND, L after the increment or decrement for OUTI and OUTD).
    /// B must already be decremented.
    fn inner_block_io_flags(&mut self, data: u8, counter: u8) {
        let b = self.b();
        let k = u16::from(data) + u16::from(counter);

        let mut flags = (b & (alu::S | alu::Y | alu::X)) | if b == 0 { alu::Z } else { 0 };
        if data & 0x80 != 0 {
            flags |= alu::N;
        }
        if k > 0xff {
            flags |= alu::H | alu::C;
        }
        if ((k as u8 & 7) ^ b).count_ones() % 2 == 0 {
            flags |= alu::PV;
        }
        self.set_flags(flags);
    }

    fn inner_ini_flags(&mut self) {
        let data = self.get_data_from_pins();
        self.inner_block_io_flags(data, self.reg8(Reg8::C).value().wrapping_add(1));
    }

    fn inner_ind_flags(&mut self) {
        let data = self.get_data_from_pins();
        self.inner_block_io_flags(data, self.reg8(Reg8::C).value().wrapping_sub(1));
    }

    fn inner_outi_flags(&mut self) {
        // registers modification already done
        let data = self.get_data_from_pins();
        self.inner_block_io_flags(data, self.reg8(Reg8::L).value());
    }

    fn inner_outd_flags(&mut self) {
        // registers modification already done
        let data = self.get_data_from_pins();
        self.inner_block_io_flags(data, self.reg8(Reg8::L).value());
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

    pub(crate) fn force_interruption_received(&mut self, val: bool) {
        self.interruption_received = val;
    }

    /// Handle interruption if any
    // must be called only at the end of an instruction
    fn instruction_finished(&mut self) {
        if !self.next_state.is_interruption() {
            self.record_idle_tail();
        }
        // manage the flipflip for interruption

        if self.interruption_received {
   // dbg!("z80 received interrupt request", &self.iff1, &self.iff2, &self.next_state, self.pc());


        }
        let shadowed = self.ei_shadow > 0;
        if shadowed {
            self.ei_shadow -= 1;
        }
        if std::mem::take(&mut self.nmi_pending) {
            // NMI: not maskable; IFF2 keeps IFF1 for the RETN that ends it
            self.iff1 = false;
            self.nmi_in_progress = true;
            self.next_state = MCycle::new_interruption();
            return;
        }
        if self.interruption_received
        && self.iff1
        && !shadowed/*
        && matches!(
            self.next_state,
            MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None)
        )*/ /* this test was a nonsense and wrong, it is up to the caller to be sure to call it at the right time*/
             {
            //dbg!("z80 treat interrupt request");
            self.iff1 = false;
            self.iff2 = false;
            self.interruption_received = false;
            
            // In IM 0 the interrupting device puts an instruction on the
            // bus; nothing does on a CPC (the gate array only pulls the
            // interrupt line down), so the pull-ups give &FF, RST &38: the
            // same thing as IM 1.
            debug_assert!(
                matches!(self.im, InterruptMode::Mode0 | InterruptMode::Mode1),
                "Interruption {:?} unimplemented",
                self.im
            );
            
           self.next_state = MCycle::new_interruption();
     //      dbg!(&self.next_state);
        } else {
            self.next_state = Default::default();
            // /INT is a level, looked at again by every instruction: a request the gate array
            // took back meanwhile (RMR bit 4 while interrupts were disabled) must not
            // come back as an interrupt when EI is executed
            if !shadowed {
                self.interruption_received = false;
            }
        }
    }
    
    /// Work out how many T-states the instruction that just ended left idle
    /// in its last microsecond.
    fn record_idle_tail(&mut self) {
        let (prefix, op) = (self.instr_prefix, self.instr_op);
        let pc = self.pc.get();
        let start = self.instr_start_pc;
        let sequential_len: Option<u16> = if prefix == OpcodePrefix::None {
            match op {
                0x10 | 0x20 | 0x28 | 0x30 | 0x38 => Some(2),
                0xc0 | 0xc8 | 0xd0 | 0xd8 | 0xe0 | 0xe8 | 0xf0 | 0xf8 => Some(1),
                _ if op >= 0xc0 && op & 7 == 4 && op != 0xcd => Some(3),
                _ => None,
            }
        } else {
            None
        };
        let taken = sequential_len.map_or(false, |len| pc != start.wrapping_add(len));
        let repeat = prefix == OpcodePrefix::Ed
            && matches!(op, 0xb0..=0xb3 | 0xb8..=0xbb)
            && pc == start;
        self.last_idle_t = super::tstates::idle_tail(prefix, op, taken, repeat);
    }

    /// T-states the last executed instruction left idle in its last microsecond.
    pub fn last_idle_t(&self) -> u8 {
        self.last_idle_t
    }

    /// The pin an M1 cycle requests with: IORQ for an interrupt acknowledge, MREQ for an
    /// opcode fetch and for an NMI's (an ordinary read of the opcode, then ignored).
    fn m1_request_pin(&self) -> Z80Pin {
        if self.nmi_in_progress { Z80Pin::Neg_MREQ } else { self.next_state.read_pin_type().unwrap() }
    }

    /// Request a non-maskable interrupt (the falling edge of /NMI): taken at the end of the
    /// instruction, whatever IFF1 says.
    pub fn trigger_nmi(&mut self) {
        self.nmi_pending = true;
    }

    /// The low byte of the IM 2 vector the interrupting device gives, `None` for the floating
    /// bus (&FF).
    pub fn set_interrupt_vector(&mut self, vector: Option<u8>) {
        self.interrupt_vector = vector;
    }

    /// The level of the /NMI pin: its falling edge is latched, the NMI taken at the end of the
    /// instruction.
    pub fn set_nmi_line(&mut self, level: bool) {
        if !level && self.get_pin(Z80Pin::Neg_NMI) {
            self.nmi_pending = true;
        }
        self.set_pin(Z80Pin::Neg_NMI, level);
    }

    /// Handcraft a call to let the CPU jump in 0x38
    fn simulate_call_0x38_after_fetch(&mut self) {      
        self.opcode_information = Some(&OPCODE_PREFIX[0xcd]);
        self.ir.set(0xcd);
        if std::mem::take(&mut self.nmi_in_progress) {
            // NMI: a call to &0066 right after its M1 (11 T-states: 3 us here)
            self.wz.set(0x66);
            self.im2_stage = 0;
            self.next_state = MCycle::Write(TState::T1, 0);
            return;
        }
        let im2 = self.im == InterruptMode::Mode2;
        self.wz.set(if im2 { 0 } else { 0x38 });
        self.im2_stage = u8::from(im2);
        // the response of an IM 1 interrupt lasts 5 us (ACCC 27.4), two more
        // than the M1 and the two pushes
        // ...but one microsecond less when the instruction left 2 or more idle
        // T-states in its last one: the acknowledge starts inside it
        // IM 2 reads the vector in two more microseconds: 6 us in all (ACCC
        // 27.5, CPCSyntaxError's 19 T-states), so the wait is shorter
        let extra: u8 = match (im2, self.last_idle_t >= 2) {
            (false, false) => 2,
            (false, true) | (true, false) => 1,
            (true, true) => 0,
        };
        self.next_state = if extra == 0 {
            MCycle::Write(TState::T1, 0)
        } else {
            MCycle::InterruptWait(TState::T1, extra)
        };
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

    pub fn iff1(&self) -> bool {
        self.iff1
    }

    pub fn iff2(&self) -> bool {
        self.iff2
    }

    /// Force both interrupt flip-flops.
    pub fn set_iffs(&mut self, iff1: bool, iff2: bool) {
        self.iff1 = iff1;
        self.iff2 = iff2;
    }

    pub fn im(&self) -> InterruptMode {
        self.im
    }

    /// Force the interrupt mode (e.g. when restoring a snapshot, which
    /// records it directly rather than as the `IM n` that set it).
    pub fn set_im(&mut self, im: InterruptMode) {
        self.im = im;
    }

    /// Force I and R (the refresh counter) directly - as opposed to how
    /// they're normally reached, through `LD A,I`/`LD A,R`/`LD I,A`/`LD R,A`
    /// or the M1-cycle refresh itself.
    pub fn set_i_and_r(&mut self, i: u8, r: u8) {
        self.reg8_mut(Reg8::I).set(i);
        self.reg8_mut(Reg8::R).set(r);
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





    pub fn set_a(&mut self, value: u8) {
        self.reg16_mut(Reg16::Af).high_mut().set(value)
    }
    pub fn set_b(&mut self, value: u8) {
        self.reg16_mut(Reg16::Bc).high_mut().set(value)
    }
    pub fn set_d(&mut self, value: u8) {
        self.reg16_mut(Reg16::De).high_mut().set(value)
    }
    pub fn set_h(&mut self, value: u8) {
        self.reg16_mut(Reg16::Hl).high_mut().set(value)
    }
    pub fn set_ixh(&mut self, value: u8) {
        self.reg16_mut(Reg16::Ix).high_mut().set(value)
    }
    pub fn set_iyh(&mut self, value: u8) {
        self.reg16_mut(Reg16::Iy).high_mut().set(value)
    }



    pub fn set_c(&mut self, value: u8) {
        self.reg16_mut(Reg16::Bc).low_mut().set(value)
    }
    pub fn set_e(&mut self, value: u8) {
        self.reg16_mut(Reg16::De).low_mut().set(value)
    }
    pub fn set_l(&mut self, value: u8) {
        self.reg16_mut(Reg16::Hl).low_mut().set(value)
    }
    pub fn set_ixl(&mut self, value: u8) {
        self.reg16_mut(Reg16::Ix).low_mut().set(value)
    }
    pub fn set_iyl(&mut self, value: u8) {
        self.reg16_mut(Reg16::Iy).low_mut().set(value)
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
        // ACCC 4.4.4: IORQ and /WR of an output follow the real I/O cycle (T2 to T3), which may
        // run on into the next M-cycle of this model (OUT (C),r: into its 4th microsecond)
        if let Some(window) = self.io_window {
            if window.t > window.end {
                self.set_pin(Z80Pin::Neg_IORQ, true);
                self.set_pin(Z80Pin::Neg_WR, true);
                self.io_window = None;
            } else {
                let active = window.t >= window.start;
                self.set_pin(Z80Pin::Neg_IORQ, !active);
                self.set_pin(Z80Pin::Neg_WR, !active);
                self.io_window = Some(IoWindow { t: window.t + 1, ..window });
            }
        }
      //  dbg!(&self.next_state)
        if  !self.get_pin(Z80Pin::Neg_INT) && !self.next_state.is_interruption() {
            self.interruption_received |= true;
  //          dbg!("Z80 see interrupt pin", self.interruption_received, self.iff1);
        }
        match self.next_state {
            // First state of the fetch process
            MCycle::FetchOpcode(TState::T1, _, _) | MCycle::InterruptM1(TState::T1) => {
                self.expect_additional_fake_cycle = 0;

                // Every M1 cycle - an opcode or a prefix byte, or the
                // interrupt acknowledge - is followed by a refresh cycle that
                // counts in R; the operand bytes of an instruction, and the
                // displacement and opcode of DD CB / FD CB, are ordinary
                // reads and do not.
                let is_m1 = match self.next_state {
                    MCycle::FetchOpcode(_, nb, prefix) => nb == 0 && !prefix.is_for_indexed_opcode(),
                    _ => true
                };
                if is_m1 {
                    let r = self.r.get();
                    self.r.set((r & 0x80) | (r.wrapping_add(1) & 0x7f));
                }

                self.set_address_on_pins(self.pc.get());
                if matches!(self.next_state, MCycle::FetchOpcode(_, 0, OpcodePrefix::None)) {
                    self.instr_start_pc = self.pc.get();
                }
                // pc is incremented in T2 in a real CPU
                if !self.next_state.is_interruption() {
                    self.pc.inc();
                }
                
                let request = self.m1_request_pin();
                self.set_pin(request, false);
                self.set_pin(Z80Pin::Neg_RD, false);
                // the operand bytes are ordinary reads: /M1 stays high (Z80 peripherals, such as
                // a CTC watching for RETI, tell the opcodes by it)
                self.set_pin(Z80Pin::Neg_M1, !is_m1);
                self.set_pin(Z80Pin::Neg_RFSH, true);
                
                self.next_state.inc_state();
            }
            MCycle::InterruptM1(TState::T2) => {
                self.set_pin(Z80Pin::Neg_RD, true);
                let request = self.m1_request_pin();
                self.set_pin(request, true);
                self.set_pin(Z80Pin::Neg_M1, true);
                self.set_pin(Z80Pin::Neg_RFSH, false);
                self.set_address_on_pins(self.refresh_addr);
                self.next_state.inc_state();

            }
            // Second step / may be delayed if wait is activated
            MCycle::FetchOpcode(TState::T2, nb, _)   => {
                self.set_pin(Z80Pin::Neg_RD, true);
                self.set_pin(self.next_state.read_pin_type().unwrap(), true);
                self.set_pin(Z80Pin::Neg_M1, true);
                
          
                let byte = self.get_data_from_pins();
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
            
            MCycle::FetchOpcode(TState::T3, _, _)  | MCycle::InterruptM1(TState::T3) => {
                // we do nothing here
                // but, IRL, the decoding appears there
                self.next_state.inc_state();
                self.set_pin(Z80Pin::Neg_MREQ, false); // is it normal there ?
            }

            MCycle::InterruptM1(TState::T4) => {
                self.set_pin(Z80Pin::Neg_MREQ, true);
                // /M1 is only low during T1-T2 (already released in T2)
                self.set_pin(Z80Pin::Neg_M1, true);
                self.set_pin(Z80Pin::Neg_RFSH, true);
                self.simulate_call_0x38_after_fetch();
            }
            
            MCycle::FetchOpcode(TState::T4, nb, prefix) => {
                self.set_pin(Z80Pin::Neg_MREQ, true);
                // /M1 is only low during T1-T2 (already released in T2): it
                // used to be driven low again here and stay low through the
                // instruction's following memory/I-O cycles.
                self.set_pin(Z80Pin::Neg_M1, true);
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
                                self.next_state = self.input_start_state();
                            } else {
                                self.next_state = MCycle::Read(TState::T1, 0);
                            }
                        } else if opcode_information.expect_write() {
                            if opcode_information.write_to_pins {
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
                
              //  dbg!("READ ADDR={:x}", address);
                
                debug_assert!(self.get_pin(self.next_state.read_pin_type().unwrap()));
                debug_assert!(self.get_pin(Z80Pin::Neg_RD));
                debug_assert!(self.get_pin(Z80Pin::Neg_WR));
                
                self.off(self.next_state.read_pin_type().unwrap());
                self.off(Z80Pin::Neg_RD);
                self.next_state.inc_state();
            }
            MCycle::Read(TState::T2, nb) => {
                let byte = self.get_data_from_pins();
              //  dbg!("DATA={:x}", byte);
                self.store_currently_read_byte(byte, nb);
                
                self.on(self.next_state.read_pin_type().unwrap());
                self.on(Z80Pin::Neg_RD);
                self.next_state.inc_state();
            }
            // an input holds IORQ and /RD until the data bus is latched, on T3 (see `MCycle::Input`'s T4)
            MCycle::Input(TState::T2, _) => {
                self.next_state.inc_state();
            }
            /// simulate wait
            MCycle::Read(TState::T3, nb) | MCycle::Input(TState::T3, nb) => {
                self.next_state.inc_state();
            }
            MCycle::Read(TState::T4, nb) | MCycle::Input(TState::T4, nb) => {
                if self.next_state.is_input() {
                    let byte = self.get_data_from_pins();
                    self.store_currently_read_byte(byte, nb);
                    self.on(self.next_state.read_pin_type().unwrap());
                    self.on(Z80Pin::Neg_RD);
                }
                self.post_read_action();
                
                let opcode_info = self.opcode_information.as_ref().unwrap().clone();
                if nb + 1 == opcode_info.nb_bytes_to_read {
                    if 0 != opcode_info.nb_bytes_to_write {
                        if self.must_execute_instruction_in_middle() {
                            // compute only: `execute_step()` doesn't retire
                            // the instruction here, the write still follows
                            self.execute_step();
                        }
                        if opcode_info.write_to_pins {
                            // OUTI/OUTD: the port is written on the 5th microsecond
                            self.next_state = if self.is_block_output() {
                                MCycle::IoWait(TState::T1, true)
                            } else {
                                MCycle::Output(TState::T1, 0)
                            };
                        } else {
                            self.next_state = MCycle::Write(TState::T1, 0);
                        }
                    } else {
                        self.execute_step();

                        // `execute_step()` -> `instruction_finished()` may
                        // have just accepted a pending interrupt, setting
                        // `next_state` to `MCycle::new_interruption()`
                        // (and clearing iff1 as a side effect of doing so).
                        // That must be preserved here exactly like the
                        // `Fake(T1, _)` case already is - otherwise the
                        // interrupt is silently swallowed: iff1 stays
                        // cleared forever, since the CPU never actually
                        // reaches the &0038 vector to run the handler that
                        // would `ei` again.
                        if !matches!(
                            self.next_state,
                            MCycle::Fake(TState::T1, _)
                        ) && !self.next_state.is_interruption() {
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
                    
                
                debug_assert!(self.get_pin(self.next_state.write_pin_type().unwrap()));
                debug_assert!(self.get_pin(Z80Pin::Neg_RD));
                debug_assert!(self.get_pin(Z80Pin::Neg_WR));

                // For memory writes, MREQ becomes active one T-state before WR.
                // I/O writes assert IORQ and WR together on T1: the effect of an
                // OUT lands on the start of the 3rd (OUT (C),r8 / OUT (n),A) or
                // 5th (OUTI/OUTD) microsecond of the instruction (ACCC 4.4.3).
                if !self.next_state.is_output() {
                    self.off(self.next_state.write_pin_type().unwrap());
                }

                // Preload the data bus before WR is asserted.
                let byte = self.get_byte_to_write_from_opcode_information(nb);
                self.set_data_on_pins(byte);

                if self.next_state.is_output() {
                    let window = self.output_window();
                    if window.start == 0 {
                        self.off(self.next_state.write_pin_type().unwrap());
                        self.off(Z80Pin::Neg_WR);
                    }
                    self.io_window = Some(IoWindow { t: 1, ..window });
                }
                
                self.next_state.inc_state();
            }
            MCycle::Write(TState::T2, nb) | MCycle::Output(TState::T2, nb) => {
                // (an output's IORQ and /WR follow `io_window`)
                if !self.next_state.is_output() {
                    self.off(Z80Pin::Neg_WR); // or is it the writer that does that ?
                }
                
                self.next_state.inc_state();
            }
            
            // simulates a wait state
            MCycle::Write(TState::T3, nb) | MCycle::Output(TState::T3, nb) => {
                self.next_state.inc_state();
            }
            
            MCycle::Write(TState::T4, nb) | MCycle::Output(TState::T4, nb) => {
                if !self.next_state.is_output() {
                    self.on(self.next_state.write_pin_type().unwrap());
                    self.on(Z80Pin::Neg_WR);
                }
                
                if nb == self.opcode_information.as_ref().unwrap().nb_bytes_to_write - 1 {
                    // TODO take into account those that do a write after the read (if that exists)
                    if !self.must_execute_instruction_in_middle() {
                        self.execute_step();
                    } else {
                        // Read-modify-write: the compute step already ran
                        // after the read, and this write was the last
                        // M-cycle - the instruction retires now.
                        self.opcode_information = None;
                        // ...unless an internal cycle still follows (RLD,
                        // (IX+d) rotates, RES, SET): a pending interrupt is
                        // only accepted once that cycle is over
                        if self.expect_additional_fake_cycle == 0 {
                            self.instruction_finished();
                        }
                    }
                    if self.im2_stage == 1 {
                        // IM 2: the return address is pushed, now fetch the
                        // handler address at I<<8 | #FF (a RET that reads
                        // there instead of the stack)
                        self.im2_stage = 2;
                        self.opcode_information = Some(&OPCODE_PREFIX[0xc9]);
                        self.ir.set(0xc9);
                        self.next_state = MCycle::Read(TState::T1, 0);
                    }
                    // See the matching comment in the `Read`/`Input` T4
                    // handler: `execute_step()` may have just accepted a
                    // pending interrupt via `instruction_finished()`, and
                    // that must not be overwritten here either.
                    if self.im2_stage != 2 && !matches!(
                        self.next_state,
                        MCycle::Fake(TState::T1, _)
                    ) && !self.next_state.is_interruption() {
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
            
            MCycle::InterruptWait(_, _) => {
                self.next_state.inc_state();
            }

            MCycle::IoWait(_, _) => {
                // nothing happens on the buses; the I/O cycle starts a
                // microsecond later (ACCC 4.4.3)
                self.next_state.inc_state();
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
        self.io_window = None;
        
        self.sp.set(0xffff);
        self.pc.set(0x0000);
        self.af.set(0xffff);
        self.iff1 = false;
        self.iff2 = false;
        // Real Z80 hardware always resets into Interrupt Mode 0 - firmware
        // explicitly executes `IM 1` as one of its first instructions
        // (the whole CPC interrupt architecture depends on IM1's constant
        // 0x0038 vector). Defaulting the reset state to Mode1 skipped that
        // and hid whatever a real machine's Mode 0 window would do if an
        // interrupt happened to land before firmware got there.
        self.im = InterruptMode::Mode0;
    }


}

/// SSM (ScreenShot Management) support - see SSM-STANDARD-EN.pdf.
///
/// SSM repurposes the 177 "neutral" ED-prefixed suffix bytes that don't
/// correspond to any real Z80A instruction: two of them executed back to
/// back (ED <LL> immediately followed by ED <HH>) form a 16-bit request
/// code (HH*256+LL) that an SSM-aware emulator can react to (e.g. saving a
/// screenshot or a snapshot). Any other fully-decoded instruction in
/// between - including a real (non-neutral) ED instruction - cancels the
/// pending pair.
impl Z80 {
    /// Whether `byte`, used as an ED-prefix suffix, is one of the codes the
    /// SSM standard reserves as "neutral" and therefore eligible to take
    /// part in an SSM request. The ranges are the standard's, not simply
    /// "every undefined ED opcode": #FE/#FF are deliberately excluded from
    /// this pool since they're reserved to build the FFxx meta-codes.
    fn is_ssm_neutral_ed_suffix(byte: u8) -> bool {
        matches!(byte,
            0x00..=0x3F
            | 0x7F..=0x9F
            | 0xA4..=0xA7
            | 0xAC..=0xAF
            | 0xB4..=0xB7
            | 0xBC..=0xBF
            | 0xC0..=0xFD
        )
    }

    /// Feed one just-decoded ED-prefix suffix byte through the SSM pair
    /// detector.
    fn detect_ssm(&mut self, suffix: u8) {
        if Self::is_ssm_neutral_ed_suffix(suffix) {
            match self.pending_ssm_low.take() {
                Some(low) => {
                    let code = (suffix as u16) << 8 | low as u16;
                    self.ssm_codes.push(code);
                },
                None => {
                    self.pending_ssm_low = Some(suffix);
                }
            }
        } else {
            // A real ED instruction breaks a pending pair too.
            self.pending_ssm_low = None;
        }
    }

    /// Drain and return every SSM code detected since the last call.
    pub fn take_ssm_codes(&mut self) -> Vec<u16> {
        std::mem::take(&mut self.ssm_codes)
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

        force_front(&mut cpu);
        
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

        assert_eq!(cpu.reg8(Reg8::Z).get(), 0x12);
        
        assert_eq!(
            cpu.next_state,
            MCycle::FetchOpcode(TState::T1, 2, OpcodePrefix::None)
        );
        cpu.tick();
        cpu.set_data_on_pins(0x34);
        cpu.tick();
        cpu.tick();
        cpu.tick();

        assert_eq!(cpu.reg8(Reg8::W).get(), 0x34);
        assert_eq!(cpu.reg16(Reg16::Wz).get(), 0x3412);
        
        // Execute the single write cycle for LD (nn),A
        assert_eq!(cpu.next_state, MCycle::Write(TState::T1, 0));
        cpu.tick();
        assert_eq!(cpu.get_address_from_pins(), 0x3412);
        assert!(!cpu.get_pin(Z80Pin::Neg_MREQ));
        assert!(cpu.get_pin(Z80Pin::Neg_WR));
        cpu.tick();
        assert_eq!(cpu.get_data_from_pins(), 0xff);
        assert!(!cpu.get_pin(Z80Pin::Neg_MREQ));
        assert!(!cpu.get_pin(Z80Pin::Neg_WR));
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

        force_front(&mut cpu);
        
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
        assert_eq!(cpu.get_pin(Z80Pin::Neg_M1), true, "the operand is an ordinary read: no /M1");
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
        assert_eq!(cpu.get_pin(Z80Pin::Neg_M1), true);
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
