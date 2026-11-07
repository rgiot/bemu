use amsrust::{debug::Monitor, motherboard::MotherboardBuilder};

#[test]
fn test_boot() {
    let mut motherboard = MotherboardBuilder::default().build();
    for _ in 0..10000 {
        Monitor::monitor(&motherboard);
        println!();
        motherboard.execute_next_opcode();
    }
}
