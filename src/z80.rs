use crate::chip::*;

use num::integer::Integer;
use num::One;
use num::traits::ops::overflowing::OverflowingAdd;
use num::traits::ops::overflowing::OverflowingSub;

use std::fmt;
use std::fmt::Debug;
use std::mem::swap;

use regex::Regex;
use lazy_static::lazy_static;
use cpclib_asm::preamble::*;


/// Possible states for a cycle
/// Warning: Wait state does not count
#[derive(Debug, PartialEq)]
enum TState {
    T1,
    T2,
    T3,
    T4
}

impl Default for TState {
    fn default() -> Self {
        TState::T1
    }
}

/// This enumeration aims at helping to treat the 
/// prefixed operations
#[derive(Debug, PartialEq, Copy, Clone)]
enum OpcodePrefix {
    None,
    Dd,
    Fd,
    Cb,
    Ed
}

impl Default for OpcodePrefix {
    fn default() -> Self {
        OpcodePrefix::None
    }
}



// Generate the various lookup tables needed to control the opcodes execution
// rust is currently unable to do that in a static way
lazy_static! {
    // Get the number of bytes to read in addition to the opcode
    static ref TABINSTR_BYTES_TO_READ: [u8; 256] = array_init::array_init(OpcodePrefix::None.bytes_to_read_build_function());
    static ref TABINSTRCB_BYTES_TO_READ: [u8; 256] = array_init::array_init(OpcodePrefix::Cb.bytes_to_read_build_function());
    static ref TABINSTRDD_BYTES_TO_READ: [u8; 256] = array_init::array_init(OpcodePrefix::Dd.bytes_to_read_build_function());
    static ref TABINSTRED_BYTES_TO_READ: [u8; 256] = array_init::array_init(OpcodePrefix::Ed.bytes_to_read_build_function());
    static ref TABINSTRFD_BYTES_TO_READ: [u8; 256] = array_init::array_init(OpcodePrefix::Fd.bytes_to_read_build_function());


    // Get the location of the address value (if any) to read something
    static ref TABINSTR_INPUT_ADDRESS: [AddressSource; 256]  = array_init::array_init(OpcodePrefix::None.read_address_source_table_function());
    static ref TABINSTRCB_INPUT_ADDRESS: [AddressSource; 256]  = array_init::array_init(OpcodePrefix::Cb.read_address_source_table_function());
    static ref TABINSTRDD_INPUT_ADDRESS: [AddressSource; 256]  = array_init::array_init(OpcodePrefix::Dd.read_address_source_table_function());
    static ref TABINSTRED_INPUT_ADDRESS: [AddressSource; 256]  = array_init::array_init(OpcodePrefix::Ed.read_address_source_table_function());
    static ref TABINSTRFD_INPUT_ADDRESS: [AddressSource; 256]  = array_init::array_init(OpcodePrefix::Fd.read_address_source_table_function());

}

/// Contains all the informations needed to drive the next
/// cycles of the instruction
#[derive(Copy, Clone, Debug)]
struct OpcodeInformation {
    /// Prefix for the current opcode
    prefix: OpcodePrefix,
    /// Number of bytes of the opcode that need to be fetched
    nb_opcode_bytes: u8,
    /// Location of the reading address
    read_address: AddressSource,
    /// Number of bytes to read from the address
    nb_bytes_to_read: u8,
}

impl OpcodeInformation {
    /// Fetch is finished for opcode of 1 byte with no memory/port access
    fn fetch_finished(&self) -> bool {
        self.nb_opcode_bytes == 0 && self.read_address == AddressSource::None
    }

    /// Check if CPU as to read from memory
    fn expect_read(&self) -> bool {
        AddressSource::None != self.read_address
    }
}


