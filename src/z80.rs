use crate::chip::*;

use num::integer::Integer;
use num::traits::ops::overflowing::OverflowingAdd;
use num::traits::ops::overflowing::OverflowingSub;
use num::One;

use std::fmt;
use std::fmt::Debug;
use std::mem::swap;

use cpclib_asm::preamble::*;
use lazy_static::lazy_static;
use regex::Regex;

/// Possible states for a cycle
/// Warning: Wait state does not count
#[derive(Debug, PartialEq)]
pub(crate) enum TState {
    T1,
    T2,
    T3,
    T4,
}

impl Default for TState {
    fn default() -> Self {
        TState::T1
    }
}

/// This enumeration aims at helping to treat the
/// prefixed operations
#[derive(Debug, PartialEq, Copy, Clone)]
pub(crate) enum OpcodePrefix {
    None,
    Dd,
    Fd,
    Cb,
    Ed,
}

impl Default for OpcodePrefix {
    fn default() -> Self {
        OpcodePrefix::None
    }
}

// Generate the various lookup tables needed to control the opcodes execution
// rust is currently unable to do that in a static way
lazy_static! {
    static ref OPCODE_PREFIX: [OpcodeInformation; 256] =
        array_init::array_init(|i| OpcodePrefix::None.opcode_information(i as u8));
    static ref OPCODE_PREFIX_CB: [OpcodeInformation; 256] =
        array_init::array_init(|i| OpcodePrefix::Cb.opcode_information(i as u8));
    static ref OPCODE_PREFIX_DD: [OpcodeInformation; 256] =
        array_init::array_init(|i| OpcodePrefix::Dd.opcode_information(i as u8));
    static ref OPCODE_PREFIX_ED: [OpcodeInformation; 256] =
        array_init::array_init(|i| OpcodePrefix::Ed.opcode_information(i as u8));
    static ref OPCODE_PREFIX_FD: [OpcodeInformation; 256] =
        array_init::array_init(|i| OpcodePrefix::Fd.opcode_information(i as u8));
}

/// Contains all the informations needed to drive the next
/// cycles of the instruction
#[derive(Debug, PartialEq)]
pub(crate) struct OpcodeInformation {
    /// Prefix for the current opcode
    prefix: OpcodePrefix,
    /// Number of bytes of the opcode that need to be fetched
    nb_additional_opcode_bytes: u8,
    /// Location of the reading address
    read_address: AddressSource,
    /// Location of the write address (memory or output)
    write_address: AddressSource,
    /// Write data
    write_data: DataSource,
    /// Number of bytes to read from the address
    nb_additional_bytes_to_read: u8,
    /// Number of bytes the instruction request to write (in memory or chip)
    nb_bytes_to_write: u8,
    /// allow to difference memory and port addressing
    addresses_memory: bool,
    /// String representation
    repr: &'static str,
    //   /// Token just in case it is usefull (to be removed if not)
    token: cpclib_asm::preamble::Token,
}

impl OpcodeInformation {
    /// Fetch is finished for opcode of 1 byte with no memory/port access
    fn fetch_finished(&self) -> bool {
        self.nb_additional_opcode_bytes == 0
    }

    /// Check if CPU as to read from bus
    fn expect_read(&self) -> bool {
        self.nb_additional_bytes_to_read != 0
    }

    /// Check if CPU as to write in bus
    fn expect_write(&self) -> bool {
        self.nb_bytes_to_write != 0
    }
}

impl OpcodePrefix {
    /// Retreive all the information needed to data collection for the current opcode
    fn opcode_information(&self, opcode: u8) -> OpcodeInformation {
        let opcode = opcode as usize;

        let repr = self.disassembling_table()[opcode];
        let token = cpclib_asm::parser::parse_token(if repr.is_empty() { "NOP" } else { repr })
            .unwrap()
            .1;
        let addresses_memory = !(token.is_input_opcode() | token.is_output_opcode());

        // For instructions reading from memory, get the location of the address
        let read_address = if token.is_input_opcode() {
            self.input_address_source(&token)
        } else if token.is_output_opcode() {
            if token.mnemonic_arg1().is_none() {
                // The instructions with no arg are those that read from memory
                AddressSource::Register16(cpclib_asm::preamble::Register16::Hl)
            } else {
                AddressSource::None
            }
        } else if addresses_memory {
            self.read_address_source(opcode)
        } else {
            AddressSource::None
        };

        // For instructions writting ot memory, get the location of address
        let write_address = if token.is_output_opcode() {
            self.output_address_source(&token)
        } else if addresses_memory {
            self.write_address_source(opcode)
        } else {
            AddressSource::None
        };

        /// Location of the byte to write on data
        let write_data: DataSource = if token.is_output_opcode() {
            match token.mnemonic_arg2() {
                Some(position) => position.into(),
                None => DataSource::Pins, // the address is already on the pins
            }
        } else {
            if write_address == AddressSource::None {
                DataSource::None
            } else {
                token.mnemonic_arg2().unwrap().into()
            }
        };

        // get the number of bytes needed to build the complete opcode
        let nb_additional_bytes_to_read = if token.is_output_opcode() {
            if let Some(DataAccess::PortC) = token.mnemonic_arg1() {
                0
            } else {
                1 // port n belongs to the opcode
            }
        } else if token.is_input_opcode() {
            if let Some(DataAccess::PortC) = token.mnemonic_arg2() {
                0
            } else {
                1 // port n
            }
        } else {
            match read_address {
                AddressSource::None => 0,
                _ => 1, // TODO really set the right value
            }
        };

        let nb_bytes_to_write = match write_data {
            DataSource::None => 0,
            DataSource::Register16(_) => 2,
            DataSource::IndexRegister16(_) => 2,
            DataSource::Register8(_) => 1,
            DataSource::IndexRegister8(_) => 1,
            DataSource::Immediate => 1,           // XXX not sure of that
            DataSource::MemoryRegister16(_) => 1, // XXX not sure of that see if we keep
            DataSource::Pins => 1,                // the byte is already on the pins
        };

        OpcodeInformation {
            prefix: self.clone(),
            nb_additional_opcode_bytes: self.opcode_bytes_to_read(opcode),
            read_address,
            write_address,
            write_data,
            nb_additional_bytes_to_read,
            nb_bytes_to_write,
            repr,
            addresses_memory,
            token,
        }
    }

