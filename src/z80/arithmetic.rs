pub fn res_bit(byte: &mut u8, bit: u8) {
    *byte = *byte & (!(1 << bit));
}

pub fn set_bit(byte: &mut u8, bit: u8) {
    *byte = *byte | (1 << bit);
}

pub fn and(byte: &mut u8, value: u8) {
    *byte = *byte & value;
}

pub fn or(byte: &mut u8, value: u8) {
    *byte = *byte | value;
}

pub fn xor(byte: &mut u8, value: u8) {
    *byte = *byte ^ value;
}

pub fn rla(byte: &mut u8, c: bool) -> bool  {
    let bit7 = *byte & 1 << 7 != 0;
    *byte = *byte << 1;
    if c {
        *byte |= 1;
    }
    bit7
}

pub fn rlc(byte: &mut u8) -> bool {
    let bit7 = *byte & 1 << 7 != 0;
    *byte = *byte << 1;
    if bit7 {
        set_bit(byte, 0);
    }
    bit7
}

pub fn rr(byte: &mut u8, c: bool) -> bool {
    let bit0 = *byte & 1 << 0 != 0;
    *byte = *byte >> 1;
    if c {
        set_bit(byte, 7);
    }
    bit0
}

pub fn rrc(byte: &mut u8) -> bool {
    let bit0 = *byte & 1  != 0;
    *byte = *byte >> 1;
    if bit0 {
        set_bit(byte, 7);
    }
    bit0
}
pub fn sl1(byte: &mut u8) -> bool {
    let bit7 = *byte & 1 << 7 != 0;
    *byte = *byte << 1;
    *byte |= 1;
    bit7
}

pub fn sla(byte: &mut u8) -> bool {
    let bit7 = *byte & 1 << 7 != 0;
    *byte = *byte << 1;
    bit7
}



pub fn srl(byte: &mut u8) -> bool {
    let bit0 = *byte & 1 << 0 != 0;
    *byte = *byte >> 1;
    bit0
}

pub fn sra(byte: &mut u8) -> bool {
    let bit0 = *byte & 1 << 0 != 0;
    let bit7 = *byte & 1 << 7;

    *byte = *byte >> 1;
    *byte = *byte | bit7;
    bit0
}