//! The arithmetic and logic of the Z80, as pure functions on values and
//! flag bytes: every function returns the result and the new F register,
//! undocumented X/Y bits included. Nothing here touches the CPU, so the
//! flag rules are tested (and read) in one place.
//!
//! Checked against a reference core by `tests/z80_reference.rs`.

pub const S: u8 = 0x80;
pub const Z: u8 = 0x40;
pub const Y: u8 = 0x20;
pub const H: u8 = 0x10;
pub const X: u8 = 0x08;
pub const PV: u8 = 0x04;
pub const N: u8 = 0x02;
pub const C: u8 = 0x01;

/// The flags every 8-bit result sets the same way: sign, zero and the two
/// undocumented bits copied from the result.
#[inline]
const fn szxy(result: u8) -> u8 {
    (result & (S | Y | X)) | if result == 0 { Z } else { 0 }
}

#[inline]
const fn parity(value: u8) -> u8 {
    if value.count_ones() % 2 == 0 { PV } else { 0 }
}

/// Flags of the logical operation `result` (AND sets H, OR/XOR clear it).
#[inline]
pub const fn logic_flags(result: u8, half_carry: bool) -> u8 {
    szxy(result) | parity(result) | if half_carry { H } else { 0 }
}

/// `a + b + carry`: ADD and ADC.
#[inline]
pub const fn add8(a: u8, b: u8, carry: bool) -> (u8, u8) {
    let sum = a as u16 + b as u16 + carry as u16;
    let result = sum as u8;
    let mut flags = szxy(result);
    if (a ^ b ^ result) & 0x10 != 0 {
        flags |= H;
    }
    // overflow: operands of the same sign, result of the other
    if (a ^ result) & (b ^ result) & 0x80 != 0 {
        flags |= PV;
    }
    if sum > 0xff {
        flags |= C;
    }
    (result, flags)
}

/// `a - b - carry`: SUB, SBC, and CP (whose caller takes X/Y from `b`).
#[inline]
pub const fn sub8(a: u8, b: u8, carry: bool) -> (u8, u8) {
    let difference = (a as u16).wrapping_sub(b as u16).wrapping_sub(carry as u16);
    let result = difference as u8;
    let mut flags = szxy(result) | N;
    if (a ^ b ^ result) & 0x10 != 0 {
        flags |= H;
    }
    // overflow: operands of different signs, result of the sign of b
    if (a ^ b) & (a ^ result) & 0x80 != 0 {
        flags |= PV;
    }
    if difference > 0xff {
        flags |= C;
    }
    (result, flags)
}

/// CP: SUB whose result is dropped and whose X/Y come from the operand.
#[inline]
pub const fn cp8(a: u8, b: u8) -> u8 {
    let (_, flags) = sub8(a, b, false);
    (flags & !(X | Y)) | (b & (X | Y))
}

/// INC r / INC (HL): carry is untouched, P/V is the overflow.
#[inline]
pub const fn inc8(value: u8, flags: u8) -> (u8, u8) {
    let result = value.wrapping_add(1);
    let mut new = szxy(result) | (flags & C);
    if value & 0x0f == 0x0f {
        new |= H;
    }
    if value == 0x7f {
        new |= PV;
    }
    (result, new)
}

/// DEC r / DEC (HL).
#[inline]
pub const fn dec8(value: u8, flags: u8) -> (u8, u8) {
    let result = value.wrapping_sub(1);
    let mut new = szxy(result) | N | (flags & C);
    if value & 0x0f == 0 {
        new |= H;
    }
    if value == 0x80 {
        new |= PV;
    }
    (result, new)
}

/// ADD HL,rr (and ADD IX,rr): S, Z and P/V are untouched, H is the carry
/// out of bit 11, X/Y come from the high byte of the result.
#[inline]
pub const fn add16(left: u16, right: u16, flags: u8) -> (u16, u8) {
    let sum = left as u32 + right as u32;
    let result = sum as u16;
    let mut new = flags & (S | Z | PV);
    new |= ((result >> 8) as u8) & (Y | X);
    if (left ^ right ^ result) & 0x1000 != 0 {
        new |= H;
    }
    if sum > 0xffff {
        new |= C;
    }
    (result, new)
}

/// ADC HL,rr.
#[inline]
pub const fn adc16(left: u16, right: u16, carry: bool) -> (u16, u8) {
    let sum = left as u32 + right as u32 + carry as u32;
    let result = sum as u16;
    let mut flags = ((result >> 8) as u8) & (S | Y | X);
    if result == 0 {
        flags |= Z;
    }
    if (left ^ right ^ result) & 0x1000 != 0 {
        flags |= H;
    }
    if (left ^ result) & (right ^ result) & 0x8000 != 0 {
        flags |= PV;
    }
    if sum > 0xffff {
        flags |= C;
    }
    (result, flags)
}