    /// Return the disassembling LUT table depending on the prefix
    fn disassembling_table(&self) -> &[&'static str; 256] {
        // the decoding table depends on the prefix
        match self {
            OpcodePrefix::None => &cpclib_asm::disass::TABINSTR,
            OpcodePrefix::Cb => &cpclib_asm::disass::TABINSTRCB,
            OpcodePrefix::Dd => &cpclib_asm::disass::TABINSTRDD,
            OpcodePrefix::Ed => &cpclib_asm::disass::TABINSTRED,
            OpcodePrefix::Fd => &cpclib_asm::disass::TABINSTRFD,
        }
    }

    /// Serve to generate lookup tables at start
    fn opcode_bytes_to_read(self, i: usize) -> u8 {
        lazy_static! {
            static ref HEIGHT_BITS: Regex = Regex::new(r"([^n]nn[^n])|([^n]nn$)").unwrap();
            static ref SIXTEEN_BITS: Regex = Regex::new(r"[^n]nnnn[^n]|([^n]nnnn$)").unwrap();
        }

        let repr = self.disassembling_table()[i];

        let nb_height_bits_args = HEIGHT_BITS.captures_iter(repr).count();
        let nb_sixteen_bits_args = SIXTEEN_BITS.captures_iter(repr).count();

        (nb_height_bits_args + 2 * nb_sixteen_bits_args) as u8
    }

    fn read_address_source(self, i: usize) -> AddressSource {
        lazy_static! {
            static ref INDEXED_IY: Regex = Regex::new(r"\(IY\+nn\)$").unwrap();
            static ref INDEXED_IX: Regex = Regex::new(r"\(IX\+nn\)$").unwrap();
            static ref IN_HL: Regex = Regex::new(r"\(HL\)$").unwrap();
            static ref IN_BC: Regex = Regex::new(r"\(BC\)$").unwrap();
            static ref IN_DE: Regex = Regex::new(r"\(DE\)$").unwrap();
            static ref IN_AF: Regex = Regex::new(r"\(AF\)$").unwrap();
            static ref IN_IX: Regex = Regex::new(r"\(IX\)$").unwrap();
            static ref IN_IY: Regex = Regex::new(r"\(IY\)$").unwrap();
            static ref IMMEDIATE: Regex = Regex::new(r"\(nnnn\)$").unwrap();
        }

        let repr = self.disassembling_table()[i];

        if INDEXED_IY.is_match(repr) {
            AddressSource::Indexed(cpclib_asm::preamble::IndexRegister16::Iy)
        } else if INDEXED_IX.is_match(repr) {
            AddressSource::Indexed(cpclib_asm::preamble::IndexRegister16::Ix)
        } else if IN_AF.is_match(repr) {
            AddressSource::Register16(cpclib_asm::preamble::Register16::Af)
        } else if IN_HL.is_match(repr) {
            AddressSource::Register16(cpclib_asm::preamble::Register16::Hl)
        } else if IN_DE.is_match(repr) {
            AddressSource::Register16(cpclib_asm::preamble::Register16::De)
        } else if IN_BC.is_match(repr) {
            AddressSource::Register16(cpclib_asm::preamble::Register16::Bc)
        } else if IN_IX.is_match(repr) {
            AddressSource::IndexRegister16(cpclib_asm::preamble::IndexRegister16::Ix)
        } else if IN_IY.is_match(repr) {
            AddressSource::IndexRegister16(cpclib_asm::preamble::IndexRegister16::Iy)
        } else if IMMEDIATE.is_match(repr) {
            AddressSource::Immediate
        } else {
            AddressSource::None
        }
    }

    fn input_address_source(self, token: &Token) -> AddressSource {
        if !token.is_input_opcode() {
            return AddressSource::None;
        }

        match token.mnemonic_arg2() {
            Some(DataAccess::PortC) | None => {
                AddressSource::Register16(cpclib_asm::preamble::Register16::Bc)
            }
            Some(DataAccess::PortN(_)) => AddressSource::Immediate,
            _ => unreachable!("{:?}", token),
        }
    }

    /// Provide the location that contains the address to write in the address bus to access a chip
    fn output_address_source(self, token: &Token) -> AddressSource {
        if !token.is_output_opcode() {
            return AddressSource::None;
        }

        match token.mnemonic_arg1() {
            Some(DataAccess::PortC) | None => {
                AddressSource::Register16(cpclib_asm::preamble::Register16::Bc)
            }
            Some(DataAccess::PortN(_)) => AddressSource::Immediate,
            _ => unreachable!("{:?}", token),
        }
    }

