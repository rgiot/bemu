/// Identifiers for any 16 bits registers
#[derive(Debug, Copy, Clone, PartialEq)]
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
#[derive(Debug, Copy, Clone, PartialEq)]
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
impl Into<Reg8> for cpclib_asm::preamble::Register8 {
    fn into(self) -> Reg8 {
        (&self).into()
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
impl Into<Reg8> for cpclib_asm::preamble::IndexRegister8 {
    fn into(self) -> Reg8 {
        (&self).into()
    }
}

impl Reg8 {
    pub fn is_low(&self) -> bool {
        !self.is_high()
    }

    pub fn is_high(&self) -> bool {
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

    pub fn full(&self) -> Reg16 {
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
    pub fn high(&self) -> Reg8 {
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

    pub fn low(&self) -> Reg8 {
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

impl Into<Reg16> for cpclib_asm::preamble::Register16 {
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

impl Into<Reg16> for cpclib_asm::preamble::IndexRegister16 {
    fn into(self) -> Reg16 {
        match self {
            cpclib_asm::preamble::IndexRegister16::Ix => Reg16::Ix,
            cpclib_asm::preamble::IndexRegister16::Iy => Reg16::Iy,
        }
    }
}
