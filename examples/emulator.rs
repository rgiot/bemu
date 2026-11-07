use amsrust::{debug::Monitor, motherboard::MotherboardBuilder};

fn main() {
    let mut motherboard = MotherboardBuilder::default().build();
    loop {
        Monitor::monitor(&motherboard);
        motherboard.execute_next_opcode();
    }
}