    /// Provide the location of the address to put on the address bus to write in memory
    fn write_address_source(self, i: usize) -> AddressSource {
        lazy_static! {
            static ref INDEXED_IY: Regex = Regex::new(r"\(IY\+nn\)[^$]").unwrap();
            static ref INDEXED_IX: Regex = Regex::new(r"\(IX\+nn\)[^$]").unwrap();
            static ref IN_HL: Regex = Regex::new(r"\(HL\)[^$]").unwrap();
            static ref IN_BC: Regex = Regex::new(r"\(BC\)[^$]").unwrap();
            static ref IN_DE: Regex = Regex::new(r"\(DE\)[^$]").unwrap();
            static ref IN_AF: Regex = Regex::new(r"\(AF\)[^$]").unwrap();
            static ref IN_IX: Regex = Regex::new(r"\(IX\)[^$]").unwrap();
            static ref IN_IY: Regex = Regex::new(r"\(IY\)[^$]").unwrap();
            static ref IMMEDIATE: Regex = Regex::new(r"\(nnnn\)[^$]").unwrap();
        }

        let repr = self.disassembling_table()[i];

        if INDEXED_IY.is_match(repr) {
            AddressSource::Indexed(cpclib_asm::preamble::IndexRegister16::Iy)
        } else if INDEXED_IX.is_match(repr) {
            AddressSource::Indexed(cpclib_asm::preamble::IndexRegister16::Ix)
        } else if IN_AF.is_match(repr) {
            AddressSource::Register16(cpclib_asm::preamble::Register16::Af)
        } else if IN_HL.is_match(repr) {
            AddressSource::Register16(cpclib_asm::preamble::Register16::Hl)
        } else if IN_DE.is_match(repr) {
            AddressSource::Register16(cpclib_asm::preamble::Register16::De)
        } else if IN_BC.is_match(repr) {
            AddressSource::Register16(cpclib_asm::preamble::Register16::Bc)
        } else if IN_IX.is_match(repr) {
            AddressSource::IndexRegister16(cpclib_asm::preamble::IndexRegister16::Ix)
        } else if IN_IY.is_match(repr) {
            AddressSource::IndexRegister16(cpclib_asm::preamble::IndexRegister16::Iy)
        } else if IMMEDIATE.is_match(repr) {
            AddressSource::Immediate
        } else {
            AddressSource::None
        }
    }
}

/// Where the CPU searches the address information to be put on the address line
#[derive(Debug, PartialEq, Copy, Clone)]
enum AddressSource {
    /// No address handled by the opcode
    None,
    /// The address has been read by the CPU and is stored in extra bytes
    Immediate,
    /// The address is contained inside a 16 bits register
    Register16(cpclib_asm::preamble::Register16),
    /// The address is contained in a 16 bits indexed regiser
    IndexRegister16(cpclib_asm::preamble::IndexRegister16),
    /// The delta has already been read by the CPU
    Indexed(cpclib_asm::preamble::IndexRegister16),
}

/// Where the CPU searches the information to link with the data bus
#[derive(Debug, PartialEq, Copy, Clone)]
enum DataSource {
    /// No data handled by the opcode
    None,
    /// The data value has been read by the CPU and is stored in extra bytes
    Immediate,
    /// Data stays on the pins
    Pins,
    /// The value is in a 8 bit register
    Register8(cpclib_asm::preamble::Register8),
    /// The value is in a 16 bit register
    Register16(cpclib_asm::preamble::Register16),
    /// The value is in a 16 bits indexed regiser
    IndexRegister16(cpclib_asm::preamble::IndexRegister16),
    /// The value is in a 8 bits indexed regiser
    IndexRegister8(cpclib_asm::preamble::IndexRegister8),
    /// The value msut be read from memory and tḧe address is in the register
    MemoryRegister16(cpclib_asm::preamble::Register16),
}

type DataDestination = DataSource;

impl From<&DataAccess> for DataSource {
    fn from(d: &DataAccess) -> DataSource {
        match d {
            DataAccess::IndexRegister16(reg) => DataSource::IndexRegister16(*reg),
            DataAccess::IndexRegister8(reg) => DataSource::IndexRegister8(*reg),
            DataAccess::Register16(reg) => DataSource::Register16(*reg),
            DataAccess::Register8(reg) => DataSource::Register8(*reg),
            DataAccess::Expression(_) => DataSource::Immediate, // to be read
            _ => panic!("{:?} unimplemented", d),
        }
    }
}

/// Simulate the Cycles of the CPU. No need to simulate the pipeline stuff however
#[derive(Debug, PartialEq)]
pub(crate) enum MCycle {
    /// M1 (State, byte number, opcode prefix)
    /// - T1: request the reading of a byte
    /// - T2: read the byte
    /// - T3&T4: memory refresh / decode the opcode
    FetchOpcode(TState, u8, OpcodePrefix),
    /// Read state (state, byte index to read) opciode information is stored by CPU
    Read(TState, u8),
    Write(TState, u8),
    Input(TState, u8),
    Output(TState, u8),
}

impl Default for MCycle {
    fn default() -> Self {
        MCycle::FetchOpcode(TState::default(), 0, OpcodePrefix::None)
    }
}

impl MCycle {
    /// Test if we are handling a new opcode in the next cycle
    pub fn is_new_opcode(&self) -> bool {
        *self == MCycle::default()
    }

    fn inc_state(&mut self) {
        *self = match self {
            MCycle::FetchOpcode(TState::T1, nb, prefix) => {
                MCycle::FetchOpcode(TState::T2, *nb, *prefix)
            }
            MCycle::FetchOpcode(TState::T2, nb, prefix) => {
                MCycle::FetchOpcode(TState::T3, *nb, *prefix)
            }
            MCycle::FetchOpcode(TState::T3, nb, prefix) => {
                MCycle::FetchOpcode(TState::T4, *nb, *prefix)
            }
            MCycle::FetchOpcode(TState::T4, nb, prefix) => panic!("Ambigous"),

            MCycle::Read(TState::T1, nb) => MCycle::Read(TState::T2, *nb),
            MCycle::Read(TState::T2, nb) => MCycle::Read(TState::T3, *nb),
            MCycle::Read(TState::T3, nb) => panic!("Ambigous"),

            MCycle::Write(TState::T1, nb) => MCycle::Write(TState::T2, *nb),
            MCycle::Write(TState::T2, nb) => MCycle::Write(TState::T3, *nb),
            MCycle::Write(TState::T3, nb) => panic!("Ambigous"),

            MCycle::Output(TState::T1, nb) => MCycle::Output(TState::T2, *nb),
            MCycle::Output(TState::T2, nb) => MCycle::Output(TState::T3, *nb),
            MCycle::Output(TState::T3, nb) => panic!("Ambigous"),

            _ => unimplemented!(),
        }
    }

