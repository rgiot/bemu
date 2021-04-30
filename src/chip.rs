use num::cast::ToPrimitive;

/// Each chip of the machine must implement this trait
pub trait Chip {
    type PinType: Into<u64> + Clone + Copy + 'static;

    /// One clock tick appens; the chip has to make its stuff
    /// One tick correspond to activation/deactivation of the pin
    fn tick(&mut self) {
        if self.must_execute_tick() {
            self.execute_tick();
        }
    }

    /// Check of the chip must execute its processing cycle
    fn must_execute_tick(&self) -> bool;

    /// Handle the appropriate treatment for the given tick
    fn execute_tick(&mut self);

    /// Set the boolean value to the selected pin of the chip
    /// panic if the pin does not exists
    /// panic if the pin is read only
    fn set_pin(&mut self, pin: Self::PinType, value: bool) {
        let pin: u64 = pin.into();
        use std::ops::Not;
        assert_eq!(pin.count_ones(), 1);

        let buffer = self.pins_buffer_mut();
        *buffer &= pin.not(); // Keep ALL other bits
        if value {
            *buffer |= pin; // Set the bit activated
        }
    }

    /// Get the boolean value of the selected pin of the chip
    /// panic if the pin does not exists
    fn get_pin(&self, pin: Self::PinType) -> bool {
        let pin: u64 = pin.into();

        use std::ops::Not;
        assert_eq!(pin.count_ones(), 1);
        let buffer = self.pins_buffer();
        0 != (*buffer & pin)
    }

    fn pins_buffer_mut(&mut self) -> &mut u64;
    fn pins_buffer(&self) -> &u64;

    /// Reset pins and counter
    fn reset(&mut self);

    /// Set the provided value in the provided list of pins. Pin index corresponds to the bit number
    /// panic if the number is larger
    fn set_value_in_pins<V>(&mut self, mut value: V, pins: &[Self::PinType])
    where
        V: num::Integer + num::cast::ToPrimitive + num::cast::FromPrimitive,
    {
        for pin in pins.iter() {
            self.set_pin(pin.clone(), value.is_odd());
            value = value.div_floor(&V::from_u8(2).unwrap());
        }
        assert!(value.to_usize().unwrap() == 0);
    }

    fn get_value_from_pins(&self, pins: &[Self::PinType]) -> u16 {
        assert!(pins.len() <= 16);

        let mut res = 0;
        for pin in pins.iter().rev() {
            res <<= 1;
            if self.get_pin(*pin) {
                res += 1;
            }
        }

        res
    }

    fn get_8bit_value_from_pins(&self, pins: &[Self::PinType]) -> u8 {
        return (self.get_value_from_pins(pins) & 0xff) as u8;
    }
}

pub trait DataChip: Chip {
    /// Returns the ordered list of pins that contains the data
    fn pins_data() -> &'static [Self::PinType; 8];
}

pub trait AddressChip: Chip {
    /// Returns the ordered list of pins that contains the address
    fn pins_address() -> &'static [Self::PinType; 16];
}

pub trait ReadDataChip: DataChip {
    /// Get the value from the appropriate pins
    fn get_data_from_pins(&self) -> u8 {
        self.get_8bit_value_from_pins(Self::pins_data())
    }
}

pub trait ReadAddressChip: AddressChip {
    /// Get the provided address from the appropriate pins
    fn get_address_from_pins(&self) -> u16 {
        self.get_value_from_pins(Self::pins_address())
    }
}

pub trait WriteDataChip: DataChip {
    /// Set the value on the appropriate pins
    fn set_data_on_pins(&mut self, addr: u8) {
        self.set_value_in_pins(addr, Self::pins_data());
    }
}

pub trait WriteAddressChip: AddressChip {
    /// Set the provided address on the appropriate pins
    fn set_address_on_pins(&mut self, addr: u16) {
        self.set_value_in_pins(addr, Self::pins_address());
    }
}

/// Synchronize the pins of the first chip to the pins of the second chip
pub fn sync_pins<C1: Chip, C2: Chip>(
    chip1: &C1,
    pins1: &[C1::PinType],
    chip2: &mut C2,
    pins2: &[C2::PinType],
) {
    assert_eq!(pins1.len(), pins2.len());

    use itertools::zip;
    for (pin1, pin2) in zip(pins1.iter(), pins2.iter()) {
        chip2.set_pin(*pin2, chip1.get_pin(*pin1));
    }
}
