
///! This module contains all that can help to manage the variuous states of the CPU
use lazy_static::lazy_static;
use regex::Regex;
//use cpclib_asm::preamble::*;
use super::{Reg16, Reg8, Z80Pin};
use cpclib_asm::preamble::{DataAccess, Mnemonic, Token};

// Generate the various lookup tables needed to control the opcodes execution
// rust is currently unable to do that in a static way
lazy_static! {
    pub(crate) static ref OPCODE_PREFIX: [OpcodeInformation; 256] =
    array_init::array_init(|i| OpcodePrefix::None.build_opcode_information(i as u8));
    pub(crate) static ref OPCODE_PREFIX_CB: [OpcodeInformation; 256] =
    array_init::array_init(|i| OpcodePrefix::Cb.build_opcode_information(i as u8));
    pub(crate) static ref OPCODE_PREFIX_DD: [OpcodeInformation; 256] =
    array_init::array_init(|i| OpcodePrefix::Dd.build_opcode_information(i as u8));
    pub(crate) static ref OPCODE_PREFIX_ED: [OpcodeInformation; 256] =
    array_init::array_init(|i| OpcodePrefix::Ed.build_opcode_information(i as u8));
    pub(crate) static ref OPCODE_PREFIX_FD: [OpcodeInformation; 256] =
    array_init::array_init(|i| OpcodePrefix::Fd.build_opcode_information(i as u8));
}

/// Possible states for a cycle
/// Warning: Wait state does not count
#[derive(Debug, PartialEq)]
pub enum TState {
    T1,
    T2,
    T3,
    T4,
}

/// This enumeration aims at helping to treat the
/// prefixed operations
#[derive(Debug, PartialEq, Copy, Clone)]
/// TODO modify for multiprefix opcodes
pub enum OpcodePrefix {
    None = 0x00,
    Dd = 0xdd,
    Fd = 0xfd,
    Cb = 0xcb,
    Ed = 0xed,
}

/// Contains all the informations needed to drive the next
/// cycles of the instruction
#[derive(Debug, PartialEq)]
pub struct OpcodeInformation {
    /// Prefix for the current opcode
    pub(crate) prefix: OpcodePrefix,
    /// Number of bytes of the opcode that need to be fetched
    pub(crate) nb_additional_opcode_bytes: u8,
    /// Location of the reading address
    pub(crate) read_address: AddressSource,
    /// Location of the write address (memory or output)
    pub(crate) write_address: AddressSource,
    /// Write data
    pub(crate) write_data: DataSource,
    ///Read data
    pub(crate) read_data: DataDestination,
    /// Number of bytes to read from the address
    pub(crate) nb_bytes_to_read: u8,
    /// Number of bytes the instruction request to write (in memory or chip)
    pub(crate) nb_bytes_to_write: u8,
    /// Read in memory ? (otherwise chips) / No meaning when no read
    pub(crate) read_from_pins: bool,
    /// Write in memory ? (otherwise chip) / No meaning when no write
    pub(crate) write_to_pins: bool,
    /// String representation
    pub(crate) repr: &'static str,
    /// Flag test for conditional execution of instruction
    pub(crate) flag_test: Option<cpclib_asm::preamble::FlagTest>,
    //   /// Token just in case it is usefull (to be removed if not)
    pub(crate) token: Token,
}

/// Where the CPU searches the address information to be put on the address line
#[derive(Debug, PartialEq, Copy, Clone)]
pub(crate) enum AddressSource {
    /// No address handled by the opcode
    None,
    /// The address has been read by the CPU and is stored in extra bytes
    Immediate,
    /// The address is contained inside a 16 bits register
    Register16(Reg16),
    /// The address is contained in a 16 bits indexed regiser
    IndexRegister16(Reg16),
    /// The delta has already been read by the CPU
    Indexed(Reg16),
    /// We are doing an operation on the stack / thus address is SP
    Stack(StackDelta),
}

// Some operations use a modified value of the the SP register
#[derive(Debug, PartialEq, Copy, Clone)]
pub enum StackDelta {
    /// Use the stack value
    Zero,
    /// Use the stack value - 2
    MinusTwo,
}

impl Default for TState {
    fn default() -> Self {
        TState::T1
    }
}

impl Default for OpcodePrefix {
    fn default() -> Self {
        OpcodePrefix::None
    }
}

impl OpcodeInformation {
    pub fn write_on_stack(&self) -> bool {
        self.write_address == AddressSource::Stack(StackDelta::MinusTwo)
    }