    fn prefix(&self) -> OpcodePrefix {
        match self {
            MCycle::FetchOpcode(_, _, prefix) => *prefix,
            _ => unreachable!(),
        }
    }
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

/// Common trait for handling Register 8 and 16 bits
/// TODO provide Carry related information
pub trait HasValue {
    /// The type that encodes the value of interest
    type ValueType: Integer + One + OverflowingAdd + OverflowingSub;

    /// Retreive the stored value
    fn value(&self) -> Self::ValueType;

    #[inline]
    fn get(&self) -> Self::ValueType {
        self.value()
    }
    /// Change the stored value
    fn set(&mut self, value: Self::ValueType);

    /// Make a wrapping addition. Return true in case of overflow
    fn add(&mut self, value: Self::ValueType) -> bool {
        let (res, ovf) = self.value().overflowing_add(&value);
        self.set(res);
        ovf
    }

    /// Make a wrapping substraction
    fn sub(&mut self, value: Self::ValueType) -> bool {
        let (res, ovf) = self.value().overflowing_sub(&value);
        self.set(res);
        ovf
    }

    /// Make a wrapping increment
    fn inc(&mut self) -> bool {
        self.add(Self::ValueType::one())
    }

    /// Make a wrapping decrement
    fn dec(&mut self) -> bool {
        self.sub(Self::ValueType::one())
    }
}

/// Represents an 8 bit register
#[derive(Copy, Clone, PartialEq)]
pub struct Register8 {
    /// The 8 bit value of the register
    val: u8,
}

impl fmt::Debug for Register8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Register8")
         .field(&format!("0x{:X}", self.value()))
         .finish()
    }
}

/// By default a Register8 is set to 0
/// TODO use an Unknown value ?
impl Default for Register8 {
    fn default() -> Self {
        Self { val: 0 }
    }
}

// TODO use macro for that
impl HasValue for Register8 {
    type ValueType = u8;

    fn value(&self) -> Self::ValueType {
        self.val
    }

    fn set(&mut self, value: Self::ValueType) {
        self.val = value;
    }
}

impl Register8 {
    pub fn res_bit(&mut self, bit: u8) {
        let new = self.value() & (!(1 << bit));
        self.set(new);
    }

    pub fn set_bit(&mut self, bit: u8) {
        let new = self.value() | (1 << bit);
        self.set(new);
    }

    pub fn and(&mut self, value: u8) {
        let new = self.value() & value;
        self.set(new);
    }

    pub fn or(&mut self, value: u8) {
        let new = self.value() | value;
        self.set(new);
    }

    pub fn xor(&mut self, value: u8) {
        let new = self.value() ^ value;
        self.set(new);
    }
}

/// Represents a 16 bits register by decomposing it in 2 8 bits registers
#[derive(Copy, Clone, Default, PartialEq)]
pub struct Register16 {
    low: Register8,
    high: Register8,
}

impl fmt::Debug for Register16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Register16")
         .field(&format!("0x{:X}", self.value()))
         .finish()
    }
}

impl HasValue for Register16 {
    type ValueType = u16;

    fn value(&self) -> Self::ValueType {
        256 * u16::from(self.high().value()) + u16::from(self.low().value())
    }

    fn set(&mut self, value: Self::ValueType) {
        self.low_mut().set((value % 256) as _);
        self.high_mut().set((value / 256) as _);
    }
}

impl Register16 {
    pub fn new(val: u16) -> Self {
        let mut r = Register16::default();
        r.set(val);
        r
    }

    pub fn low(&self) -> &Register8 {
        &self.low
    }

    pub fn high(&self) -> &Register8 {
        &self.high
    }

    pub fn low_mut(&mut self) -> &mut Register8 {
        &mut self.low
    }

    pub fn high_mut(&mut self) -> &mut Register8 {
        &mut self.high
    }
}

/// Identifiers for any 16 bits registers
#[derive(Debug, Copy, Clone)]
pub enum Reg16 {
    Af,
    Af2,
    Hl,
    Hl2,
    De,
    De2,
    Bc,
    Bc2,
    Wz,
    Pc,
    Sp,
    Ix,
    Iy,
}

/// Identifiers for any 8 bits registers
#[derive(Debug, Copy, Clone)]
pub enum Reg8 {
    A,
    A2,
    H,
    H2,
    D,
    D2,
    B,
    B2,
    W,
    Ixh,
    Iyh,

    F,
    F2,
    L,
    L2,
    E,
    E2,
    C,
    C2,
    Z,

    Ixl,
    Iyl,
}

impl Into<Reg8> for &cpclib_asm::preamble::Register8 {
    fn into(self) -> Reg8 {
        match self {
            cpclib_asm::preamble::Register8::A => Reg8::A,
            cpclib_asm::preamble::Register8::B => Reg8::B,
            cpclib_asm::preamble::Register8::C => Reg8::C,
            cpclib_asm::preamble::Register8::D => Reg8::D,
            cpclib_asm::preamble::Register8::E => Reg8::E,
            cpclib_asm::preamble::Register8::H => Reg8::H,
            cpclib_asm::preamble::Register8::L => Reg8::L,
        }
    }
}

impl Into<Reg8> for &cpclib_asm::preamble::IndexRegister8 {
    fn into(self) -> Reg8 {
        match self {
            cpclib_asm::preamble::IndexRegister8::Ixh => Reg8::Ixh,
            cpclib_asm::preamble::IndexRegister8::Ixl => Reg8::Ixl,
            cpclib_asm::preamble::IndexRegister8::Iyh => Reg8::Iyh,
            cpclib_asm::preamble::IndexRegister8::Iyl => Reg8::Iyl,
        }
    }
}

impl Reg8 {
    fn is_low(&self) -> bool {
        !self.is_high()
    }

    fn is_high(&self) -> bool {
        use Reg8::*;
        match self {
            A => true,
            A2 => true,
            H => true,
            H2 => true,
            D => true,
            D2 => true,
            B => true,
            B2 => true,
            W => true,
            Ixh => true,
            Iyh => true,

            _ => false,
        }
    }

