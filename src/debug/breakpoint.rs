use crate::motherboard::Motherboard;

#[derive(Debug)]
pub enum BreakPoint {
	/// /Very weak-based breakpoint that considers an address whatever is the memory configuration
	Address(u16)
	// TODO add conditional breakpoint
}



impl BreakPoint {
	/// Check if the breakpoint is fired
	pub fn is_fired(&self, motherboard: &Motherboard) -> bool {
		match self {
			BreakPoint::Address(addr) => {
				*addr == motherboard.pc()
			},
			_ => unimplemented!("{:?}", self)
		}
	}
}