    /// Fetch is finished for opcode of 1 byte with no memory/port access
    pub fn fetch_finished(&self) -> bool {
        self.nb_additional_opcode_bytes == 0
    }
    
    /// Check if CPU as to read from bus
    pub fn expect_read(&self) -> bool {
        self.nb_bytes_to_read != 0
    }
    
    /// Check if CPU as to write in bus
    pub fn expect_write(&self) -> bool {
        self.nb_bytes_to_write != 0
    }
}

impl From<u8> for OpcodePrefix {
    fn from(value: u8) -> Self {
        match value {
            0xcb => Self::Cb,
            0xdd => Self::Dd,
            0xed => Self::Ed,
            0xfd => Self::Fd,
            _ => Self::None,
        }
    }
}
impl OpcodePrefix {
    /// Get the opcode information from the LUT
    pub fn opcode_information(&self, opcode: u8) -> &'static OpcodeInformation {
        let table: &[OpcodeInformation; 256] = match self {
            Self::None => &OPCODE_PREFIX,
            Self::Cb => &OPCODE_PREFIX_CB,
            Self::Dd => &OPCODE_PREFIX_DD,
            Self::Ed => &OPCODE_PREFIX_ED,
            Self::Fd => &OPCODE_PREFIX_FD,
        };
        &table[opcode as usize]
    }
    
    /// Retreive all the information needed to data collection for the current opcode
    fn build_opcode_information(&self, opcode: u8) -> OpcodeInformation {
        let opcode = opcode as usize;
        
        let repr = self.disassembling_table()[opcode];
        let token = cpclib_asm::parser::parse_token(if repr.is_empty() { "NOP" } else { repr })
        .unwrap()
        .1;
        
        let read_from_pins = token.is_input_opcode() ;
        let write_to_pins = token.is_output_opcode();
        
        
        // For instructions reading from memory, get the location of the address
        let read_address = if token.is_input_opcode() {
            self.input_address_source(&token)
        } else if token.is_output_opcode() {
            if token.mnemonic_arg1().is_none() {
                // The instructions with no arg are those that read from memory
                AddressSource::Register16(Reg16::Hl)
            } else {
                AddressSource::None
            }
        } else if token.is_retlike_opcode() {
            AddressSource::Stack(StackDelta::Zero)
        } else if matches!(token, Token::OpCode(Mnemonic::ExMemSp, _, _)) {
            AddressSource::Stack(StackDelta::Zero)
        } else if !read_from_pins {
            self.read_address_source(opcode)
        } else {
            AddressSource::None
        };
        
        // For instructions writting ot memory, get the location of address
        let write_address = if token.is_output_opcode() {
            self.output_address_source(&token)
        }
        else if let Some(Mnemonic::ExMemSp) = token.mnemonic() {
            AddressSource::Stack(StackDelta::Zero)
        }
        else if !write_to_pins {
            self.write_address_source(opcode)
        } else {
            AddressSource::None
        };
        
        // Location of the byte to write on data
        let write_data: DataSource = if token.is_output_opcode() {
            match token.mnemonic_arg2() {
                Some(position) => position.into(),
                None => DataSource::Pins, // the data is already on the pins
            }
        } else if let Some(Mnemonic::ExMemSp) = token.mnemonic() {
            let reg = match token.mnemonic_arg1() {
                Some(DataAccess::Register16(reg)) => reg.into(),
                Some(DataAccess::IndexRegister16(reg)) => reg.into(),
                _ => unreachable!()
            };
            DataSource::Register16(reg)
        }
        else {
            // opcode that is not an output
            match write_address {
                AddressSource::None => DataSource::None,
                AddressSource::Stack(_) => match token.mnemonic() {
                    Some(Mnemonic::Call) => DataSource::Register16(Reg16::Pc),
                    Some(Mnemonic::Push) => {
                        let reg: Reg16 = match token.mnemonic_arg1() {
                            Some(DataAccess::Register16(reg)) => reg.into(),
                            Some(DataAccess::IndexRegister16(reg)) => reg.into(),
                            _ => unreachable!(),
                        };
                        DataSource::Register16(reg)
                    }
                    
                    _ => unreachable!(),
                },
                _ => match token.mnemonic_arg2() {
                    Some(stuff) => stuff.into(),
                    None => DataDestination::Pins,
                },
            }
        };
        
        let mnemonic = token.mnemonic().unwrap();
        
        // Location where to store the data to read
        let read_data: DataDestination = 
        // input like opcodes
        if token.is_input_opcode() {
            match token.mnemonic_arg1() {
                Some(position) => position.into(),
                None => DataDestination::Pins,
            }
        } 
        // retlike opcodes
        else if token.is_retlike_opcode() {
            // stack content is stored in PC
            DataDestination::Register16(Reg16::Pc)
        } 
        // ld like opcocdes
        else if let Token::OpCode(
            Mnemonic::Ld,
            Some(DataAccess::Register16(reg)),
            Some(DataAccess::Memory(_)),
        ) = token
        {
            DataDestination::Register16(reg.into())
        } else if let Token::OpCode(
            Mnemonic::Ld,
            Some(DataAccess::IndexRegister16(reg)),
            Some(DataAccess::Memory(_)),
        ) = token
        {
            DataDestination::IndexRegister16(reg.into())
        } else if let Token::OpCode(
            Mnemonic::Ld,
            Some(DataAccess::Register8(ref reg)),
            Some(DataAccess::Memory(_) | DataAccess::MemoryRegister16(_) | DataAccess::IndexRegister16WithIndex(_, _)),
        ) = token
        {
            DataDestination::Register8(reg.into())
        } 
        // pop
        else if let Token::OpCode(Mnemonic::Pop, Some(DataAccess::Register16(reg)), _) = token {
            DataDestination::Register16(reg.into())
        } 
        else if let Token::OpCode(Mnemonic::Pop, Some(DataAccess::IndexRegister16(reg)), _) = token {
            DataDestination::Register16(reg.into())
        } 
        // dec-inc
        else if let Token::OpCode(
            Mnemonic::Inc | Mnemonic::Dec,
            Some(DataAccess::MemoryRegister16(_) | DataAccess::IndexRegister16WithIndex(_, _)),
            _,
        ) = token
        {
            DataDestination::Pins // here we read/write on the data pins
        } 
        // JP / jr
        else if let Token::OpCode( Mnemonic::Jp | Mnemonic::Jr, _, Some(DataAccess::MemoryRegister16(_) | DataAccess::MemoryIndexRegister16(_) )) = token {
            DataDestination::Register16(Reg16::Wz)
        }
        // others (mathematicla ?)
        else if let Token::OpCode(_, _, Some(DataAccess::MemoryRegister16(_) | DataAccess::IndexRegister16WithIndex(_, _))) = token {
            DataDestination::Pins
        } else if let Token::OpCode(_, Some(DataAccess::MemoryRegister16(_)| DataAccess::IndexRegister16WithIndex(_, _)), None) = token {
            DataDestination::Pins
        }
        // repetition 
        else if matches!(
            mnemonic, 
            Mnemonic::Outi | Mnemonic::Outd
            | Mnemonic::Otir | Mnemonic::Otdr
            | Mnemonic::Ldi | Mnemonic::Ldd
            | Mnemonic::Ldir | Mnemonic::Lddr
            
        )
        {
            DataDestination::Pins
        } 
        else if let Some(Mnemonic::ExMemSp) = token.mnemonic() {
            DataDestination::Register16(Reg16::Wz)
        }
        else {
            assert_eq!(AddressSource::None, read_address, "{:?}", token);
            DataDestination::None // TODO implement the other ones
        };
        /*
        // get the number of bytes needed to build the complete opcode
        let nb_bytes_to_read = if token.is_output_opcode() {
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
        */
        let nb_bytes_to_write = if let Some(Mnemonic::ExMemSp) = token.mnemonic() {
            2
        }
        else {match write_data {
            DataSource::None => 0,
            DataSource::Register16(_) => 2,
            DataSource::IndexRegister16(_) => 2,
            DataSource::Register8(_) => 1,
            DataSource::IndexRegister8(_) => 1,
            DataSource::Immediate => 1,           // XXX not sure of that
            DataSource::MemoryRegister16(_) => 1, // XXX not sure of that see if we keep
            DataSource::Pins => 1,                // the byte is already on the pins
        }
    };
    
    if read_address != AddressSource::None {
        assert!(
            read_data != DataSource::None,
            "{:?} / {:?} expect read_data to be filled",
            token,
            read_address
        );
    }
    
    let nb_bytes_to_read = if let Some(Mnemonic::ExMemSp) = token.mnemonic() {
        2
    }
    else if token.is_output_opcode() {
        if let Some(DataAccess::PortC) = token.mnemonic_arg1() {
            0
        } else {
            1 // port n belongs to the opcode
        }
    } else {
        match read_data {
            DataSource::None => 0,
            DataSource::Register16(_) => 2,
            DataSource::IndexRegister16(_) => 2,
            DataSource::Register8(_) => 1,
            DataSource::IndexRegister8(_) => 1,
            DataSource::Immediate => 1, // XXX not sure of that
            DataSource::MemoryRegister16(_) => 1, // XXX not sure of that see if we keep
            DataSource::Pins => 1,      // the byte is already on the pins
        }
    };
    
    let flag_test = match token.mnemonic_arg1() {
        Some(DataAccess::FlagTest(f)) => Some(*f),
        _ => None,
    };
    
    OpcodeInformation {
        prefix: self.clone(),
        nb_additional_opcode_bytes: self.opcode_bytes_to_read(opcode),
        read_address,
        write_address,
        write_data,
        read_data,
        nb_bytes_to_read,
        nb_bytes_to_write,
        repr,
        read_from_pins,
        write_to_pins,
        token,
        flag_test,
    }
}

