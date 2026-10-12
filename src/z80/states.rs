
///! This module contains all that can help to manage the variuous states of the CPU
use lazy_static::lazy_static;
//use cpclib_asm::preamble::*;
use super::{Reg16, Reg8, Z80Pin};
use cpclib_asm::preamble::{
    DataAccess, DataAccessElem, Expr, ExprEvaluationExt, ListingElement, Mnemonic,
    Token,
};

/// Provide all the information needed by the CPU to properly execute the instruction
trait OpcodeDecoder {
    /// Check if data has to be obtained from another chip
    fn read_data_from_bus(&self) -> bool;
    /// Check if data has to be sent to  another chip
    fn write_data_to_bus(&self) -> bool;
    /// For instructions reading from memory, get the location of the address
    fn read_address(&self) -> AddressSource;
    /// For instructions writting to memory, get the location of address
    fn write_address(&self) -> AddressSource;
    /// Where to get the data to write
    fn write_data(&self) -> DataSource;
    /// Where to write the data read
    fn read_data(&self) -> DataSource;
    /// The number of bytes to write
    fn nb_bytes_to_write(&self) -> u8;
    /// The number of bytes to read
    fn nb_bytes_to_read(&self) -> u8;
}

impl OpcodeDecoder for Token {
    fn read_data_from_bus(&self) -> bool {
        self.is_input_opcode() 
    }
    
    fn write_data_to_bus(&self) -> bool {
        self.is_output_opcode()
    }
    
    fn read_address(&self) -> AddressSource {
        if self.is_input_opcode() {
            match self.mnemonic_arg2() {
                Some(DataAccess::PortC) | None => {
                    AddressSource::Register16(Reg16::Bc)
                },
                Some(DataAccess::PortN(_)) => {
                    AddressSource::PortN
                },
                _ => unreachable!("{:?}", self),
            }
        } else if self.is_output_opcode() {
            if self.mnemonic_arg1().is_none() {
                // The instructions with no arg are those that read from memory
                AddressSource::Register16(Reg16::Hl)
            } else {
                AddressSource::None
            }
        } else if self.is_retlike_opcode() {
            AddressSource::Stack(StackDelta::Zero)
        } else if matches!(
            self.mnemonic(), 
            Some(Mnemonic::ExMemSp | Mnemonic::Pop)) 
            {
                AddressSource::Stack(StackDelta::Zero)
            } else if matches!(
                self.mnemonic(), 
                Some(
                    Mnemonic::Ldd | Mnemonic::Lddr |
                    Mnemonic::Ldi | Mnemonic::Ldir |
                    Mnemonic::Cpd | Mnemonic::Cpdr |
                    Mnemonic::Cpi | Mnemonic::Cpir |
                    Mnemonic::Rld | Mnemonic::Rrd
                )
            ) 
            {
                AddressSource::Register16(Reg16::Hl)
            } 
            else if matches!(self,
                Token::OpCode(
                    Mnemonic::Rlc | Mnemonic::Rrc |
                    Mnemonic::Rl | Mnemonic::Rr |
                    Mnemonic::Sla | Mnemonic::Sra |
                    Mnemonic::Sl1 | Mnemonic::Srl,
                    Some(DataAccess::IndexRegister16WithIndex(_, _, _)),
                    Some(DataAccess::Register8(_)),
                    None
                )
            ) {
                AddressSource::Indexed(self.mnemonic_arg1().unwrap().get_indexregister16().unwrap().into())
            }
            else if matches!(self,
                Token::OpCode(
                    Mnemonic::Res | Mnemonic::Set,
                    Some(_),
                    Some(DataAccess::IndexRegister16WithIndex(_, _, _)),
                    Some(_)
                )
            ) {
                AddressSource::Indexed(self.mnemonic_arg2().unwrap().get_indexregister16().unwrap().into())
            }
            else if self.mnemonic() == Some(&Mnemonic::Jp) {
                AddressSource::None // JP (HL) is a misleading name
            }
            else {
                // all other opcodes, rules are the same depending on the arguments
                
                match (self.mnemonic_arg1(), self.mnemonic_arg2()) {
                    (_, Some(DataAccess::IndexRegister16WithIndex(reg, _, _))) |(Some(DataAccess::IndexRegister16WithIndex(reg, _, _)), None)
                    => AddressSource::Indexed(reg.into()),
                    
                    (_, Some(DataAccess::MemoryRegister16(reg))) |(Some(DataAccess::MemoryRegister16(reg)), None)
                    => AddressSource::Register16(reg.into()),
                    
                    (_, Some(DataAccess::MemoryIndexRegister16(reg))) |(Some(DataAccess::MemoryIndexRegister16(reg)), None)
                    => AddressSource::IndexRegister16(reg.into()),
                    
                    (_, Some(DataAccess::Memory(_))) |(Some(DataAccess::Memory(_)), None)
                    => AddressSource::Immediate,
                    
                    (_,_) => AddressSource::None
                }
            }
        }
        
        
        fn write_address(&self) -> AddressSource {
            if self.is_output_opcode() {
                match self.mnemonic_arg1() {
                    Some(DataAccess::PortC) | None => AddressSource::Register16(Reg16::Bc),
                    Some(DataAccess::PortN(_)) => AddressSource::PortN,
                    _ => unreachable!("{:?}", self),
                }
            }
            // INI/IND/INIR/INDR store the port byte into (HL) - the
            // read-side symmetric case of OUTI/OUTD reading it from there.
            else if self.is_input_opcode() && self.mnemonic_arg1().is_none() {
                AddressSource::Register16(Reg16::Hl)
            }
            else if let Some(Mnemonic::ExMemSp) = self.mnemonic() {
                AddressSource::Stack(StackDelta::Zero)
            }
            else if let Some(Mnemonic::Call | Mnemonic::Push | Mnemonic::Rst) = self.mnemonic() {
                AddressSource::Stack(StackDelta::MinusTwo)
            }  else if matches!(
                self.mnemonic(), 
                Some(
                    Mnemonic::Ldd | Mnemonic::Lddr |
                    Mnemonic::Ldi | Mnemonic::Ldir
                )
            ) 
            {
                AddressSource::Register16(Reg16::De)
            } else if matches!(
                self.mnemonic(),
                Some(Mnemonic::Rld | Mnemonic::Rrd)
            ) {
                AddressSource::Register16(Reg16::Hl)
            }
            else if let Some(Mnemonic::Bit) = self.mnemonic() {
                AddressSource::None
            }
            else if self.mnemonic() == Some(&Mnemonic::Jp) {
                AddressSource::None // JP (HL) is a misleading name
            }
            else {
                match (self.mnemonic_arg1(), self.mnemonic_arg2()) {
                    (Some(DataAccess::IndexRegister16WithIndex(reg, _, _)), _) | (Some(DataAccess::Expression(_)), Some(DataAccess::IndexRegister16WithIndex(reg, _, _))) 
                    => AddressSource::Indexed(reg.into()),
                    
                    (Some(DataAccess::MemoryRegister16(reg)), _) | (Some(DataAccess::Expression(_)), Some(DataAccess::MemoryRegister16(reg))) 
                    => AddressSource::Register16(reg.into()),
                    
                    (Some(DataAccess::MemoryIndexRegister16(reg)), _) | (Some(DataAccess::Expression(_)), Some(DataAccess::MemoryIndexRegister16(reg))) 
                    => AddressSource::IndexRegister16(reg.into()),
                    
                    (Some(DataAccess::Memory(_)), _) | (Some(DataAccess::Expression(_)), Some(DataAccess::Memory(_))) 
                    => AddressSource::Immediate,
                    
                    (_,_) => AddressSource::None
                }
            }
        }
        
