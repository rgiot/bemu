use cpclib_asm::preamble::*;

use super::DisassembledInstruction;

/// This is an ugly name to indicate the property of the instruction to use step into like debug
pub trait StepInto {
    fn can_use_step_into(&self) -> bool;
}

impl StepInto for Token {
    fn can_use_step_into(&self) -> bool {
        match self {
            Self::OpCode(
                Mnemonic::Rst
                | Mnemonic::Call
                | Mnemonic::Ldir
                | Mnemonic::Lddr
                | Mnemonic::Otdr
                | Mnemonic::Otir
                | Mnemonic::Halt,
                _,
                _,
                _,
            ) => true,
            _ => true,
        }
    }
}

impl StepInto for DisassembledInstruction {
    fn can_use_step_into(&self) -> bool {
        self.token.can_use_step_into()
    }
}