/// Return the disassembling LUT table depending on the prefix
pub const fn disassembling_table(&self) -> &[&'static str; 256] {
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

/// Obtain the address source from general instructions. Some specific ones are handled somewhere manually.
/// TODO : merge the two places ...
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
        static ref POP: Regex = Regex::new(r"^POP").unwrap();
        static ref LDSTAR: Regex = Regex::new(r"(LDD)|(LDI)R?").unwrap();
    }
    
    let repr = self.disassembling_table()[i];
    
    if LDSTAR.is_match(repr) {
        AddressSource::Register16(Reg16::Hl)
    }
    else if POP.is_match(repr) {
        AddressSource::Stack(StackDelta::Zero)
    } else if INDEXED_IY.is_match(repr) {
        AddressSource::Indexed(Reg16::Iy)
    } else if INDEXED_IX.is_match(repr) {
        AddressSource::Indexed(Reg16::Ix)
    } else if IN_AF.is_match(repr) {
        AddressSource::Register16(Reg16::Af)
    } else if IN_HL.is_match(repr) {
        AddressSource::Register16(Reg16::Hl)
    } else if IN_DE.is_match(repr) {
        AddressSource::Register16(Reg16::De)
    } else if IN_BC.is_match(repr) {
        AddressSource::Register16(Reg16::Bc)
    } else if IN_IX.is_match(repr) {
        AddressSource::IndexRegister16(Reg16::Ix)
    } else if IN_IY.is_match(repr) {
        AddressSource::IndexRegister16(Reg16::Iy)
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
        Some(DataAccess::PortC) | None => AddressSource::Register16(Reg16::Bc),
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
        Some(DataAccess::PortC) | None => AddressSource::Register16(Reg16::Bc),
        Some(DataAccess::PortN(_)) => AddressSource::Immediate,
        _ => unreachable!("{:?}", token),
    }
}