impl OpcodePrefix {
    /// Retreive all the information needed to data collection for the current opcode
    fn opcode_information(&self, opcode: u8) -> OpcodeInformation {
        let opcode = opcode as usize;

        let read_address = self.read_address_source_table()[opcode];
        let nb_bytes_to_read = match read_address {
            AddressSource::None => 0,
            _ => 1 // TODO really set the right value
        };
        OpcodeInformation {
            prefix: self.clone(),
            nb_opcode_bytes: self.bytes_to_read_table()[opcode],
            read_address ,
            nb_bytes_to_read, 
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

    /// Returns the table that indicates where the CPU must retreive the information to put on the address bus in the Read cycles
    fn read_address_source_table(&self) -> &'static [AddressSource; 256] {
        match self {
            OpcodePrefix::None => &TABINSTR_INPUT_ADDRESS,
            OpcodePrefix::Cb => &TABINSTRCB_INPUT_ADDRESS,
            OpcodePrefix::Dd => &TABINSTRDD_INPUT_ADDRESS,
            OpcodePrefix::Ed => &TABINSTRED_INPUT_ADDRESS,
            OpcodePrefix::Fd => &TABINSTRFD_INPUT_ADDRESS,
        }     
    }


    /// Returns the table that provide the number of bytes to read to complete the opcode
    fn bytes_to_read_table(&self) -> &'static [u8; 256]{
        match self {
            OpcodePrefix::None => &TABINSTR_BYTES_TO_READ,
            OpcodePrefix::Cb => &TABINSTRCB_BYTES_TO_READ,
            OpcodePrefix::Dd => &TABINSTRDD_BYTES_TO_READ,
            OpcodePrefix::Ed => &TABINSTRED_BYTES_TO_READ,
            OpcodePrefix::Fd => &TABINSTRFD_BYTES_TO_READ,
        }
    }


    /// Serve to generate lookup tables at start
    fn bytes_to_read_build_function(self) -> Box<Fn(usize) -> u8> {
        lazy_static! {
            static ref  height_bits: Regex = Regex::new(r"[^n]?nn[^n]?").unwrap();
            static ref sixteen_bits: Regex = Regex::new(r"[^n]?nnnn[^n]?").unwrap();
        }

        Box::new(move |i: usize|{
            let repr = self.disassembling_table()[i];

            let nb_height_bits_args = height_bits.captures_iter(repr).count();
            let nb_sixteen_bits_args = sixteen_bits.captures_iter(repr).count();

            (nb_height_bits_args + 2*nb_sixteen_bits_args) as u8
        })
    }

