use num::integer::Integer;
use num::traits::ops::overflowing::OverflowingAdd;
use num::traits::ops::overflowing::OverflowingSub;
use num::One;

use std::fmt;

use super::arithmetic::*;

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
    pub(crate) val: u8,
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
    pub fn new(val: u8) -> Self {
        Register8 { val }
    }
}

impl Register8 {
    pub fn res_bit(&mut self, bit: u8) {
        res_bit(&mut self.val, bit)
    }

    pub fn set_bit(&mut self, bit: u8) {
        set_bit(&mut self.val, bit)
    }

    pub fn and(&mut self, value: u8) {
        and(&mut self.val, value)
    }

    pub fn or(&mut self, value: u8) {
        or(&mut self.val, value)
    }

    pub fn xor(&mut self, value: u8) {
        xor(&mut self.val, value)
    }

    pub fn rlc(&mut self) -> bool {
        rlc(&mut self.val)
    }

    pub fn rr(&mut self, c: bool) -> bool {
        rr(&mut self.val, c)
    }
    pub fn rrc(&mut self) -> bool {
        rrc(&mut self.val)
    }
    pub fn cpl(&mut self) {
        self.val = !self.val;
    }
}

/// Represents a 16 bits register by decomposing it in 2 8 bits registers
#[derive(Copy, Clone, Default, PartialEq)]
pub struct Register16 {
    pub(crate) low: Register8,
    pub(crate) high: Register8,
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

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn rlc() {
        let mut r = Register8 { val: 0b11000000 };
        let c = r.rlc();

        assert!(c);
        assert_eq!(r.value(), 0b10000001);
    }
}
