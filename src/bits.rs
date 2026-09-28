pub use std::ops::{AddAssign, BitAnd, BitAndAssign, Deref, DerefMut, Shl, Shr, Sub, SubAssign};

use num::{
    traits::{WrappingAdd, WrappingSub},
    FromPrimitive, Integer, One, Zero,
};

///! Helpers for bit manipulation

/**
 * Number stored with a maximum number of bits
 */
#[derive(Debug, Default, PartialEq, PartialOrd)]
pub struct BitMax<T, const C: usize>(T);

impl<T, const C: usize> BitMax<T, C>
where
    T: Clone,
{
    pub fn value(&self) -> T {
        self.0.clone()
    }

    pub fn nb_bits() -> usize {
        C
    }
}

impl<T, const C: usize> Deref for BitMax<T, C> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T, const C: usize> DerefMut for BitMax<T, C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T, const C: usize> From<T> for BitMax<T, C> {
    fn from(val: T) -> Self {
        Self(val)
    }
}

impl<T, const C: usize> BitMax<T, C>
where
    T: Clone + One + Shl<Output = T> + FromPrimitive + Sub<Output = T>,
{
    pub fn mask() -> T {
        (T::one() << T::from_usize(Self::nb_bits()).unwrap()) - T::one()
    }
}

impl<T, const C: usize> BitMax<T, C>
where
    T: Clone
        + Zero
        + One
        + Shl<Output = T>
        + Shr<Output = T>
        + BitAnd<Output = T>
        + FromPrimitive
        + Sub<Output = T>
        + PartialEq,
{
    /// Whether bit `pos` of the *stored value* is set.
    ///
    /// This used to test `(1 << pos) != 0` alone - a tautology for any
    /// `pos < C` that never looked at `self.0` at all, so it always
    /// returned `true` regardless of what the counter actually held. The
    /// only real caller (`GateArray`'s R52 interrupt counter, ACCC §27.3.2)
    /// was silently getting a constant `true` for "has R52 reached 32
    /// yet?", every single time.
    pub fn bit(&self, pos: usize) -> bool {
        assert!(pos < C);
        (self.0.clone() >> T::from_usize(pos).unwrap()) & T::one() != T::zero()
    }
}

impl<T, const C: usize> BitMax<T, C>
where
    T: Clone + Zero,
{
    pub fn reset(&mut self) {
        self.0 = T::zero();
    }
}

impl<T, const C: usize> BitMax<T, C>
where
    T: Clone
        + WrappingAdd
        + Integer
        + FromPrimitive
        + BitAnd<Output = T>
        + One
        + Shl<Output = T>
        + FromPrimitive
        + Sub<Output = T>,
{
    pub fn inc(&mut self) {
        self.add_assign(T::from_u16(1).unwrap());
    }
}

impl<T, const C: usize> BitMax<T, C>
where
    T: Clone
        + WrappingSub
        + Integer
        + FromPrimitive
        + BitAnd<Output = T>
        + One
        + Shl<Output = T>
        + FromPrimitive
        + Sub<Output = T>,
{
    pub fn dec(&mut self) {
        self.sub_assign(T::from_u16(1).unwrap());
    }
}

impl<T, const C: usize> AddAssign<T> for BitMax<T, C>
where
    T: Clone
        + WrappingAdd
        + Integer
        + FromPrimitive
        + BitAnd<Output = T>
        + One
        + Shl<Output = T>
        + FromPrimitive
        + Sub<Output = T>,
{
    fn add_assign(&mut self, rhs: T) {
        self.0 = self.0.wrapping_add(&rhs) & Self::mask();
    }
}

impl<T, const C: usize> SubAssign<T> for BitMax<T, C>
where
    T: Clone
        + WrappingSub
        + Integer
        + FromPrimitive
        + BitAnd<Output = T>
        + One
        + Shl<Output = T>
        + FromPrimitive
        + Sub<Output = T>,
{
    fn sub_assign(&mut self, rhs: T) {
        self.0 = self.0.wrapping_sub(&rhs) & Self::mask();
    }
}


impl<T, const C: usize> PartialEq<T> for BitMax<T, C> 
where BitMax<T, C>:From<T> ,
T: PartialEq + Clone
{
    fn eq(&self, other: &T) -> bool {
        self.value() == *other    
    }
}


impl<T, const C: usize> PartialOrd<T> for BitMax<T, C> 
where BitMax<T, C>:From<T> ,
T: PartialOrd + Clone {
    fn partial_cmp(&self, other: &T) -> Option<std::cmp::Ordering> {
        self.value().partial_cmp(other)
    }
}



pub type Bit8Max5 = BitMax<u8, 5>;
pub type Bit8Max6 = BitMax<u8, 6>;
pub type Bit8Max7 = BitMax<u8, 7>;

pub enum Bit8MaxVarying {
    Bit8Max6(Bit8Max6),
    Bit8Max7(Bit8Max7)
}

#[cfg(test)]
mod test_super {
    use super::*;

    #[test]
    fn test_b6() {
        assert_eq!(Bit8Max6::mask(), 0b111111);

        let mut c = Bit8Max6::from(0);

        assert_eq!(0, c.value());

        c.inc();
        c.inc();

        assert_eq!(2, c.value());
    }
}