    fn read_address_source_table_function(self) -> Box<Fn(usize) -> AddressSource> {
          lazy_static! {
            static ref  indexed_iy: Regex = Regex::new(r"\(IY\+nn\)$").unwrap();
            static ref  indexed_ix: Regex = Regex::new(r"\(IX\+nn\)$").unwrap();
            static ref  in_hl: Regex = Regex::new(r"\(HL\)$").unwrap();
            static ref  in_bc: Regex = Regex::new(r"\(BC\)$").unwrap();
            static ref  in_de: Regex = Regex::new(r"\(DE\)$").unwrap();
            static ref  in_af: Regex = Regex::new(r"\(AF\)$").unwrap();
            static ref  in_ix: Regex = Regex::new(r"\(IX\)$").unwrap();
            static ref  in_iy: Regex = Regex::new(r"\(IY\)$").unwrap();
            static ref  immediate: Regex = Regex::new(r"\(nnnn\)$").unwrap();
          }

          Box::new(move |i: usize|{
            let repr = self.disassembling_table()[i];

            if indexed_iy.is_match(repr) {
                AddressSource::Indexed(cpclib_asm::preamble::IndexRegister16::Iy)
            }
            else if indexed_ix.is_match(repr) {
                AddressSource::Indexed(cpclib_asm::preamble::IndexRegister16::Ix)
            }
            else if in_af.is_match(repr) {
                AddressSource::Register16(cpclib_asm::preamble::Register16::Af)
            }
            else if in_hl.is_match(repr) {
                AddressSource::Register16(cpclib_asm::preamble::Register16::Hl)
            }
            else if in_de.is_match(repr) {
                AddressSource::Register16(cpclib_asm::preamble::Register16::De)
            }
            else if in_bc.is_match(repr) {
                AddressSource::Register16(cpclib_asm::preamble::Register16::Bc)
            }
            else if in_ix.is_match(repr) {
                AddressSource::IndexRegister16(cpclib_asm::preamble::IndexRegister16::Ix)
            }
            else if in_iy.is_match(repr) {
                AddressSource::IndexRegister16(cpclib_asm::preamble::IndexRegister16::Iy)
            }
            else if immediate.is_match(repr) {
                AddressSource::Immediate
            }
            else {
                AddressSource::None
            }
        })
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
    Indexed(cpclib_asm::preamble::IndexRegister16)
}

/// Where the CPU searches the information to link with the data bus
#[derive(Debug, PartialEq, Copy, Clone)]
enum DataSource {
    /// No data handled by the opcode
    None,
    /// The data value has been read by the CPU and is stored in extra bytes
    Immediate,
    /// The value is in a 8 bit register
    Register8(cpclib_asm::preamble::Register8),
    /// The value is in a 16 bit register
    Register16(cpclib_asm::preamble::Register16),
    /// The value is in a 16 bits indexed regiser
    IndexRegister16(cpclib_asm::preamble::IndexRegister16),
    /// The value is in a 8 bits indexed regiser
    IndexRegister8(cpclib_asm::preamble::IndexRegister8),

}



/// Simulate the Cycles of hte CPU. No need to simulate the pipeline stuff however
#[derive(Debug, PartialEq)]
enum MCycle {
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
    fn inc_state(&mut self) {

        *self = match self {
            MCycle::FetchOpcode(TState::T1, nb, prefix) => MCycle::FetchOpcode(TState::T2, *nb, *prefix),
            MCycle::FetchOpcode(TState::T2, nb, prefix) => MCycle::FetchOpcode(TState::T3, *nb, *prefix),
            MCycle::FetchOpcode(TState::T3, nb, prefix) => MCycle::FetchOpcode(TState::T4, *nb, *prefix),
            MCycle::FetchOpcode(TState::T4, nb, prefix) => panic!("Ambigous"),


            MCycle::Read(TState::T1, nb) => MCycle::Read(TState::T2, *nb),
            MCycle::Read(TState::T2, nb) => MCycle::Read(TState::T3, *nb),
            MCycle::Read(TState::T3, nb) => panic!("Ambigous"),

            _ => unimplemented!()
        }
    }