        fn write_data(&self) -> DataSource {
            if self.is_output_opcode() {
                match self.mnemonic_arg2() {
                    // OUT (C),0: the only output with a literal, and no
                    // immediate byte follows the opcode
                    Some(DataAccess::Expression(Expr::Value(0))) => DataSource::Zero,
                    Some(position) => position.into(),
                    None => DataSource::Pins, // the data is already on the pins
                }
            } else if self.is_autocopy_opcode() {
                DataSource::Pins // copy to register is manually handled in the emulator
            }  
            else if let Some(Mnemonic::ExMemSp) = self.mnemonic() {
                let reg = match self.mnemonic_arg1() {
                    Some(DataAccess::Register16(reg)) => reg.into(),
                    Some(DataAccess::IndexRegister16(reg)) => reg.into(),
                    _ => unreachable!()
                };
                DataSource::Register16(reg)
            }
            else if let Some(Mnemonic::Rst) = self.mnemonic() {
                DataSource::Register16(Reg16::Pc)
            }
            else {
                match self.write_address() {
                    AddressSource::None => DataSource::None,
                    AddressSource::Stack(_) => match self.mnemonic() {
                        Some(Mnemonic::Call) => DataSource::Register16(Reg16::Pc),
                        Some(Mnemonic::Push) => {
                            let reg: Reg16 = match self.mnemonic_arg1() {
                                Some(DataAccess::Register16(reg)) => reg.into(),
                                Some(DataAccess::IndexRegister16(reg)) => reg.into(),
                                _ => unreachable!(),
                            };
                            DataSource::Register16(reg)
                        }
                        
                        _ => unreachable!(),
                    },
                    _ => match self.mnemonic_arg2() {
                        Some(stuff) => stuff.into(),
                        None => DataDestination::Pins,
                    },
                }
            }
        }
        
        
        