/// Provide the location of the address to put on the address bus to write in memory  from general instructions. Some specific ones are handled somewhere manually.
/// TODO : merge the two places ...
fn write_address_source(self, i: usize) -> AddressSource {
    lazy_static! {
        static ref INDEXED_IY: Regex = Regex::new(r"\(IY\+nn\)[^$]").unwrap();
        static ref INDEXED_IX: Regex = Regex::new(r"\(IX\+nn\)[^$]").unwrap();
        static ref IN_HL: Regex = Regex::new(r"\(HL\)[^$]").unwrap();
        static ref IN_HL_END: Regex = Regex::new(r"\(HL\)$").unwrap();
        static ref IN_BC: Regex = Regex::new(r"\(BC\)[^$]").unwrap();
        static ref IN_DE: Regex = Regex::new(r"\(DE\)[^$]").unwrap();
        static ref IN_AF: Regex = Regex::new(r"\(AF\)[^$]").unwrap();
        static ref IN_IX: Regex = Regex::new(r"\(IX\)[^$]").unwrap();
        static ref IN_IY: Regex = Regex::new(r"\(IY\)[^$]").unwrap();
        static ref IMMEDIATE: Regex = Regex::new(r"\(nnnn\)[^$]").unwrap();
        static ref CALL: Regex = Regex::new(r"^CALL").unwrap();
        static ref PUSH: Regex = Regex::new(r"^PUSH").unwrap();
        static ref SET: Regex = Regex::new(r"^SET").unwrap();
        static ref INC: Regex = Regex::new(r"^INC").unwrap();
        static ref DEC: Regex = Regex::new(r"^DEC").unwrap();
        static ref LDSTAR: Regex = Regex::new(r"(LDD)|(LDI)R?").unwrap();
    }
    
    let repr = self.disassembling_table()[i];
    
    if LDSTAR.is_match(repr) {
        AddressSource::Register16(Reg16::De)
    }
    else if IN_HL_END.is_match(repr)
    && (SET.is_match(repr) || INC.is_match(repr) || DEC.is_match(repr))
    {
        AddressSource::Register16(Reg16::Hl)
    } else if CALL.is_match(repr) || PUSH.is_match(repr) {
        AddressSource::Stack(StackDelta::MinusTwo)
    } else if INDEXED_IY.is_match(repr) {
        AddressSource::Indexed(Reg16::Iy)
    } else if INDEXED_IX.is_match(repr) {
        AddressSource::Indexed(Reg16::Ix)
    } else if IN_AF.is_match(repr) {
        AddressSource::Register16(Reg16::Af)
    } else if IN_HL.is_match(repr) {
        AddressSource::Register16(Reg16::Hl)
    } else if IN_DE.is_match(repr) {
        AddressSource::Register16(Reg16::De)
    } else if IN_BC.is_match(repr) {
        AddressSource::Register16(Reg16::Bc)
    } else if IN_IX.is_match(repr) {
        AddressSource::IndexRegister16(Reg16::Ix)
    } else if IN_IY.is_match(repr) {
        AddressSource::IndexRegister16(Reg16::Iy)
    } else if IMMEDIATE.is_match(repr) {
        AddressSource::Immediate
    } else {
        AddressSource::None
    }
}
}