    fn full(&self) -> Reg16 {
        use Reg8::*;
        match self {
            A => Reg16::Af,
            A2 => Reg16::Af2,
            H => Reg16::Hl,
            H2 => Reg16::Hl2,
            D => Reg16::De,
            D2 => Reg16::De2,
            B => Reg16::Bc,
            B2 => Reg16::Bc2,
            W => Reg16::Wz,
            Ixh => Reg16::Ix,
            Iyh => Reg16::Iy,

            F => Reg16::Af,
            F2 => Reg16::Af2,
            L => Reg16::Hl,
            L2 => Reg16::Hl2,
            E => Reg16::De,
            E2 => Reg16::De2,
            C => Reg16::Bc,
            C2 => Reg16::Bc2,
            Z => Reg16::Wz,

            Ixl => Reg16::Ix,
            Iyl => Reg16::Iy,
        }
    }
}

impl Reg16 {
    fn high(&self) -> Reg8 {
        use Reg16::*;
        match self {
            Af => Reg8::A,
            Af2 => Reg8::A2,
            Hl => Reg8::H,
            Hl2 => Reg8::H2,
            De => Reg8::D,
            De2 => Reg8::D2,
            Bc => Reg8::B,
            Bc2 => Reg8::B2,
            Wz => Reg8::W,
            Ix => Reg8::Ixh,
            Iy => Reg8::Iyh,
            _ => unreachable!(),
        }
    }

    fn low(&self) -> Reg8 {
        use Reg16::*;
        match self {
            Af => Reg8::F,
            Af2 => Reg8::F2,
            Hl => Reg8::L,
            Hl2 => Reg8::L2,
            De => Reg8::E,
            De2 => Reg8::E2,
            Bc => Reg8::C,
            Bc2 => Reg8::C2,
            Wz => Reg8::Z,
            Ix => Reg8::Ixl,
            Iy => Reg8::Iyl,
            _ => unreachable!(),
        }
    }
}

impl Into<Reg16> for &cpclib_asm::preamble::Register16 {
    fn into(self) -> Reg16 {
        match self {
            cpclib_asm::preamble::Register16::Af => Reg16::Af,
            cpclib_asm::preamble::Register16::Hl => Reg16::Hl,
            cpclib_asm::preamble::Register16::Bc => Reg16::Bc,
            cpclib_asm::preamble::Register16::De => Reg16::De,
            cpclib_asm::preamble::Register16::Sp => Reg16::Sp,
        }
    }
}

impl Into<Reg16> for &cpclib_asm::preamble::IndexRegister16 {
    fn into(self) -> Reg16 {
        match self {
            cpclib_asm::preamble::IndexRegister16::Ix => Reg16::Ix,
            cpclib_asm::preamble::IndexRegister16::Iy => Reg16::Iy,
        }
    }
}

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

    pub(crate) iff1: Register16,
    pub(crate) iff2: Register16,
    pub(crate) im: u8,

    /// Refresh adr
    /// XXX Not handled at all by the emulator
    pub(crate) refresh_addr: u16,

    /// Information for the current opcode - Setup after fetching the very first byte
    pub(crate) opcode_information: Option<OpcodeInformation>,

    /// Flag to know if the cpu is accepting interruptions
    pub(crate) interruption_disabled: bool,
}

// Flags related code
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
    fn test_flag(&self, flag_test: FlagTest) -> bool {
        let flag = self.reg8(Reg8::F).value();
        match flag_test {
            FlagTest::NZ => Z80Flags::Z.is_off(flag),
            FlagTest::Z  => Z80Flags::Z.is_on(flag),

            FlagTest::NC => Z80Flags::C.is_off(flag),
            FlagTest::C  => Z80Flags::C.is_on(flag),

            FlagTest::PO => Z80Flags::P.is_off(flag),
            FlagTest::PE => Z80Flags::P.is_on(flag),

            FlagTest::P  => Z80Flags::S.is_off(flag),
            FlagTest::M  => Z80Flags::S.is_on(flag),
        }
    }
}

impl Z80 {

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

    fn get_address_from_address_source(&self, src: &AddressSource) -> u16 {
        match src {
            /// Impossible case
            AddressSource::None => unreachable!(),
            /// As already been loaded in Wz
            AddressSource::Immediate => self.reg16(Reg16::Wz).get(),
            /// Is stored withing a 16 bits register
            AddressSource::Register16(ref reg) => self.reg16(reg.into()).get(),
            AddressSource::IndexRegister16(ref reg) => self.reg16(reg.into()).get(),
            /// Corresponds to an indexed address
            AddressSource::Indexed(ref reg) => {
                // by definition, delta is within the first byte
                panic!("BUG for negative delta value or to be checked");
                self.reg16(reg.into()).get() + (((self.reg8(Reg8::Z).get() as i8) as i32) as u16)
                //
            }
        }
    }

    /// Returns the byte that needs to be written
    fn get_byte_to_write_from_opcode_information(&self) -> u8 {
        self.get_byte_from_data_source(&self.opcode_information.as_ref().unwrap().write_data)
    }

    /// Returns the appropriate byte
    fn get_byte_from_data_source(&self, src: &DataSource) -> u8 {
        match src {
            DataSource::Register8(reg) => self.reg8(reg.into()).get(),
            DataSource::Pins => self.get_data_from_pins(),
            _ => panic!(
                "{:?} contains a 16 bits value. Find another way to handle that",
                src
            ),
        }
    }