        fn read_data(&self) -> DataDestination {
            // input like opcodes
            if self.is_input_opcode() {
                match self.mnemonic_arg1() {
                    Some(DataAccess::Register8(reg)) => DataDestination::Register8(reg.into()),
                    // IN F,(C) (undocumented): the byte is only used to
                    // compute flags (see execute_opcode), never stored, so
                    // the generic read-store must not clobber F with it
                    // before execute_opcode runs.
                    Some(DataAccess::Expression(Expr::Value(0))) => DataDestination::Pins,
                    None => DataDestination::Pins,
                    _ => unreachable!()
                }
            } 
            // retlike opcodes
            else if self.is_retlike_opcode() {
                // stack content is stored in PC
                DataDestination::Register16(Reg16::Pc)
            } 
            // ld like opcocdes
            else if let Token::OpCode(
                Mnemonic::Ld,
                Some(DataAccess::Register16(reg)),
                Some(DataAccess::Memory(_)),_
            ) = self
            {
                DataDestination::Register16(reg.into())
            } else if let Token::OpCode(
                Mnemonic::Ld,
                Some(DataAccess::IndexRegister16(reg)),
                Some(DataAccess::Memory(_)),_
            ) = self
            {
                DataDestination::IndexRegister16(reg.into())
            } else if let Token::OpCode(
                Mnemonic::Ld,
                Some(DataAccess::Register8(ref reg)),
                Some(DataAccess::Memory(_) | DataAccess::MemoryRegister16(_) | DataAccess::IndexRegister16WithIndex(_, _, _)),_
            ) = self
            {
                DataDestination::Register8(reg.into())
            } 
            // pop
            else if let Token::OpCode(Mnemonic::Pop, Some(DataAccess::Register16(reg)), _,_) = self {
                DataDestination::Register16(reg.into())
            } 
            else if let Token::OpCode(Mnemonic::Pop, Some(DataAccess::IndexRegister16(reg)), _,_) = self {
                DataDestination::Register16(reg.into())
            } 
            // dec-inc
            else if let Token::OpCode(
                Mnemonic::Inc | Mnemonic::Dec,
                Some(DataAccess::MemoryRegister16(_) | DataAccess::IndexRegister16WithIndex(_, _, _)),
                _,_
            ) = self
            {
                DataDestination::Pins // here we read/write on the data pins
            } 
            // JP / jr
            else if let Token::OpCode( Mnemonic::Jp | Mnemonic::Jr, _, _, _) = self {
                DataDestination::None // notation is erroneous; the value of the register is taken into account, not its address
            }
            // others (mathematicla ?)
            else if let Token::OpCode(_, _, Some(DataAccess::MemoryRegister16(_) | DataAccess::IndexRegister16WithIndex(_, _, _)), _) = self {
                DataDestination::Pins
            } else if let Token::OpCode(_, Some(DataAccess::MemoryRegister16(_)| DataAccess::IndexRegister16WithIndex(_, _, _)), None, _) = self {
                DataDestination::Pins
            }
            // repetition 
            else if matches!(
                self.mnemonic(), 
                Some(
                    Mnemonic::Outi | Mnemonic::Outd
                    | Mnemonic::Otir | Mnemonic::Otdr
                    | Mnemonic::Ldi | Mnemonic::Ldd
                    | Mnemonic::Ldir | Mnemonic::Lddr
                    | Mnemonic::Cpi | Mnemonic::Cpd
                    | Mnemonic::Cpir | Mnemonic::Cpdr
                    | Mnemonic::Rld | Mnemonic::Rrd
                )
            )
            {
                DataDestination::Pins
            } 
            else if let Some(Mnemonic::ExMemSp) = self.mnemonic() {
                DataDestination::Register16(Reg16::Wz)
            }

            else if matches!(self,
                Token::OpCode(
                    Mnemonic::Rlc | Mnemonic::Rrc |
                    Mnemonic::Rl | Mnemonic::Rr |
                    Mnemonic::Sla | Mnemonic::Sra |
                    Mnemonic::Sl1 | Mnemonic::Srl,
                    Some(DataAccess::IndexRegister16WithIndex(_, _, _)),
                    Some(DataAccess::Register8(_)),
                    None
                ))
                {
                    DataDestination::Pins

            } else {
                assert_eq!(AddressSource::None, self.read_address(), "{:?}", self);
                DataDestination::None // TODO implement the other ones
            }
        }
        
        
        fn nb_bytes_to_write(&self) -> u8 {
            if let Some(Mnemonic::ExMemSp) = self.mnemonic() {
                2
            }
            else {match self.write_data() {
                DataSource::None => 0,
                DataSource::Register16(_) => 2,
                DataSource::IndexRegister16(_) => 2,
                DataSource::Register8(_) => 1,
                DataSource::IndexRegister8(_) => 1,
                DataSource::Immediate => 1,           // XXX not sure of that
                DataSource::MemoryRegister16(_) => 1, // XXX not sure of that see if we keep
                DataSource::Pins => 1,                // the byte is already on the pins
                DataSource::Zero => 1,

            }
        }
    }
    