/// Where the CPU searches the information to link with the data bus
#[derive(Debug, PartialEq, Copy, Clone)]
pub(crate) enum DataSource {
    /// No data handled by the opcode
    None,
    /// The data value has been read by the CPU and is stored in extra bytes
    Immediate,
    /// Data stays on the pins
    Pins,
    /// The value is in a 8 bit register
    Register8(Reg8),
    /// The value is in a 16 bit register
    Register16(Reg16),
    /// The value is in a 16 bits indexed register
    IndexRegister16(Reg16),
    /// The value is in a 8 bits indexed register
    IndexRegister8(Reg8),
    /// The value must be read from memory and tḧe address is in the register
    MemoryRegister16(cpclib_asm::preamble::Register16),
}

pub(crate) type DataDestination = DataSource;

impl From<&DataAccess> for DataSource {
    fn from(d: &DataAccess) -> DataSource {
        match d {
            DataAccess::IndexRegister16(reg) => DataSource::IndexRegister16(reg.into()),
            DataAccess::IndexRegister8(reg) => DataSource::IndexRegister8(reg.into()),
            DataAccess::Register16(reg) => DataSource::Register16(reg.into()),
            DataAccess::Register8(reg) => DataSource::Register8(reg.into()),
            DataAccess::Expression(_) => DataSource::Immediate, // to be read
            DataAccess::MemoryRegister16(_) => DataSource::Pins, // what has bin read is stored in the pins
            _ => panic!("{:?} unimplemented", d),
        }
    }
}

/// Simulate the Cycles of the CPU. No need to simulate the pipeline stuff however
#[derive(Debug, PartialEq)]
pub enum MCycle {
    /// M1 (State, byte number, opcode prefix)
    /// - T1: request the reading of a byte
    /// - T2: read the byte
    /// - T3&T4: memory refresh / decode the opcode
    FetchOpcode(TState, u8, OpcodePrefix),
    /// Read state (state, byte index to read) opciode information is stored by CPU
    Read(TState, u8),
    /// Write in memory
    Write(TState, u8),
    /// Read from a chip
    Input(TState, u8),
    /// Write to a chip
    Output(TState, u8),
    /// Fake state to simulate additional computations
    Fake(TState, u8)
}

impl Default for MCycle {
    fn default() -> Self {
        MCycle::FetchOpcode(TState::default(), 0, OpcodePrefix::None)
    }
}
impl ToString for MCycle {
    fn to_string(&self) -> String {
        let inner = match self {
            Self::FetchOpcode(t, b , pr) => {
                format!("F({:#?},{},{:#?})", t, b, pr)
            },
            Self::Read(t, b) => {
                format!("R({:#?},{})", t, b)
            },
            Self::Write(t, b) => {
                format!("W({:#?},{})", t, b)
            },
            Self::Input(t, b) => {
                format!("I({:#?},{})", t, b)
            },
            Self::Output(t, b) => {
                format!("O({:#?},{})", t, b)
            },
            Self::Fake(t, c) => {
                format!("F({:#?},{})", t, c)
            }
        };
        format!("\"{}\"", inner)
    }
}