    fn prefix(&self) -> OpcodePrefix {
        match self {
            MCycle::FetchOpcode(_, _, prefix) => *prefix,
            _ => unreachable!()
        }
    }
}

// http://www.z80.info/zinout.htm#Z80CPU
#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u64)]
pub enum Z80Pin {
    A11=1<<0,
    A12=1<<1,
    A13=1<<2,
    A14=1<<3,
    A15=1<<4,
    Neg_CLK=1<<5,
    /// Data bus in/out
    D4=1<<6,
    /// Data bus in/out
    D3=1<<7,
    /// Data bus in/out
    D5=1<<8,
    /// Data bus in/out
    D6=1<<9,
    VCC=1<<10,
    /// Data bus in/out
    D2=1<<11,
    /// Data bus in/out
    D7=1<<12,
    /// Data bus in/out
    D0=1<<13,
    /// Data bus in/out
    D1=1<<14,
    Neg_INT=1<<15,
    Neg_NMI=1<<16,
    Neg_HALT=1<<17,
    Neg_MREQ=1<<18,
    Neg_IORQ=1<<19,
    A10=1<<20,
    A9=1<<21,
    A8=1<<22,
    A7=1<<23,
    A6=1<<24,
    A5=1<<25,
    A4=1<<26,
    A3=1<<27,
    A2=1<<28,
    A1=1<<29,
    A0=1<<30,
    GND=1<<31,
    Neg_RFSH=1<<32,
    Neg_M1=1<<33,
    Neg_RESET=1<<34,
    Neg_BUSRQ=1<<35,
    Neg_WAIT=1<<36,
    Neg_BUSACK=1<<37,
    Neg_WR=1<<38,
    Neg_RD=1<<39
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
const Z80_PIN_DATA_BUS : [Z80Pin;8] = [Z80Pin::D7, Z80Pin::D6, Z80Pin::D5, Z80Pin::D4, Z80Pin::D3, Z80Pin::D2, Z80Pin::D1, Z80Pin::D0];

/// Set of Z80Pin for the address bus
const PIN_ADDRESS_BUS : [Z80Pin;16] = [Z80Pin::A15, Z80Pin::A14, Z80Pin::A13, Z80Pin::A12, Z80Pin::A11, Z80Pin::A10, Z80Pin::A9, Z80Pin::A8, Z80Pin::A7, Z80Pin::A6, Z80Pin::A5, Z80Pin::A4, Z80Pin::A3, Z80Pin::A2, Z80Pin::A1, Z80Pin::A0];

const PIN_SYSTEM_CONTROL : [Z80Pin;5] = [Z80Pin::Neg_M1, Z80Pin::Neg_MREQ, Z80Pin::Neg_IORQ, Z80Pin::Neg_RD, Z80Pin::Neg_WR];

const PIN_CPU_CONTROL : [Z80Pin;5]= [Z80Pin::Neg_HALT, Z80Pin::Neg_WAIT, Z80Pin::Neg_INT, Z80Pin::Neg_NMI, Z80Pin::Neg_RESET];

const PIN_CPU_BUS_CONTROL : [Z80Pin;2] = [Z80Pin::Neg_BUSRQ, Z80Pin::Neg_BUSACK];




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
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Register8 {
    /// The 8 bit value of the register
    val: u8,
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

impl Debug for Register16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({:?}, {:?})", &self.high, &self.low)
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
    fn low(&self) -> &Register8 {
        &self.low
    }

    fn high(&self) -> &Register8 {
        &self.high
    }

    fn low_mut(&mut self) -> &mut Register8 {
        &mut self.low
    }

    fn high_mut(&mut self) -> &mut Register8 {
        &mut self.high
    }
}




/// Identifiers for any 16 bits registers
#[derive(Debug)]
enum Reg16 {
    Af,
    Af2,
    Hl,
    Hl2,
    De,
    De2,
    Bc,
    Bc2,
    Wz,
    Wz2,
    Pc,
    Sp,
    Ix,
    Iy
}

/// Identifiers for any 8 bits registers
#[derive(Debug)]
enum Reg8 {
    A,
    A2,
    H,
    H2,
    D,
    D2,
    B,
    B2,
    W,
    W2,
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
    Z2,
    
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
        ! self.is_high()
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
            W2 => true,
            Ixh => true,
            Iyh => true,
        
            _ => false
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
            W2 => Reg16::Wz2,
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
            Z2 => Reg16::Wz2,
            
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
            Wz2 => Reg8::W2,
            Ix => Reg8::Ixh,
            Iy => Reg8::Iyh,
            _ => unreachable!()
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
            Wz2 => Reg8::Z2,
            Ix => Reg8::Ixl,
            Iy => Reg8::Iyl,
            _ => unreachable!()
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




#[derive(Default)]
pub struct Z80 {
    /// Encoding of the 40 Z80Pin
    pins: u64,
    /// State of the Z80 state-machine
    next_state: MCycle,

    pc: Register16,
    sp: Register16,

    af: Register16,
    af2: Register16,

    hl: Register16,
    bc: Register16,
    de: Register16,

    hl2: Register16,
    bc2: Register16,
    de2: Register16,

    ix: Register16,
    iy: Register16,

    wz: Register16,
    wz2: Register16,

    i: Register8,
    r: Register8,

    /// Instruction Register / contains the opcode of interest
    ir: Register8,
    /// Additional bytes read during the fetch pass
    opcode_extra_byte: Vec<u8>,
    /// Additioanl bytes read during the read pass
    memory_read: Vec<u8>,
  

    /// Refresh adr
    /// XXX Not handled at all by the emulator
    refresh_addr: u16,

    /// Information for the current opcode - Setup after fetching the very first byte
    opcode_information: Option<OpcodeInformation>
}

impl Z80 {