/// SBC HL,rr.
#[inline]
pub const fn sbc16(left: u16, right: u16, carry: bool) -> (u16, u8) {
    let difference = (left as u32).wrapping_sub(right as u32).wrapping_sub(carry as u32);
    let result = difference as u16;
    let mut flags = (((result >> 8) as u8) & (S | Y | X)) | N;
    if result == 0 {
        flags |= Z;
    }
    if (left ^ right ^ result) & 0x1000 != 0 {
        flags |= H;
    }
    if (left ^ right) & (left ^ result) & 0x8000 != 0 {
        flags |= PV;
    }
    if difference > 0xffff {
        flags |= C;
    }
    (result, flags)
}

/// CPL.
#[inline]
pub const fn cpl(a: u8, flags: u8) -> (u8, u8) {
    let result = !a;
    (result, (flags & (S | Z | PV | C)) | H | N | (result & (Y | X)))
}

/// SCF.
#[inline]
pub const fn scf(a: u8, flags: u8) -> u8 {
    (flags & (S | Z | PV)) | C | (a & (Y | X))
}

/// CCF: H receives the old carry.
#[inline]
pub const fn ccf(a: u8, flags: u8) -> u8 {
    (flags & (S | Z | PV)) | (if flags & C != 0 { H } else { C }) | (a & (Y | X))
}

/// DAA.
#[inline]
pub const fn daa(a: u8, flags: u8) -> (u8, u8) {
    let subtraction = flags & N != 0;
    let carry = flags & C != 0;
    let half = flags & H != 0;

    // Adjust each nibble that is not a decimal digit (or that carried); the
    // high one also sets the carry.
    let mut correction = 0;
    if half || a & 0x0f > 9 {
        correction |= 0x06;
    }
    if carry || a > 0x99 {
        correction |= 0x60;
    }
    let result = if subtraction { a.wrapping_sub(correction) } else { a.wrapping_add(correction) };

    let mut new = szxy(result) | parity(result) | (flags & N);
    if carry || a > 0x99 {
        new |= C;
    }
    let half_carry = if subtraction { half && a & 0x0f < 6 } else { a & 0x0f > 9 };
    if half_carry {
        new |= H;
    }
    (result, new)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn add_sets_half_carry_and_overflow() {
        assert_eq!(add8(0x7f, 0x01, false), (0x80, S | H | PV));
        assert_eq!(add8(0xff, 0x01, false), (0x00, Z | H | C));
        assert_eq!(add8(0x0f, 0x00, true), (0x10, H));
        // the carry in takes part in the half carry
        assert_eq!(add8(0x0f, 0x0f, true).1 & H, H);
    }

    #[test]
    fn sub_borrows_and_overflows() {
        assert_eq!(sub8(0x00, 0x01, false), (0xff, S | Y | X | H | N | C));
        assert_eq!(sub8(0x80, 0x01, false), (0x7f, Y | X | H | PV | N));
        assert_eq!(sub8(0x10, 0x00, true).1 & H, H);
    }

    #[test]
    fn compare_takes_x_and_y_from_its_operand() {
        // A = B: zero, N; X/Y from B (0x28)
        assert_eq!(cp8(0x28, 0x28), Z | N | X | Y);
        assert_eq!(cp8(0x28, 0x00) & (X | Y), 0);
    }

    #[test]
    fn inc_and_dec_keep_the_carry() {
        assert_eq!(inc8(0x7f, C), (0x80, S | H | PV | C));
        assert_eq!(dec8(0x80, 0), (0x7f, Y | X | H | PV | N));
        assert_eq!(dec8(0x01, C), (0x00, Z | N | C));
    }

    #[test]
    fn sixteen_bit_addition_flags() {
        // ADD keeps S/Z/PV and reads the half carry at bit 11
        assert_eq!(add16(0x0fff, 0x0001, S | Z | PV), (0x1000, S | Z | PV | H));
        assert_eq!(add16(0xffff, 0x0001, 0), (0x0000, H | C));
        // ADC/SBC compute everything
        assert_eq!(adc16(0x7fff, 0x0000, true), (0x8000, S | H | PV));
        assert_eq!(sbc16(0x0000, 0x0000, true), (0xffff, S | Y | X | H | N | C));
    }

    #[test]
    fn daa_adjusts_bcd() {
        // 0x15 + 0x27 = 0x3c -> 0x42
        let (sum, flags) = add8(0x15, 0x27, false);
        assert_eq!(daa(sum, flags).0, 0x42);
        // 0x99 + 0x01 = 0x9a -> 0x00 with carry
        let (sum, flags) = add8(0x99, 0x01, false);
        let (result, flags) = daa(sum, flags);
        assert_eq!(result, 0x00);
        assert_eq!(flags & (C | Z), C | Z);
    }

    #[test]
    fn flag_only_instructions() {
        assert_eq!(cpl(0x00, 0), (0xff, H | N | Y | X));
        assert_eq!(scf(0x00, S | Z | PV | H | N), S | Z | PV | C);
        assert_eq!(ccf(0x00, C), H);
        assert_eq!(ccf(0x00, 0), C);
    }
}