    fn nb_bytes_to_read(&self) -> u8 {
        if let Some(Mnemonic::ExMemSp) = self.mnemonic() {
            2
        }
        else if self.is_output_opcode() {
            if self.mnemonic_arg1().is_none() {
                1
            } else {
                0
            }
        } else {
            match self.read_data() {
                DataSource::None => 0,
                DataSource::Register16(_) => 2,
                DataSource::IndexRegister16(_) => 2,
                DataSource::Register8(_) => 1,
                DataSource::IndexRegister8(_) => 1,
                DataSource::Immediate => 1, // XXX not sure of that
                DataSource::MemoryRegister16(_) => 1, // XXX not sure of that see if we keep
                DataSource::Pins => 1,      // the byte is already on the pins
                DataSource::Zero => 1,

            }
        }
    }
}
    
    
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
        pub(crate) static ref OPCODE_PREFIX_DDCB: [OpcodeInformation; 256] =
        array_init::array_init(|i| OpcodePrefix::DdCb.build_opcode_information(i as u8));
        pub(crate) static ref OPCODE_PREFIX_FDCB: [OpcodeInformation; 256] =
        array_init::array_init(|i| OpcodePrefix::FdCb.build_opcode_information(i as u8));
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
        None ,
        Dd,
        Fd,
        Cb,
        Ed,
        DdCb,
        FdCb
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
        /// Uses A and Immediate value
        PortN
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
    

    impl TState {
        fn inc_state(&self) -> Self {
            match self {
                TState::T1 => TState::T2,
                TState::T2 => TState::T3,
                TState::T3 => TState::T4,
                TState::T4 => panic!("MCycle need to change")
            }
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


        /// Whether the read and write halves of this instruction share the
        /// same address, so the address already on the pins after the read
        /// can be reused for the write instead of recomputing it (true RMW,
        /// e.g. `INC (HL)`, `RLC (HL)`). False for every instruction whose
        /// read and write sides are on different buses or registers - the
        /// LDx family (HL to DE) and the block I/O family (a port to/from
        /// (HL), on the I/O bus for one side and the memory bus for the
        /// other) - even though both still round-trip the byte through the
        /// pins rather than computing anything from it.
        pub fn compute_between_read_write(&self) -> bool {
            self.read_data == DataSource::Pins 
                && self.write_data == DataSource::Pins 
                && !matches!(
                    self.token.mnemonic(),
                    Some(
                        Mnemonic::Ldd | Mnemonic::Lddr
                        | Mnemonic::Ldi | Mnemonic::Ldir
                        | Mnemonic::Ini | Mnemonic::Ind
                        | Mnemonic::Inir | Mnemonic::Indr
                        | Mnemonic::Outi | Mnemonic::Outd
                        | Mnemonic::Otir | Mnemonic::Otdr
                    )
                )
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

        pub fn prefix_len(&self) -> usize {
            match self {
                Self::None => 0,
                Self::Cb | Self::Dd | Self::Ed | Self::Fd => 1,
                Self::DdCb | Self::FdCb => 2
            }
        }


        /// Return true for 4 bytes opcode where penultimate byte is the index
        pub fn is_for_indexed_opcode(&self) -> bool {
            match self {
                OpcodePrefix::DdCb | OpcodePrefix::FdCb => true,
                _ => false
            }
        }
        /// Get the opcode information from the LUT
        pub fn opcode_information(&self, opcode: u8) -> &'static OpcodeInformation {
            let table: &[OpcodeInformation; 256] = match self {
                Self::None => &OPCODE_PREFIX,
                Self::Cb => &OPCODE_PREFIX_CB,
                Self::Dd => &OPCODE_PREFIX_DD,
                Self::Ed => &OPCODE_PREFIX_ED,
                Self::Fd => &OPCODE_PREFIX_FD,
                Self::DdCb => &OPCODE_PREFIX_DDCB,
                Self::FdCb => &OPCODE_PREFIX_FDCB
            };
            &table[opcode as usize]
        }
        
        /// Retreive all the information needed to data collection for the current opcode
        fn build_opcode_information(&self, opcode: u8) -> OpcodeInformation {
            // A DD/FD-prefixed opcode that doesn't touch HL/H/L/(HL)
            // decodes as its unprefixed form - it is not a no-op: the real
            // CPU executes it as such (e.g. `DD 41` is `LD B,C`, with 4
            // wasted T-states for the prefix fetch; zexall's "ld
            // <bcdexya>,<bcdexya>" test checks it), operand bytes included
            // (`DD 01 nnnn` is `LD BC,nnnn`). See `decoder.rs`.
            let token = super::decoder::template(*self, opcode);

            let read_from_pins = token.read_data_from_bus() ;
            let write_to_pins = token.write_data_to_bus();
            let read_address = token.read_address();
            let write_address = token.write_address();
            let write_data: DataSource = token.write_data();
            let read_data = token.read_data();
            let nb_bytes_to_write = token.nb_bytes_to_write();
            
            if read_address != AddressSource::None {
                assert!(
                    read_data != DataSource::None,
                    "{:?} / {:?} expect read_data to be filled",
                    token,
                    read_address
                );
            }
            
            let nb_bytes_to_read = token.nb_bytes_to_read();
            
            let flag_test = match token.mnemonic_arg1() {
                Some(DataAccess::FlagTest(f)) => Some(f.clone()),
                _ => None,
            };

            OpcodeInformation {
                prefix: self.clone(),
                // Warning: for DD CB / FD CB this counts the displacement
                // and the opcode, which follow the prefixes.
                nb_additional_opcode_bytes: super::decoder::nb_operand_bytes(*self, opcode),
                read_address,
                write_address,
                write_data,
                read_data,
                nb_bytes_to_read,
                nb_bytes_to_write,
                read_from_pins,
                write_to_pins,
                token,
                flag_test,
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
        /// The constant 0 (undocumented OUT (C),0, ED 71)
        Zero,
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
                DataAccess::MemoryRegister16(_) | DataAccess::IndexRegister16WithIndex(_, _, _) => DataSource::Pins, // what has bin read is stored in the pins
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
        /// M1 state dedicated to the interruption acknowledge
        InterruptM1(TState),
        /// Read state (state, byte index to read) opciode information is stored by CPU
        Read(TState, u8),
        /// Write in memory
        Write(TState, u8),
        /// Read from a chip
        Input(TState, u8),
        /// Write to a chip
        Output(TState, u8),
        /// Fake state to simulate additional computations
        Fake(TState, u8),
        /// One microsecond of waiting before an I/O cycle (`true`: an output,
        /// else an input). ACCC 4.4.3: the effect of IN r,(C) and INI/IND lands
        /// on the 4th microsecond of the instruction, OUTI/OUTD on the 5th.
        IoWait(TState, bool),
        /// Extra microseconds of the interrupt response (ACCC 27.4: the call to
        /// #38 lasts 5 us, an RST #38 4 us and the M1 plus the two pushes 3),
        /// before the return address is pushed: (T-state, microseconds left)
        InterruptWait(TState, u8)
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
                Self::InterruptM1(t) => {
                    format!("I({:#?})", t)
                }
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
                Self::IoWait(t, out) => {
                    format!("W({:#?},{})", t, out)
                }
                Self::InterruptWait(t, left) => {
                    format!("IW({:#?},{})", t, left)
                }
            };
            format!("\"{}\"", inner)
        }
    }
    
    impl MCycle {
        pub fn new_fake(duration: u8) -> Self {
            MCycle::Fake(TState::T1, duration)
        }

        pub fn new_interruption() -> Self {
            MCycle::InterruptM1(TState::T1)
        }

        pub fn new_write() -> Self {
            MCycle::Write(TState::T1, 0)
        }

        pub fn new_read() -> Self {
            MCycle::Read(TState::T1, 0)
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

        pub fn is_interruption(&self) -> bool {
            match self {
                Self::InterruptM1(_) => true,
                _ => false
            }
        }
        
        pub(crate) fn inc_state(&mut self) {
            *self = match self {
                MCycle::FetchOpcode(TState::T4, nb, prefix) => panic!("Ambigous"),
                MCycle::FetchOpcode(t, nb, prefix) => MCycle::FetchOpcode(t.inc_state(), *nb, *prefix),

                MCycle::Read(TState::T4, nb) => panic!("Ambigous"),
                MCycle::Read(t, nb) => MCycle::Read(t.inc_state(), *nb),
                
                MCycle::Write(TState::T4, nb) => panic!("Ambigous"),
                MCycle::Write(t, nb) => MCycle::Write(t.inc_state(), *nb),

                MCycle::Output(TState::T4, nb) => panic!("Ambigous"),
                MCycle::Output(t, nb) => MCycle::Output(t.inc_state(), *nb),

                MCycle::Input(TState::T4, nb) => panic!("Ambigous"),
                MCycle::Input(t, nb) => MCycle::Input(t.inc_state(), *nb),

                MCycle::IoWait(TState::T4, true) => MCycle::Output(TState::T1, 0),
                MCycle::IoWait(TState::T4, false) => MCycle::Input(TState::T1, 0),
                MCycle::IoWait(t, out) => MCycle::IoWait(t.inc_state(), *out),

                MCycle::InterruptWait(TState::T4, 1) => MCycle::Write(TState::T1, 0),
                MCycle::InterruptWait(TState::T4, left) => MCycle::InterruptWait(TState::T1, *left - 1),
                MCycle::InterruptWait(t, left) => MCycle::InterruptWait(t.inc_state(), *left),

                MCycle::Fake(TState::T4, nb) => MCycle::Fake(TState::T1, *nb - 1),
                MCycle::Fake(t, nb) => MCycle::Fake(t.inc_state(), *nb),

                MCycle::InterruptM1(TState::T4) => MCycle::Write(TState::T1, 0),
                MCycle::InterruptM1(t) => MCycle::InterruptM1(t.inc_state()),
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
                Self::Input(_, _) | Self::InterruptM1(_) => Some(Z80Pin::Neg_IORQ),
                Self::Read(_, _) | Self::FetchOpcode(_, _, _) => Some(Z80Pin::Neg_MREQ),
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
        use std::ops::Add;

        use cpclib_asm::preamble::{FlagTest, IndexRegister16, Register16};
        
        use super::*;
        
        #[test]
        fn lookup_check_hl() {
            // ld hl, xxxx
            assert_eq!(dbg!(&OPCODE_PREFIX[0x21]).nb_additional_opcode_bytes, 2);
        }
       
        #[test]
        fn lookup_check_bit_2_hl() {
            let bit = dbg!(&OPCODE_PREFIX_CB[0b01010110]);

            assert_eq!(bit.nb_additional_opcode_bytes, 0);
            assert_eq!(bit.read_address, AddressSource::Register16(Reg16::Hl));
            assert_eq!(bit.write_address, AddressSource::None);
        } 

        #[test]
        fn lookup_check_call() {
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
        fn lookup_check_cpi() {
            let cpi = dbg!(&OPCODE_PREFIX_ED[0xa1]);
            assert_eq!(cpi.read_address, AddressSource::Register16(Reg16::Hl));
            assert_eq!(cpi.read_data, DataSource::Pins);
            assert_eq!(cpi.nb_bytes_to_read, 1);
        }
        #[test]
        fn lookup_check_call_conditional() {
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
        fn lookup_check_dec_mem_hl() {
            let dec = dbg!(&OPCODE_PREFIX[0x35]);
            assert_eq!(dec.read_address, AddressSource::Register16(Reg16::Hl));
            assert_eq!(dec.read_data, DataSource::Pins);
            assert_eq!(dec.nb_bytes_to_read, 1);
            assert_eq!(dec.write_address, AddressSource::Register16(Reg16::Hl));
            assert_eq!(dec.write_data, DataSource::Pins);
            assert_eq!(dec.nb_bytes_to_write, 1);
        }
        
        #[test]
        fn lookup_check_ex_sp_hl() {
            let ex = dbg!(&OPCODE_PREFIX[0xe3]);
            assert_eq!(ex.read_address, AddressSource::Stack(StackDelta::Zero));
            assert_eq!(ex.read_data, DataSource::Register16(Reg16::Wz)); // wz expected
            assert_eq!(ex.nb_bytes_to_read, 2);
            assert_eq!(ex.write_address, AddressSource::Stack(StackDelta::Zero));
            assert_eq!(ex.write_data, DataSource::Register16(Reg16::Hl));
            assert_eq!(ex.nb_bytes_to_write, 2);
        }
        
        #[test]
        fn lookup_check_ret() {
            let ret = dbg!(&OPCODE_PREFIX[0xc9]);
            assert_eq!(ret.nb_additional_opcode_bytes, 0);
            assert_eq!(ret.read_address, AddressSource::Stack(StackDelta::Zero)); // we want to write on the stack
            assert_eq!(ret.nb_bytes_to_read, 2);
            assert_eq!(ret.read_data, DataSource::Register16(Reg16::Pc)); // we want to override PC
        }
          
        #[test]
        fn lookup_check_reti() {
            let ret = dbg!(&OPCODE_PREFIX_ED[0x4d]);
            assert_eq!(ret.nb_additional_opcode_bytes, 0);
            assert_eq!(ret.read_address, AddressSource::Stack(StackDelta::Zero)); // we want to write on the stack
            assert_eq!(ret.nb_bytes_to_read, 2);
            assert_eq!(ret.read_data, DataSource::Register16(Reg16::Pc)); // we want to override PC
        }      

        #[test]
        fn lookup_check_ret_c() {
            let ret = dbg!(&OPCODE_PREFIX[0b11011000]);
            assert_eq!(ret.nb_additional_opcode_bytes, 0);
            assert_eq!(ret.read_address, AddressSource::Stack(StackDelta::Zero)); // we want to write on the stack
            assert_eq!(ret.nb_bytes_to_read, 2);
            assert_eq!(ret.read_data, DataSource::Register16(Reg16::Pc));
            assert_eq!(ret.flag_test, Some(FlagTest::C));
        }

        #[test]
        fn lookup_halt() {
            let halt = dbg!(&OPCODE_PREFIX[0x76]);
            assert_eq!(halt.nb_additional_opcode_bytes, 0);
            assert_eq!(halt.nb_bytes_to_read, 0);
            assert_eq!(halt.nb_bytes_to_write, 0);
        }

        #[test]
        fn lookup_checks_jr() {
            let jr = dbg!(&OPCODE_PREFIX[0x18]);
            assert_eq!(jr.nb_additional_opcode_bytes, 1);
            assert_eq!(jr.nb_bytes_to_read, 0);
            assert_eq!(jr.nb_bytes_to_write, 0);
        }
        #[test]
        fn lookup_check_ld_sp() {
            let call = dbg!(&OPCODE_PREFIX[0x31]);
            assert_eq!(call.nb_additional_opcode_bytes, 2);
        }
        
        #[test]
        fn lookup_check_ld_hl_mem() {
            let ld = dbg!(&OPCODE_PREFIX[0x2a]);
            assert_eq!(ld.nb_additional_opcode_bytes, 2);
            assert_eq!(ld.nb_bytes_to_read, 2);
            assert_eq!(ld.read_address, AddressSource::Immediate);
            assert_eq!(ld.read_data, DataDestination::Register16(Reg16::Hl));
        }
        
        #[test]
        fn lookup_check_ld_sp_mem() {
            let ld = dbg!(&OPCODE_PREFIX_ED[0x7b]);
            assert_eq!(ld.nb_additional_opcode_bytes, 2);
            assert_eq!(ld.nb_bytes_to_read, 2);
            assert_eq!(ld.read_address, AddressSource::Immediate);
            assert_eq!(ld.read_data, DataDestination::Register16(Reg16::Sp));
        }
        
        #[test]
        fn lookup_check_ld_a_mem_ix_delta() {
            let ld = dbg!(&OPCODE_PREFIX_DD[0x7e]);
            assert_eq!(ld.nb_additional_opcode_bytes, 1);
            assert_eq!(ld.nb_bytes_to_read, 1);
            assert_eq!(ld.read_address, AddressSource::Indexed(Reg16::Ix));
            assert_eq!(ld.read_data, DataDestination::Register8(Reg8::A));
        }

        #[test]
        fn lookup_check_ld_a_mem_bc() {
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
        fn lookup_check_in_a_c() {
            
            let ina = dbg!(&OPCODE_PREFIX_ED[0b01111000]);

            assert_eq!(ina.nb_additional_opcode_bytes, 0);
            assert_eq!(ina.nb_bytes_to_read, 1);
            assert_eq!(ina.read_address, AddressSource::Register16(Reg16::Bc));
            assert_eq!(ina.read_data, DataSource::Register8(Reg8::A)); // we want to write PC
        }

        #[test]
        fn lookup_check_in_a_n() {
            
            let ina = dbg!(&OPCODE_PREFIX[0xdb]);

            assert_eq!(ina.nb_additional_opcode_bytes, 1);
            assert_eq!(ina.nb_bytes_to_read, 1);
            assert_eq!(ina.read_address, AddressSource::PortN);
            assert_eq!(ina.read_data, DataSource::Register8(Reg8::A)); // we want to write PC
        }
        
        

        #[test]
        fn lookup_check_in_0() {
            // IN F,(C) (undocumented): only the flags change, the byte read
            // is never stored anywhere - in particular not in F itself,
            // which execute_opcode still needs at its original value to
            // preserve the carry flag.
            let ina = dbg!(&OPCODE_PREFIX_ED[0b01110000]);
            assert_eq!(ina.nb_additional_opcode_bytes, 0);
            assert_eq!(ina.nb_bytes_to_read, 1);
            assert_eq!(ina.read_address, AddressSource::Register16(Reg16::Bc));
            assert_eq!(ina.read_data, DataSource::Pins);
        }


        #[test]
        fn lookup_check_out() {
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
                dbg!(&OPCODE_PREFIX_ED[0x79]).read_address,
                AddressSource::None
            );
            assert_eq!(
                dbg!(&OPCODE_PREFIX_ED[0x79]).write_data,
                DataSource::Register8(Reg8::A)
            );
            
            // out (n), a
            let out = dbg!(&OPCODE_PREFIX[0xd3]);
            assert_eq!(out.nb_additional_opcode_bytes, 1 );
            assert_eq!(out.nb_bytes_to_write, 1);
            assert_eq!(out.nb_bytes_to_read, 0);
            assert_eq!(out.write_address, AddressSource::PortN);
            
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
        fn lookup_check_push_hl() {
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
        fn lookup_check_rlc_indexed() {
            // Here we have a difficulty because 06 is obtained AFTER having read the two bytes... so extra should be viewed as 0
            let rlc = dbg!(&OPCODE_PREFIX_DDCB[06]);
            assert_eq!(
                2,
                rlc.nb_additional_opcode_bytes
            );
            assert_eq!(
                1,
                rlc.nb_bytes_to_read
            );
            assert_eq!(
                1,
                rlc.nb_bytes_to_write
            );
            assert_eq!(
                rlc.read_data,
                DataDestination::Pins
            );
            assert_eq!(
                rlc.write_data,
                DataDestination::Pins
            );
            assert_eq!(
                rlc.read_address,
                AddressSource::Indexed(Reg16::Ix)
            );
            assert_eq!(
                rlc.write_address,
                AddressSource::Indexed(Reg16::Ix)
            );
        }

        #[test]
        fn lookup_check_rlc_indexed_hidden() {
            // Here we have a difficulty because 06 is obtained AFTER having read the two bytes... so extra should be viewed as 0
            let rlc = dbg!(&OPCODE_PREFIX_DDCB[00]);
            assert_eq!(
                2,
                rlc.nb_additional_opcode_bytes
            );
            assert_eq!(
                1,
                rlc.nb_bytes_to_read
            );
            assert_eq!(
                1,
                rlc.nb_bytes_to_write
            );
            assert_eq!(
                rlc.read_data,
                DataDestination::Pins
            );
            assert_eq!(
                rlc.write_data,
                DataDestination::Pins
            );
            assert_eq!(
                rlc.read_address,
                AddressSource::Indexed(Reg16::Ix)
            );
            assert_eq!(
                rlc.write_address,
                AddressSource::Indexed(Reg16::Ix)
            );
        }



        #[test]
        fn lookup_check_rst() {
            // call pq
            let rst = dbg!(&OPCODE_PREFIX[0b11001111]);
            assert_eq!(rst.nb_additional_opcode_bytes, 0);
            assert_eq!(
                rst.write_address,
                AddressSource::Stack(StackDelta::MinusTwo)
            ); // we want to write on the stack
            assert_eq!(rst.write_data, DataSource::Register16(Reg16::Pc));
        }

        #[test]
        fn lookup_check_pop_hl() {
            // call pq
            let pop = dbg!(&OPCODE_PREFIX[0xe1]);
            assert_eq!(pop.nb_additional_opcode_bytes, 0);
            assert_eq!(pop.read_address, AddressSource::Stack(StackDelta::Zero)); // we want to write on the stack
            assert_eq!(pop.read_data, DataSource::Register16(Reg16::Hl));
        }
  
        #[test]
        fn lookup_check_rld() {
            // call pq
            let rld = dbg!(&OPCODE_PREFIX_ED[0x6f]);
            assert_eq!(rld.nb_additional_opcode_bytes, 0);
            assert_eq!(rld.write_address,AddressSource::Register16(Reg16::Hl));
            assert_eq!(rld.read_address,AddressSource::Register16(Reg16::Hl));
            assert_eq!(rld.write_data, DataSource::Pins);
            assert_eq!(rld.read_data, DataSource::Pins);
        }
            


        #[test]
        fn lookup_check_srl_mem_hl() {
            let srl = dbg!(&OPCODE_PREFIX_CB[0x3e]);
            assert_eq!(srl.nb_bytes_to_read, 1);
            assert_eq!(srl.nb_bytes_to_write, 1);
            assert_eq!(srl.write_data, DataSource::Pins);
            assert_eq!(srl.read_data, DataSource::Pins);
            assert_eq!(srl.read_address, AddressSource::Register16(Reg16::Hl));
            assert_eq!(srl.write_address, AddressSource::Register16(Reg16::Hl));
            assert!(srl.compute_between_read_write());
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
