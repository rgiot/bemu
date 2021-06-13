use std::{ops::{AddAssign, BitAnd, Deref, Shl, Sub, SubAssign}};

use num::{FromPrimitive, Integer, One, Zero, traits::{WrappingAdd, WrappingSub}};

///! Helpers for bit manipulation

/**
 * Number stored with a maximum number of bits
 */
 #[derive(Debug, Default, PartialEq)]
pub struct BitMax<T, const C: usize>(T);

impl<T, const C: usize> BitMax<T, C> 
where T: Clone
{

	pub fn value(&self) -> T {
		self.0.clone()
	}

	pub fn nb_bits() -> usize {
		C
	}


}


impl<T, const C: usize>  Deref for BitMax<T, C> {
	type Target = T;
	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

impl<T, const C: usize>  From<T> for BitMax<T, C> {

	fn from(val: T) -> Self {
		Self(val)
	}
}

impl<T, const C: usize> BitMax<T, C> 
where T: Clone + One + Shl<Output=T> + FromPrimitive + Sub<Output=T>
{
	pub fn mask() -> T {
		(T::one()<<T::from_usize(Self::nb_bits()).unwrap()) - T::one()
	}
}

impl<T, const C: usize> BitMax<T, C> 
where T: Clone + Zero + One + Shl<Output=T> + FromPrimitive + Sub<Output=T> + PartialEq
{
	pub fn bit(&self, pos: usize) -> bool {
		assert!(pos<C);
		(T::one() << T::from_usize(pos).unwrap()) != T::zero()
	}
}


impl<T, const C: usize> BitMax<T, C> 
where T: Clone + Zero 
{
	pub fn reset(&mut self) {
		self.0 = T::zero();
	}
}


impl<T, const C: usize> BitMax<T, C> 
where T: Clone + WrappingAdd + Integer + FromPrimitive + BitAnd<Output=T>  + One + Shl<Output=T> + FromPrimitive + Sub<Output=T>{
	pub fn inc(&mut self) {
		self.add_assign(T::from_u16(1).unwrap());
	}
}

impl<T, const C: usize> BitMax<T, C> 
where T: Clone + WrappingSub + Integer + FromPrimitive + BitAnd<Output=T>  + One + Shl<Output=T> + FromPrimitive + Sub<Output=T>{
	pub fn dec(&mut self) {
		self.sub_assign(T::from_u16(1).unwrap());
	}

}

impl<T, const C: usize>  AddAssign<T> for BitMax<T, C> 
where T: Clone + WrappingAdd + Integer + FromPrimitive + BitAnd<Output=T>  + One + Shl<Output=T> + FromPrimitive + Sub<Output=T> {
	fn add_assign(&mut self, rhs: T) {
		self.0 = self.0.wrapping_add(&rhs) & Self::mask();
	}
}

impl<T, const C: usize>  SubAssign<T> for BitMax<T, C> 
where T: Clone + WrappingSub + Integer + FromPrimitive + BitAnd<Output=T>  + One + Shl<Output=T> + FromPrimitive + Sub<Output=T> {
	fn sub_assign(&mut self, rhs: T) {
		self.0 = self.0.wrapping_sub(&rhs) & Self::mask();
	}
}

pub type BitMax6 = BitMax<u8, 6>;

#[cfg(test)]
mod test_super {
	use super::*;

	#[test]
	fn test_b6() {
		assert_eq!(
			BitMax6::mask(),
			0b111111
		);

		let mut c = BitMax6::from(0);

		assert_eq!(
			0,
			c.value()
		);

		c.inc(); c.inc();

		assert_eq!(
			2,
			c.value()
		);
	}
}