    /// During the fetch cycle, the CPU collected the information to obtain the address to read/write
    /// This method obtains it
    fn get_read_address_from_opcode_information(&self) -> u16 {
        match self.opcode_information.as_ref() {
            Some(oi) => {
                match oi.read_address {
                    AddressSource::None => unreachable!(),
                    AddressSource::Immediate => {
                        assert_eq!(self.opcode_extra_byte.len(), 2, "Impossible or unimplemented case");
                        256 * self.opcode_extra_byte[1] as u16 + self.opcode_extra_byte[0] as u16 
                    },
                    AddressSource::Register16(ref reg) => {
                        self.reg16(reg.into()).get()
                    }
                    ,
                    AddressSource::IndexRegister16(ref reg) => {
                        self.reg16(reg.into()).get()
                    }
                    ,
                    AddressSource::Indexed(ref reg) => {
                        // by definition, delta is within the first byte
                        self.reg16(reg.into()).get() + self.opcode_extra_byte[0] as u16
                    }
                }
            },
            None => unreachable!()
        }
    }

    
    /// Execute the instruction ; all necessary stuffs has already been collected
    fn execute_step(&mut self) {
        let prefix = &self.opcode_information.as_ref().unwrap().prefix;

        // By construction, it cannot fail
        let (opcode,rest) = dbg!(cpclib_asm::disass::disassemble_with_potential_argument(
            self.ir.get(),
            &prefix.disassembling_table(),
            &self.opcode_extra_byte
        ).unwrap());
        assert_eq!(rest.len(), 0);
        assert!(opcode.is_opcode());
        self.execute_opcode(
            opcode.mnemonic().unwrap(),
            opcode.mnemonic_arg1(),
            opcode.mnemonic_arg2(),
        );

        // reset the various buffers
        self.opcode_information = None;
        self.opcode_extra_byte.clear();
        self.memory_read.clear();
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
                    self.next_state = MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None);
                    return true;
                }
                else if opcode_information.nb_opcode_bytes != 0 {
                    self.next_state = MCycle::FetchOpcode(TState::T1, 1, current_prefix);
                    return false;
                }
                else if opcode_information.expect_read() {
                    self.next_state = MCycle::Read(TState::T1, 0);
                    return false;
                }
                else {
                    unimplemented!()
                }
            }
        }
    }


    /// The complete opcode has been fetched and pc has been properly incremented.
    /// This function then execute the opcode
    fn execute_opcode(&mut self, opcode: &Mnemonic, arg1: Option<&DataAccess>, arg2: Option<&DataAccess>) {
        match opcode {
            Mnemonic::Add => {
                match arg1.unwrap() {
                    DataAccess::Register8(_) => { // this MUST be A register
                        // 8bits version
                        let right = self.get_available_8bits_value(arg2.unwrap());
                        let ovf = self.reg8_mut(Reg8::A).add(right); // TODO manage the flags
                    },
                    _ => unimplemented!() 
                }

            },
            Mnemonic::Inc => {
                let ovf = match arg1.unwrap() {
                    DataAccess::IndexRegister8(reg) => {
                        let left: Reg8 = reg.into();
                        self.reg8_mut(left).inc()
                    },
                    _ => unimplemented!() 
                };
            },
            Mnemonic::Ld => {
                match arg1.unwrap() {
                    DataAccess::Register8(reg) => {
                        // 8bits version
                        let left: Reg8 = reg.into();
                        let right = self.get_available_8bits_value(arg2.unwrap());
                        self.reg8_mut(left).set(right);
                    },

                    _ => unimplemented!()
                }

            }
            Mnemonic::Nop => {
            },
            Mnemonic::Xor => {
                let right = dbg!(self.get_available_8bits_value(arg1.unwrap()));
                self.reg8_mut(Reg8::A).xor(right);
            },
            _ => unimplemented!()
        }
    }

    /// Return the value for this data access
    /// - Inside a register / straightworard to obtain
    /// - In memory / the cpu has read it and stored it in XXX
    /// - In chip / the cpu has read it and stored it in XXX
    fn get_available_8bits_value(&self, data: &DataAccess) -> u8 {
        match data {
            DataAccess::Register8(reg) => {
                let reg: Reg8 = dbg!(reg.into());
                return self.reg8(reg).get();
            },
            DataAccess::IndexRegister8(reg) => {
                let reg: Reg8 = reg.into();
                return self.reg8(reg).get();
            },
            DataAccess::MemoryRegister16(_reg) => {
                // Here we do not care at all of which register is used for memory reading
                // Because the CPU has already read the information
                assert_eq!(1, self.memory_read.len());
                return self.memory_read[0];
            },
            DataAccess::Expression(expr) => {
                return expr.eval().unwrap() as u8; // cannot fail as the value has been disassembled
            }

            _ => unimplemented!()
        }
    }

    /// Get the appropriate (inner) 16bits register in a mutable state
    fn reg16_mut(&mut self, reg: Reg16) -> &mut Register16 {
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
            Reg16::Wz2 => &mut self.wz2,

            Reg16::Ix => &mut self.ix,
            Reg16::Iy => &mut self.iy,
            Reg16::Pc => &mut self.pc,
            Reg16::Sp => &mut self.sp,
        }
    }

    fn reg16(& self, reg: Reg16) -> & Register16 {
        match reg {
            Reg16::Af => & self.af,
            Reg16::Hl => & self.hl,
            Reg16::De => & self.de,
            Reg16::Bc => & self.bc,
            Reg16::Wz => & self.wz,

            Reg16::Af2 => & self.af2,
            Reg16::Hl2 => & self.hl2,
            Reg16::De2 => & self.de2,
            Reg16::Bc2 => & self.bc2,
            Reg16::Wz2 => & self.wz2,

            Reg16::Ix => & self.ix,
            Reg16::Iy => & self.iy,
            Reg16::Pc => & self.pc,
            Reg16::Sp => & self.sp,
        }
    }

    fn reg8_mut(&mut self, reg: Reg8) -> &mut Register8 {
        let r16 = self.reg16_mut(reg.full());
        if reg.is_high() {
            r16.high_mut()
        } else {
            r16.low_mut()
        }
    }

    fn reg8(&self, reg: Reg8) -> & Register8 {
        let r16 = self.reg16(reg.full());
        if reg.is_high() {
            r16.high()
        } else {
            r16.low()
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



impl WriteAddressChip for Z80 {

}

impl ReadAddressChip for Z80 {

}

impl WriteDataChip for Z80 {

}

impl ReadDataChip  for Z80 {

}

impl Chip for Z80 {
    type PinType = Z80Pin;

    
    fn pins_buffer_mut(&mut self) -> &mut u64 {
        &mut self.pins
    }
    fn pins_buffer(&self) -> & u64 {
        & self.pins
    }


    fn must_execute_tick(&self) -> bool {
        self.get_pin(Z80Pin::Neg_WAIT) // TODO do that only for Fetch/T2
    }

    fn execute_tick(&mut self) {
        dbg!(&self.next_state) ;
        dbg!(&self.opcode_information) ;

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
            },

            // Second step / may be delayed if wait is activated
            MCycle::FetchOpcode(TState::T2, nb,_) => {

                self.set_pin(Z80Pin::Neg_RD, true);
                self.set_pin(Z80Pin::Neg_MREQ, true);
                self.set_pin(Z80Pin::Neg_M1, true);

                let byte = self.get_data_from_pins();
                match nb {
                    0 => {
                        self.ir.set(byte);
                        self.opcode_extra_byte.clear();
                    },
                    _ => { // TODO find where such information is really saved
                        self.opcode_extra_byte.push(byte);
                    }
                }
                self.set_pin(Z80Pin::Neg_RFSH, false);
                self.set_address_on_pins(self.refresh_addr);
                self.next_state.inc_state();
            },

            MCycle::FetchOpcode(TState::T3, _,_) => {
                // we do nothing here
                // but, IRL, the decoding appears there
                self.next_state.inc_state();
                self.set_pin(Z80Pin::Neg_MREQ, false);
            },
            
            MCycle::FetchOpcode(TState::T4, nb, prefix) => {
                self.set_pin(Z80Pin::Neg_MREQ, false);
                self.set_pin(Z80Pin::Neg_M1, false);
                self.set_pin(Z80Pin::Neg_RFSH, true);

                if nb == 0 {
                    let execute = self.decode_step(); // setup next state
                    if execute {
                        // nothing is required for the execution
                         // TODO check if input or memory cycle is required
                        self.execute_step();
                    }
                } else {
                    if nb == self.opcode_information.as_ref().unwrap().nb_opcode_bytes {
                        if 0 == self.opcode_information.as_ref().unwrap().nb_bytes_to_read {
                            self.next_state = MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None); // go back for a new instruction
                            self.execute_step();
                        }
                        else {
                            self.next_state = MCycle::Read(TState::T1, 0);
                        }
                    } 
                    else {
                        // we still need to read something
                        self.next_state = MCycle::FetchOpcode(TState::T1, nb+1, prefix);
                    }
                }

               
            },


            MCycle::Read(TState::T1, nb) => {
                let address = self.get_read_address_from_opcode_information();
                self.set_address_on_pins(address);
                self.set_pin(Z80Pin::Neg_MREQ, false);
                self.set_pin(Z80Pin::Neg_RD, false);
                self.next_state.inc_state();
            },
            MCycle::Read(TState::T2, nb) => {
                self.set_pin(Z80Pin::Neg_RD, true);
                self.set_pin(Z80Pin::Neg_MREQ, true);
                let byte = self.get_data_from_pins();
                self.memory_read.push(byte);
                self.next_state.inc_state();
            },
            MCycle::Read(TState::T3, nb) => {
                if self.memory_read.len() as u8 == self.opcode_information.as_ref().unwrap().nb_bytes_to_read {
                    // TODO take into account those that do a write after the read (if that exists)
                    self.execute_step();
                    self.next_state = MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None); // go back for a new instruction

                } else {
                    // go to read the next byte
                    self.next_state = MCycle::Read(TState::T1, nb+1);
                }

            }



            _ => unimplemented!()
        }
    }


    fn reset(&mut self) {
        self.pins = 0;

        self.set_pin(Z80Pin::Neg_MREQ, true);
        self.set_pin(Z80Pin::Neg_RD, true);
        self.set_pin(Z80Pin::Neg_M1, true);
        self.set_pin(Z80Pin::Neg_RFSH, true);
        self.set_pin(Z80Pin::Neg_WR, true);


        self.next_state = MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None);
        self.pc.set(0);
    }
}