    /// Execute the instruction ; all necessary stuffs has already been collected
    fn execute_step(&mut self) {
        let prefix = &self.opcode_information.as_ref().unwrap().prefix;

        let memory = [self.reg8(Reg8::Z).get(), self.reg8(Reg8::W).get()];

        // By construction, it cannot fail
        let (opcode, rest) = dbg!(cpclib_asm::disass::disassemble_with_potential_argument(
            self.ir.get(),
            &prefix.disassembling_table(),
            &memory
        )
        .unwrap());
        assert!(opcode.is_opcode());
        self.execute_opcode(
            opcode.mnemonic().unwrap(),
            opcode.mnemonic_arg1(),
            opcode.mnemonic_arg2(),
        );

        // reset the various buffers
        self.opcode_information = None;
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
                assert!(current_prefix == OpcodePrefix::None, "Not implemented yet");
                return false;
            }
            0xfd => {
                self.next_state = MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::Fd);
                assert!(current_prefix == OpcodePrefix::None, "Not implemented yet");
                return false;
            }
            0xcb => {
                self.next_state = MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::Cb);
                assert!(current_prefix == OpcodePrefix::None, "Not implemented yet");
                return false;
            }
            0xed => {
                self.next_state = MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::Ed);
                assert!(current_prefix == OpcodePrefix::None, "Not implemented yet");
                return false;
            }

            _ => {
                // Get the number of additional bytes to read as well as the information for the next cycles
                self.opcode_information = Some(current_prefix.opcode_information(self.ir.get()));
                let opcode_information = self.opcode_information.as_ref().unwrap();

                if opcode_information.fetch_finished() {
                    if opcode_information.expect_read() {
                        self.next_state = MCycle::Read(TState::T1, 0);
                        return false;
                    } else if opcode_information.expect_write() {
                        if opcode_information.addresses_memory {
                            self.next_state = MCycle::Write(TState::T1, 0);
                        } else {
                            self.next_state = MCycle::Output(TState::T1, 0);
                        }
                        return false;
                    } else {
                        // Start a new opcode
                        self.next_state = MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None);
                        return true;
                    }
                } else if opcode_information.nb_additional_opcode_bytes != 0 {
                    // continue to read the bytes for the current opcode
                    self.next_state = MCycle::FetchOpcode(TState::T1, 1, current_prefix);
                    return false;
                } else {
                    unimplemented!("{:?}", opcode_information)
                }
            }
        }
    }

    /// Execute the end of the instration (everything related to memory or ports has already been handled).
    /// Here we take care of registers modifications and flags configuration
    fn execute_opcode(
        &mut self,
        opcode: &Mnemonic,
        arg1: Option<&DataAccess>,
        arg2: Option<&DataAccess>,
    ) {
        match opcode {
            Mnemonic::Nop | Mnemonic::Out => {
                // nothing to do because everything has been handled by cpu
            }

            Mnemonic::Add => {
                match arg1.unwrap() {
                    DataAccess::Register8(_) => {
                        // this MUST be A register
                        // 8bits version
                        let right = self.get_available_8bits_value(arg2.unwrap());
                        let ovf = self.reg8_mut(Reg8::A).add(right); // TODO manage the flags
                    }
                    _ => unimplemented!(),
                }
            }

            Mnemonic::Dec => {
                let mut update_flags = true; // disable only for 16 bits
                let mut val = 0;
                match arg1.unwrap() {
                    DataAccess::Register8(reg) => {
                        let left: Reg8 = reg.into();
                        self.reg8_mut(left).dec();
                        val = self.reg8(left).value();
                    }
                    DataAccess::IndexRegister8(reg) => {
                        let left: Reg8 = reg.into();
                        self.reg8_mut(left).dec();
                        val = self.reg8(left).value();
                    }
                    _ => unimplemented!(),
                }

                if update_flags {
                    let f = &mut self.reg8_mut(Reg8::F).val;
                    Z80Flags::S.set(val == 0, f);
                    Z80Flags::Z.set(val & 1 << 7 != 0, f);
                    Z80Flags::Y.set(val & 1 << 5 != 0, f);
                    // TODO handle H flag
                    Z80Flags::X.set(val & 1 << 3 != 0, f);
                    // TODO handle P flag
                    Z80Flags::N.set_on(f);
                }
            }

            Mnemonic::Di => {
                self.interruption_disabled = true;
            }

            Mnemonic::Inc => {
                let mut update_flags = true; // disable only for 16 bits
                let mut val = 0;
                match arg1.unwrap() {
                    DataAccess::Register8(reg) => {
                        let left: Reg8 = reg.into();
                        self.reg8_mut(left).inc();
                        val = self.reg8(left).value();
                    }
                    DataAccess::IndexRegister8(reg) => {
                        let left: Reg8 = reg.into();
                        self.reg8_mut(left).inc();
                        val = self.reg8(left).value();
                    }
                    _ => unimplemented!(),
                }

                if update_flags {
                    let f = &mut self.reg8_mut(Reg8::F).val;
                    Z80Flags::S.set(val == 0, f);
                    Z80Flags::Z.set(val & 1 << 7 != 0, f);
                    Z80Flags::Y.set(val & 1 << 5 != 0, f);
                    // TODO handle H flag
                    Z80Flags::X.set(val & 1 << 3 != 0, f);
                    // TODO handle P flag
                    Z80Flags::N.set_off(f);
                }
            }

            Mnemonic::Jp => {
                let do_jump = match arg1 {
                    None => true,
                    Some(DataAccess::FlagTest(flag_test)) => self.test_flag(*flag_test),
                    _ => unreachable!()
                };

                if do_jump {
                   
                  let adr =  match arg2.unwrap() {
                        DataAccess::Expression(expr) => {
                            expr.eval().unwrap() as u16
                        }

                        DataAccess::MemoryRegister16(reg) => {
                            self.reg16(reg.into()).value()
                        }

                        DataAccess::MemoryIndexRegister16(reg) => {
                            self.reg16(reg.into()).value()
                        },

                        _ => unreachable!()
                    };

                    self.pc.set(adr);

                }

            }

            Mnemonic::Jr => {
                let do_jump = match arg1 {
                    None => true,
                    Some(DataAccess::FlagTest(flag_test)) => self.test_flag(*flag_test),
                    _ => unreachable!()
                };

                if do_jump {
                    dbg!("The implementation is incorrect. An additional mcycle must handle the operation");
                    
                    let mut sym = SymbolsTableCaseDependent::default();
                    sym.set_symbol_to_value("$", -2);

                    let expr = arg2.unwrap().expr().unwrap();
                    let displacement = expr.relative_delta().unwrap();

                    dbg!(displacement);
                    
                    // the very same value is also here
                //   let displacement = self.wz.low().value() as i8 as i16 as u16;


                    if displacement > 0 {
                        self.pc.add(displacement as u16);
                    }
                    else {
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

            Mnemonic::Ld => {
                match arg1.unwrap() {
                    DataAccess::Register8(reg) => {
                        // 8bits version
                        let left: Reg8 = reg.into();
                        let right = self.get_available_8bits_value(arg2.unwrap());
                        self.reg8_mut(left).set(right);
                    }

                    DataAccess::Register16(reg) => {
                        let left: Reg16 = reg.into();
                        let right = self.get_available_16bits_value(arg2.unwrap());
                        self.reg16_mut(left).set(right);
                    }

                    DataAccess::Memory(_) => {
                        // we have nothing to do because CPU has already executed everything
                    }

                    _ => unimplemented!(),
                }
            }

            Mnemonic::Outd => {
                let b_is_zero = self.reg8(Reg8::B).value() == 0;
                let f = &mut self.reg8_mut(Reg8::F).val;
                Z80Flags::Z.set(b_is_zero, f);
                Z80Flags::N.set_on(f);
            }

            Mnemonic::Xor => {
                let right = dbg!(self.get_available_8bits_value(arg1.unwrap()));
                self.reg8_mut(Reg8::A).xor(right);
            }
            _ => unimplemented!("{:?}, {:?}, {:?}", opcode, arg1, arg2),
        }
    }

    /// Return the value for this data access
    /// - Inside a register / straightworard to obtain
    /// - In memory / the cpu has read it and stored it in XXX
    /// - In chip / the cpu has read it and stored it in XXX
    pub fn get_available_8bits_value(&self, data: &DataAccess) -> u8 {
        match data {
            DataAccess::Register8(reg) => {
                let reg: Reg8 = dbg!(reg.into());
                return self.reg8(reg).get();
            }
            DataAccess::IndexRegister8(reg) => {
                let reg: Reg8 = reg.into();
                return self.reg8(reg).get();
            }
            DataAccess::MemoryRegister16(_reg) => {
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

    /// Retreive the 16 bits that corresponds to the data choice
    pub fn get_available_16bits_value(&self, data: &DataAccess) -> u16 {
        match data {
            DataAccess::Register16(reg) => {
                let reg: Reg16 = dbg!(reg.into());
                return self.reg16(reg).get();
            }
            DataAccess::IndexRegister16(reg) => {
                let reg: Reg16 = reg.into();
                return self.reg16(reg).get();
            }
            DataAccess::MemoryRegister16(_reg) => {
                // Here we do not care at all of which register is used for memory reading
                // Because the CPU has already read the information
                return self.reg16(Reg16::Wz).get();
            }
            DataAccess::Expression(expr) => {
                return expr.eval().unwrap() as u16; // cannot fail as the value has been disassembled
            }
            DataAccess::Memory(_) => return self.reg16(Reg16::Wz).get(),
            _ => {
                panic!("{:?} unhandled", data);
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
        let r16 = self.reg16_mut(reg.full());
        if reg.is_high() {
            r16.high_mut()
        } else {
            r16.low_mut()
        }
    }

    pub fn reg8(&self, reg: Reg8) -> &Register8 {
        let r16 = self.reg16(reg.full());
        if reg.is_high() {
            r16.high()
        } else {
            r16.low()
        }
    }
}

// Opcode specific code
impl Z80 {
    /// Manage the specific treatment after the read_instruction
    pub fn post_read_action(&mut self) {
        let opcode_info = self.opcode_information.as_ref().unwrap();

        match opcode_info.token.mnemonic() {
            Some(Mnemonic::Outd) => {
                self.reg16_mut(Reg16::Hl).dec();
                self.reg8_mut(Reg8::B).dec();
            }

            _ => {
                // by default, there is nothing to
            }
        }
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
        match self.next_state {
            // First state of the fetch process
            MCycle::FetchOpcode(TState::T1, _, _) => {
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

            MCycle::FetchOpcode(TState::T3, _, _) => {
                // we do nothing here
                // but, IRL, the decoding appears there
                self.next_state.inc_state();
                self.set_pin(Z80Pin::Neg_MREQ, false);
            }

            MCycle::FetchOpcode(TState::T4, nb, prefix) => {
                self.set_pin(Z80Pin::Neg_MREQ, false);
                self.set_pin(Z80Pin::Neg_M1, false);
                self.set_pin(Z80Pin::Neg_RFSH, true);

                if nb == 0 {
                    let execute = dbg!(self.decode_step()); // setup next state
                    if execute {
                        // nothing is required for the execution
                        // TODO check if input or memory cycle is required
                        self.execute_step();
                    } else {
                        // nothing to do ; the cpu is already in the right state
                    }
                } else {
                    let opcode_information = self.opcode_information.as_ref().unwrap();
                    if nb == opcode_information.nb_additional_opcode_bytes {
                        if 0 != opcode_information.nb_additional_bytes_to_read {
                            self.next_state = MCycle::Read(TState::T1, 0);
                        } else if 0 != opcode_information.nb_bytes_to_write {
                            if opcode_information.addresses_memory {
                                self.next_state = MCycle::Write(TState::T1, 0);
                            } else {
                                self.next_state = MCycle::Output(TState::T1, 0);
                            }
                        } else {
                            self.next_state =
                                MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None); // go back for a new instruction
                            self.execute_step();
                        }
                    } else {
                        // we still need to read something
                        self.next_state = MCycle::FetchOpcode(TState::T1, nb + 1, prefix);
                    }
                }
            }

            MCycle::Read(TState::T1, nb) => {
                let address = self.get_read_address_from_opcode_information();
                self.set_address_on_pins(address);
                self.set_pin(Z80Pin::Neg_MREQ, false);
                self.set_pin(Z80Pin::Neg_RD, false);
                self.next_state.inc_state();
            }
            MCycle::Read(TState::T2, nb) => {
                self.set_pin(Z80Pin::Neg_RD, true);
                self.set_pin(Z80Pin::Neg_MREQ, true);
                let byte = self.get_data_from_pins();
                if nb == 0 {
                    self.reg8_mut(Reg8::Z).set(byte);
                } else
                /*if nb == 1*/
                {
                    self.reg8_mut(Reg8::W).set(byte);
                }
                self.next_state.inc_state();
            }
            MCycle::Read(TState::T3, nb) => {
                self.post_read_action();

                let opcode_info = self.opcode_information.as_ref().unwrap();
                if nb + 1 == opcode_info.nb_additional_bytes_to_read {
                    if 0 != opcode_info.nb_bytes_to_write {
                        if opcode_info.addresses_memory {
                            self.next_state = MCycle::Write(TState::T1, 0);
                        } else {
                            self.next_state = MCycle::Output(TState::T1, 0);
                        }
                    } else {
                        // TODO take into account those that do a write after the read (if that exists)
                        self.execute_step();
                        self.next_state = MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None);
                        // go back for a new instruction
                    }
                } else {
                    // go to read the next byte
                    self.next_state = MCycle::Read(TState::T1, nb + 1);
                }
            }

            MCycle::Write(TState::T1, nb) => {
                let address = self.get_write_address_from_opcode_information();
                self.set_address_on_pins(address);

                self.next_state.inc_state();
            }
            MCycle::Write(TState::T2, nb) => {
                self.set_pin(Z80Pin::Neg_MREQ, false);
                self.set_pin(Z80Pin::Neg_WR, false);

                let byte = match nb {
                    0 => self.get_byte_to_write_from_opcode_information(),
                    _ => unimplemented!("{} {:?}", nb, self.opcode_information),
                };

                self.set_data_on_pins(byte);
                self.next_state.inc_state();
            }
            MCycle::Write(TState::T3, nb) => {
                self.set_pin(Z80Pin::Neg_WR, true);
                self.set_pin(Z80Pin::Neg_MREQ, true);

                if nb == self.opcode_information.as_ref().unwrap().nb_bytes_to_write + 2 - 1 {
                    // TODO take into account those that do a write after the read (if that exists)
                    self.execute_step();
                    self.next_state = MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None);
                // go back for a new instruction
                } else {
                    // go to write the next byte
                    self.next_state = MCycle::Write(TState::T1, nb + 1);
                }
            }

            MCycle::Output(TState::T1, nb) => {
                let address = self.get_write_address_from_opcode_information();
                self.set_address_on_pins(address);
                assert!(self.get_pin(Z80Pin::Neg_IORQ));
                assert!(self.get_pin(Z80Pin::Neg_WR));

                self.next_state.inc_state();
            }
            MCycle::Output(TState::T2, nb) => {
                self.set_pin(Z80Pin::Neg_IORQ, false);
                self.set_pin(Z80Pin::Neg_WR, false);

                let byte = self.get_byte_to_write_from_opcode_information();
                assert_eq!(
                    self.opcode_information.as_ref().unwrap().nb_bytes_to_write,
                    1
                );

                self.set_data_on_pins(byte);
                self.next_state.inc_state();
            }
            MCycle::Output(TState::T3, nb) => {
                self.set_pin(Z80Pin::Neg_IORQ, true);
                self.set_pin(Z80Pin::Neg_WR, true);

                if nb == self.opcode_information.as_ref().unwrap().nb_bytes_to_write - 1 {
                    // TODO take into account those that do a write after the read (if that exists)
                    self.execute_step();
                    self.next_state = MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None);
                // go back for a new instruction
                } else {
                    // go to write the next byte
                    self.next_state = MCycle::Output(TState::T1, nb + 1);
                }
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
        self.iff1.set(0);
        self.iff2.set(0);
        self.im = 0;
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
        assert_eq!(desc.nb_additional_bytes_to_read, 1);

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

    #[test]
    fn lookup_checks_hl() {
        // ld hl, xxxx
        assert_eq!(dbg!(&OPCODE_PREFIX[0x21]).nb_additional_opcode_bytes, 2);
    }

    #[test]
    fn lookup_checks_out() {
        // out(c), a
        assert_eq!(
            dbg!(&OPCODE_PREFIX_ED[0x79]).nb_additional_opcode_bytes,
            0 // ed is the already additional
        );
        assert_eq!(dbg!(&OPCODE_PREFIX_ED[0x79]).nb_bytes_to_write, 1);
        assert_eq!(dbg!(&OPCODE_PREFIX_ED[0x79]).addresses_memory, false);
        assert_eq!(
            dbg!(&OPCODE_PREFIX_ED[0x79]).write_address,
            AddressSource::Register16(cpclib_asm::preamble::Register16::Bc)
        );
        assert_eq!(
            dbg!(&OPCODE_PREFIX_ED[0x79]).write_data,
            DataSource::Register8(cpclib_asm::preamble::Register8::A)
        );

        // out (n), a
        assert_eq!(
            dbg!(&OPCODE_PREFIX[0xd3]).nb_additional_opcode_bytes,
            1 //n
        );

        // outd
        let desc = dbg!(&OPCODE_PREFIX_ED[0xab]);
        assert_eq!(desc.nb_bytes_to_write, 1);
        assert_eq!(desc.nb_additional_opcode_bytes, 0);
        assert_eq!(
            desc.read_address,
            AddressSource::Register16(cpclib_asm::preamble::Register16::Hl)
        );
        assert_eq!(
            desc.write_address,
            AddressSource::Register16(cpclib_asm::preamble::Register16::Bc)
        );
        assert_eq!(
            desc.write_data,
            DataSource::Pins // we assume data are already on the pins because they have been read one cycle before
        );
    }
}