impl MCycle {
    pub fn new_fake(duration: u8) -> Self {
        MCycle::Fake(TState::T1, duration)
    }
}

impl MCycle {
    /// Test if we are handling a new opcode in the next cycle
    pub fn is_new_opcode(&self) -> bool {
        *self == MCycle::default()
    }
    
    pub fn is_input(&self) -> bool {
        match self {
            Self::Input(_, _) => true,
            _ => false,
        }
    }
    
    pub fn is_output(&self) -> bool {
        match self {
            Self::Output(_, _) => true,
            _ => false,
        }
    }
    
    pub(crate) fn inc_state(&mut self) {
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
            MCycle::Read(TState::T3, nb) => MCycle::Read(TState::T4, *nb),
            MCycle::Read(TState::T4, nb) => panic!("Ambigous"),
            
            MCycle::Write(TState::T1, nb) => MCycle::Write(TState::T2, *nb),
            MCycle::Write(TState::T2, nb) => MCycle::Write(TState::T3, *nb),
            MCycle::Write(TState::T3, nb) => MCycle::Write(TState::T4, *nb),
            MCycle::Write(TState::T4, nb) => panic!("Ambigous"),
            
            MCycle::Output(TState::T1, nb) => MCycle::Output(TState::T2, *nb),
            MCycle::Output(TState::T2, nb) => MCycle::Output(TState::T3, *nb),
            MCycle::Output(TState::T3, nb) => MCycle::Output(TState::T4, *nb),
            MCycle::Output(TState::T4, nb) => panic!("Ambigous"),
            
            MCycle::Input(TState::T1, nb) => MCycle::Input(TState::T2, *nb),
            MCycle::Input(TState::T2, nb) => MCycle::Input(TState::T3, *nb),
            MCycle::Input(TState::T3, nb) => MCycle::Input(TState::T4, *nb),
            MCycle::Input(TState::T4, nb) => panic!("Ambigous"),
            
            MCycle::Fake(TState::T1, nb) => MCycle::Fake(TState::T2, *nb),
            MCycle::Fake(TState::T2, nb) => MCycle::Fake(TState::T3, *nb),
            MCycle::Fake(TState::T3, nb) => MCycle::Fake(TState::T4, *nb),
            MCycle::Fake(TState::T4, nb) => MCycle::Fake(TState::T1, *nb - 1),
            _ => unimplemented!(),
        }
    }
    
    pub fn prefix(&self) -> OpcodePrefix {
        match self {
            MCycle::FetchOpcode(_, _, prefix) => *prefix,
            _ => unreachable!(),
        }
    }
    
    pub fn read_pin_type(&self) -> Option<Z80Pin> {
        match self {
            Self::Input(_, _) => Some(Z80Pin::Neg_IORQ),
            Self::Read(_, _) => Some(Z80Pin::Neg_MREQ),
            _ => unreachable!(),
        }
    }
    
    pub fn write_pin_type(&self) -> Option<Z80Pin> {
        match self {
            Self::Output(_, _) => Some(Z80Pin::Neg_IORQ),
            Self::Write(_, _) => Some(Z80Pin::Neg_MREQ),
            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
mod test {
    use cpclib_asm::preamble::Register16;
    
    use super::*;
    
    #[test]
    fn lookup_checks_hl() {
        // ld hl, xxxx
        assert_eq!(dbg!(&OPCODE_PREFIX[0x21]).nb_additional_opcode_bytes, 2);
    }
    
    #[test]
    fn lookup_checks_call() {
        // call pq
        let call = dbg!(&OPCODE_PREFIX[0xcd]);
        assert_eq!(call.nb_additional_opcode_bytes, 2);
        assert_eq!(call.read_from_pins, false);
        assert_eq!(call.read_address, AddressSource::None);
        assert_eq!(call.nb_bytes_to_read, 0);
        assert_eq!(call.flag_test, None);
        assert_eq!(call.expect_read(), false);
        assert_eq!(
            call.write_address,
            AddressSource::Stack(StackDelta::MinusTwo)
        ); // we want to write on the stack
        assert_eq!(call.write_data, DataSource::Register16(Reg16::Pc)); // we want to write PC
    }
    
    #[test]
    fn lookup_checks_call_conditional() {
        use cpclib_asm::preamble::FlagTest;
        let call = dbg!(&OPCODE_PREFIX[0b11000100]);
        assert_eq!(call.nb_additional_opcode_bytes, 2);
        assert_eq!(
            call.write_address,
            AddressSource::Stack(StackDelta::MinusTwo)
        ); // we want to write on the stack
        assert_eq!(call.write_data, DataSource::Register16(Reg16::Pc)); // we want to write PC
        assert_eq!(call.flag_test, Some(FlagTest::NZ))
    }
    
    #[test]
    fn lookup_checks_ex_sp_hl() {
        let ex = dbg!(&OPCODE_PREFIX[0xe3]);
        assert_eq!(ex.read_address, AddressSource::Stack(StackDelta::Zero));
        assert_eq!(ex.read_data, DataSource::Register16(Reg16::Wz)); // wz expected
        assert_eq!(ex.nb_bytes_to_read, 2);
        assert_eq!(ex.write_address, AddressSource::Stack(StackDelta::Zero));
        assert_eq!(ex.write_data, DataSource::Register16(Reg16::Hl));
        assert_eq!(ex.nb_bytes_to_write, 2);
    }
    
    #[test]
    fn lookup_checks_ret() {
        let ret = dbg!(&OPCODE_PREFIX[0xc9]);
        assert_eq!(ret.nb_additional_opcode_bytes, 0);
        assert_eq!(ret.read_address, AddressSource::Stack(StackDelta::Zero)); // we want to write on the stack
        assert_eq!(ret.nb_bytes_to_read, 2);
        assert_eq!(ret.read_data, DataSource::Register16(Reg16::Pc)); // we want to override PC
    }
    
    #[test]
    fn lookup_checks_ld_sp() {
        let call = dbg!(&OPCODE_PREFIX[0x31]);
        assert_eq!(call.nb_additional_opcode_bytes, 2);
    }
    
    #[test]
    fn lookup_checks_ld_hl_mem() {
        let ld = dbg!(&OPCODE_PREFIX[0x2a]);
        assert_eq!(ld.nb_additional_opcode_bytes, 2);
        assert_eq!(ld.nb_bytes_to_read, 2);
        assert_eq!(ld.read_address, AddressSource::Immediate);
        assert_eq!(ld.read_data, DataDestination::Register16(Reg16::Hl));
    }
    
    #[test]
    fn lookup_checks_ld_sp_mem() {
        let ld = dbg!(&OPCODE_PREFIX_ED[0x7b]);
        assert_eq!(ld.nb_additional_opcode_bytes, 2);
        assert_eq!(ld.nb_bytes_to_read, 2);
        assert_eq!(ld.read_address, AddressSource::Immediate);
        assert_eq!(ld.read_data, DataDestination::Register16(Reg16::Sp));
    }
    
    #[test]
    fn lookup_checks_ld_a_mem_bc() {
        let ld = dbg!(&OPCODE_PREFIX[0x0a]);
        assert_eq!(ld.nb_additional_opcode_bytes, 0);
        assert_eq!(ld.nb_bytes_to_read, 1);
        assert_eq!(ld.read_address, AddressSource::Register16(Reg16::Bc));
        assert_eq!(ld.read_data, DataDestination::Register8(Reg8::A));
    }
    
    #[test]
    fn lookup_check_ldd() {
        let ldd = dbg!(&OPCODE_PREFIX_ED[0xA8]);
        assert_eq!(ldd.nb_bytes_to_read, 1);
        assert_eq!(ldd.nb_bytes_to_write, 1);
        assert_eq!(ldd.read_address, AddressSource::Register16(Reg16::Hl));
        assert_eq!(ldd.write_address, AddressSource::Register16(Reg16::De));
        assert_eq!(ldd.read_data, DataDestination::Pins);
        assert_eq!(ldd.write_data, DataSource::Pins);
    }
    
    #[test]
    fn lookup_check_ldir() {
        let ldd = dbg!(&OPCODE_PREFIX_ED[0xb0]);
        assert_eq!(ldd.nb_bytes_to_read, 1);
        assert_eq!(ldd.nb_bytes_to_write, 1);
        assert_eq!(ldd.read_address, AddressSource::Register16(Reg16::Hl));
        assert_eq!(ldd.write_address, AddressSource::Register16(Reg16::De));
        assert_eq!(ldd.read_data, DataDestination::Pins);
        assert_eq!(ldd.write_data, DataSource::Pins);
    }
    
    
    #[test]
    fn lookup_checks_in() {
        
        let ina = dbg!(&OPCODE_PREFIX_ED[0b01111000]);
        assert_eq!(ina.nb_additional_opcode_bytes, 0);
        assert_eq!(ina.nb_bytes_to_read, 1);
        assert_eq!(ina.read_address, AddressSource::Register16(Reg16::Bc));
        assert_eq!(ina.read_data, DataSource::Register8(Reg8::A)); // we want to write PC
    }
    
    #[test]
    fn lookup_checks_out() {
        // out(c), a
        assert_eq!(
            dbg!(&OPCODE_PREFIX_ED[0x79]).nb_additional_opcode_bytes,
            0 // ed is the already additional
        );
        assert_eq!(dbg!(&OPCODE_PREFIX_ED[0x79]).nb_bytes_to_write, 1);
        assert_eq!(dbg!(&OPCODE_PREFIX_ED[0x79]).write_to_pins, true);
        assert_eq!(
            dbg!(&OPCODE_PREFIX_ED[0x79]).write_address,
            AddressSource::Register16(Reg16::Bc)
        );
        assert_eq!(
            dbg!(&OPCODE_PREFIX_ED[0x79]).write_data,
            DataSource::Register8(Reg8::A)
        );
        
        // out (n), a
        assert_eq!(
            dbg!(&OPCODE_PREFIX[0xd3]).nb_additional_opcode_bytes,
            1 //n
        );
        
        // outd
        let desc = dbg!(&OPCODE_PREFIX_ED[0xab]);
        assert_eq!(desc.nb_bytes_to_write, 1);
        assert_eq!(desc.nb_bytes_to_read, 1);
        assert_eq!(desc.nb_additional_opcode_bytes, 0);
        assert_eq!(desc.read_address, AddressSource::Register16(Reg16::Hl));
        assert_eq!(desc.write_address, AddressSource::Register16(Reg16::Bc));
        assert_eq!(desc.write_to_pins, true);
        assert_eq!(desc.read_from_pins, false);
        assert_eq!(
            desc.write_data,
            DataSource::Pins // we assume data are already on the pins because they have been read one cycle before
        );
    }
    
    #[test]
    fn lookup_checks_push_hl() {
        // call pq
        let call = dbg!(&OPCODE_PREFIX[0xe5]);
        assert_eq!(call.nb_additional_opcode_bytes, 0);
        assert_eq!(
            call.write_address,
            AddressSource::Stack(StackDelta::MinusTwo)
        ); // we want to write on the stack
        assert_eq!(call.write_data, DataSource::Register16(Reg16::Hl));
    }
    
    #[test]
    fn lookup_checks_pop_hl() {
        // call pq
        let pop = dbg!(&OPCODE_PREFIX[0xe1]);
        assert_eq!(pop.nb_additional_opcode_bytes, 0);
        assert_eq!(pop.read_address, AddressSource::Stack(StackDelta::Zero)); // we want to write on the stack
        assert_eq!(pop.read_data, DataSource::Register16(Reg16::Hl));
    }
    
    #[test]
    fn lookup_or_mem_hl() {
        // call pq
        let pop = dbg!(&OPCODE_PREFIX[0xb6]);
        assert_eq!(pop.nb_additional_opcode_bytes, 0);
        assert_eq!(pop.read_address, AddressSource::Register16(Reg16::Hl)); // we want to write on the stack
        assert_eq!(pop.read_data, DataSource::Pins); // ie Z
    }
    
    #[test]
    fn lookup_inc_mem_hl() {
        // call pq
        let pop = dbg!(&OPCODE_PREFIX[0x34]);
        assert_eq!(pop.nb_additional_opcode_bytes, 0);
        assert_eq!(pop.read_address, AddressSource::Register16(Reg16::Hl)); // we want to write on the stack
        assert_eq!(pop.read_data, DataSource::Pins);
        assert_eq!(pop.write_address, AddressSource::Register16(Reg16::Hl)); // we want to write on the stack
        assert_eq!(pop.write_data, DataSource::Pins);
    }
    
    
    
    #[test]
    fn lookup_set_mem_hl() {
        let set = dbg!(&OPCODE_PREFIX_CB[0b11111110]);
        assert_eq!(set.nb_additional_opcode_bytes, 0);
        assert_eq!(set.read_address, AddressSource::Register16(Reg16::Hl)); //
        assert_eq!(set.read_data, DataSource::Pins);
        assert_eq!(set.write_address, AddressSource::Register16(Reg16::Hl)); // we want to write on the stack
        assert_eq!(set.write_data, DataSource::Pins);
    }
}