#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_pins_indices() {
        assert_eq!(Z80Pin::A11 as u64, 1<<0);
        assert_eq!(Z80Pin::A12 as u64, 1<<1);
        assert_eq!(Z80Pin::A13 as u64, 1<<2);
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
        
    fn test_byte_FetchOpcode(cpu: &mut Z80, byte: u8) {

        let pc_base = cpu.reg16(Reg16::Pc).get();


        cpu.set_pin(Z80Pin::Neg_MREQ, true);
        cpu.set_pin(Z80Pin::Neg_RD, true);
        cpu.set_pin(Z80Pin::Neg_WAIT, true);
        cpu.set_pin(Z80Pin::Neg_M1, true);
        cpu.set_pin(Z80Pin::Neg_RFSH, true);
        cpu.set_pin(Z80Pin::Neg_IORQ, true);
        cpu.set_pin(Z80Pin::Neg_WR, true);
        

        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None));
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

        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T2, 0, OpcodePrefix::None));
    
        // GA set wait
        cpu.set_pin(Z80Pin::Neg_WAIT, false);
        cpu.tick();
        assert_eq!(cpu.pc.get(), pc_base + 1);
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T2, 0, OpcodePrefix::None));
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
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T3, 0, OpcodePrefix::None));

        cpu.tick();
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T4, 0, OpcodePrefix::None));

        cpu.tick();
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None));
    }


    fn test_byte_FetchOpcodeRead(cpu: &mut Z80, byte: u8, data: u8) {

        let pc_base = cpu.reg16(Reg16::Pc).get();


        cpu.set_pin(Z80Pin::Neg_MREQ, true);
        cpu.set_pin(Z80Pin::Neg_RD, true);
        cpu.set_pin(Z80Pin::Neg_WAIT, true);
        cpu.set_pin(Z80Pin::Neg_M1, true);
        cpu.set_pin(Z80Pin::Neg_RFSH, true);
        cpu.set_pin(Z80Pin::Neg_IORQ, true);
        cpu.set_pin(Z80Pin::Neg_WR, true);
        

        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None));
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

        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T2, 0, OpcodePrefix::None));
    
        // GA set wait
        cpu.set_pin(Z80Pin::Neg_WAIT, false);
        cpu.tick();
        assert_eq!(cpu.pc.get(), pc_base + 1);
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T2, 0, OpcodePrefix::None));
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
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T3, 0, OpcodePrefix::None));

        cpu.tick();
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T4, 0, OpcodePrefix::None));

        cpu.tick(); // execute
        assert_eq!(cpu.next_state, MCycle::Read(TState::T1, 0));

        ///TODO add assertions there
        cpu.tick();
        cpu.set_data_on_pins(data);
        cpu.tick();
        cpu.tick();
    }




    fn test_byte_FetchOpcode2(cpu: &mut Z80, byte1: u8, byte2: u8) {

        let pc_base = cpu.reg16(Reg16::Pc).get();


        cpu.set_pin(Z80Pin::Neg_MREQ, true);
        cpu.set_pin(Z80Pin::Neg_RD, true);
        cpu.set_pin(Z80Pin::Neg_WAIT, true);
        cpu.set_pin(Z80Pin::Neg_M1, true);
        cpu.set_pin(Z80Pin::Neg_RFSH, true);
        cpu.set_pin(Z80Pin::Neg_IORQ, true);
        cpu.set_pin(Z80Pin::Neg_WR, true);
        

        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None));
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

        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T2, 0, OpcodePrefix::None));
    
        // GA set wait
        cpu.set_pin(Z80Pin::Neg_WAIT, false);
        cpu.tick();
        assert_eq!(cpu.pc.get(), pc_base + 1);
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T2, 0, OpcodePrefix::None));
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
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T3, 0, OpcodePrefix::None));

        cpu.tick();
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T4, 0, OpcodePrefix::None));

        cpu.tick();
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T1, 1, OpcodePrefix::None));



        cpu.tick();
        let addr = cpu.get_address_from_pins();
        assert_eq!(addr, pc_base+1);
        assert_eq!(cpu.pc.get(), pc_base + 2);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_MREQ), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_M1), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RD), false);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_IORQ), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_WR), true);
        assert_eq!(cpu.get_pin(Z80Pin::Neg_RFSH), true);

        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T2, 1, OpcodePrefix::None));
    
        // GA set wait
        cpu.set_pin(Z80Pin::Neg_WAIT, false);
        cpu.tick();
        assert_eq!(cpu.pc.get(), pc_base + 2);
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T2, 1, OpcodePrefix::None));
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
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T3, 1, OpcodePrefix::None));

        cpu.tick();
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T4, 1, OpcodePrefix::None));

        cpu.tick();
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None));
    }



    

    fn test_byte_FetchOpcode_prefix(cpu: &mut Z80, byte1: u8, byte2: u8) {

        let prefix = match byte1 {
            0xdd => OpcodePrefix::Dd,
            _ => unimplemented!()
        };

        let pc_base = cpu.reg16(Reg16::Pc).get();


        cpu.set_pin(Z80Pin::Neg_MREQ, true);
        cpu.set_pin(Z80Pin::Neg_RD, true);
        cpu.set_pin(Z80Pin::Neg_WAIT, true);
        cpu.set_pin(Z80Pin::Neg_M1, true);
        cpu.set_pin(Z80Pin::Neg_RFSH, true);
        cpu.set_pin(Z80Pin::Neg_IORQ, true);
        cpu.set_pin(Z80Pin::Neg_WR, true);
        

        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None));
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

        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T2, 0, OpcodePrefix::None));
    
        // GA set wait
        cpu.set_pin(Z80Pin::Neg_WAIT, false);
        cpu.tick();
        assert_eq!(cpu.pc.get(), pc_base + 1);
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T2, 0, OpcodePrefix::None));
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
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T3, 0, OpcodePrefix::None));

        cpu.tick();
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T4, 0, OpcodePrefix::None));

        cpu.tick();
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T1, 0, prefix));



        cpu.tick();
        let addr = cpu.get_address_from_pins();
        assert_eq!(addr, pc_base+1);
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
        assert_eq!(cpu.next_state, MCycle::FetchOpcode(TState::T1, 0, OpcodePrefix::None));
    }
}