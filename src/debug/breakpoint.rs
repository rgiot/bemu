use crate::motherboard::Motherboard;

#[derive(Debug, PartialEq)]
pub enum BreakPoint {
    /// /Very weak-based breakpoint that considers an address whatever is the memory configuration
    Address(u16), // TODO add conditional breakpoint
}

impl std::fmt::Display for BreakPoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Address(addr) => write!(f, "z80.pc={:4X}", addr),
        }
    }
}

impl BreakPoint {
    /// Check if the breakpoint is fired
    pub fn is_fired(&self, motherboard: &Motherboard) -> bool {
        match self {
            BreakPoint::Address(addr) => *addr == motherboard.pc(),
        }
    }
}
