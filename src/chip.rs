use num::cast::ToPrimitive;
use std::fmt;

/// Previous and current state to be able to compute the front and decrease

#[derive(Default, Copy, Clone, PartialEq)]
pub struct PinsBuffer(u64, u64);

impl fmt::Debug for PinsBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PinsBuffer")
            .field("prev.", &format!("{:b}", &self.0))
            .field("curr.", &format!("{:b}", &self.1))
            .finish()
    }
}

impl From<(u64, u64)> for PinsBuffer {
    fn from(from: (u64, u64)) -> Self {
        Self(from.0, from.1)
    }
}

/// Each chip of the machine must implement this trait
pub trait Chip {
    type PinType: Into<u64> + Clone + Copy + 'static;

    /// One clock tick appens; the chip has to make its stuff
    /// One tick correspond to activation/deactivation of the pin
    fn tick(&mut self) {
        self.pre_tick();
        if self.must_execute_tick() {
            self.execute_tick();
        }
    }

    /// Is executed at each clock tick even if must_execute_tick returns false.
    /// Mainly added for the Gate Array to make all its stuff not releated to PENR/INKR/MMR
    fn pre_tick(&mut self) {}

    /// Check of the chip must execute its processing cycle
    fn must_execute_tick(&self) -> bool;

    /// Handle the appropriate treatment for the given tick
    fn execute_tick(&mut self);

    /// The buffer that encodes the current state of the pin (previous, current) (writeable)
    fn pins_buffer_mut(&mut self) -> &mut PinsBuffer;
    /// The buffer that encodes the current state of the pin (previous, current) (readable)
    fn pins_buffer(&self) -> &PinsBuffer;

    /// Reset pins and counter
    fn reset(&mut self);

    /// Set the boolean value to the selected pin of the chip
    /// panic if the pin does not exists
    /// panic if the pin is read only
    fn set_pin(&mut self, pin: Self::PinType, value: bool) {
        let pin: u64 = pin.into();
        use std::ops::Not;
        // Every `PinType` enum variant is defined as a single-bit value by
        // construction (`1 << n`) - this is a guard against a future enum
        // gaining a non-single-bit variant by mistake, not something that
        // needs re-checking on every one of the millions of pin accesses a
        // single Z80 instruction makes. `debug_assert` keeps the check for
        // tests/debug builds while not paying for it in `--release`, where
        // this was a real (non-debug) `assert!` - see the architecture/perf
        // review in [[project_cpu_tick_performance]].
        debug_assert_eq!(pin.count_ones(), 1);

        let ref mut buffer = self.pins_buffer_mut().1;
        *buffer &= pin.not(); // Keep ALL other bits
        if value {
            *buffer |= pin; // Set the bit activated
        }
    }

    fn off(&mut self, pin: Self::PinType) {
        self.set_pin(pin, false);
    }
    fn on(&mut self, pin: Self::PinType) {
        self.set_pin(pin, true);
    }

    /// Get the boolean value of the selected pin of the chip
    /// panic if the pin does not exists
    fn get_pin(&self, pin: Self::PinType) -> bool {
        let pin: u64 = pin.into();
        use std::ops::Not;
        // See `set_pin`'s comment just above - same reasoning.
        debug_assert_eq!(pin.count_ones(), 1);

        let ref current = self.pins_buffer().1;
        0 != (current & pin)
    }

    /// Check if the pin is in a up front state
    fn is_upfront(&self, pin: Self::PinType) -> bool {
        let pin: u64 = pin.into();
        use std::ops::Not;
        // See `set_pin`'s comment - same reasoning. `GateArray::
        // must_execute_tick()` calls this every GA tick (every
        // `Motherboard::tick()`), so this was on the hottest path too.
        debug_assert_eq!(pin.count_ones(), 1);

        let &PinsBuffer(ref previous, ref current) = self.pins_buffer();

        let current = 0 != (current & pin);
        let previous = 0 != (previous & pin);

        previous == false && current == true
    }

    /// Check if the pin is in a down front state
    fn is_downfront(&self, pin: Self::PinType) -> bool {
        let pin: u64 = pin.into();
        use std::ops::Not;
        debug_assert_eq!(pin.count_ones(), 1);

        let &PinsBuffer(ref previous, ref current) = self.pins_buffer();

        let current = 0 != (current & pin);
        let previous = 0 != (previous & pin);

        previous == true && current == false
    }

    /// Whether *any* pin changed since the last [`save_pins_state`](Self::save_pins_state)
    /// call - a cheap, whole-buffer dirty check (one `u64` compare) rather
    /// than asking about one specific pin. Used by `Motherboard::
    /// sync_chips_pins()` to skip its entire cross-chip sync sweep on ticks
    /// where none of its source chips changed at all, since every
    /// destination it writes is purely driven by those sources - see that
    /// function's own comment for the full reasoning.
    fn any_pin_changed(&self) -> bool {
        let &PinsBuffer(previous, current) = self.pins_buffer();
        previous != current
    }

    /// Force the pin to be in front_down state (workaround because of the difference of speec d between chips)
    fn force_front_down(&mut self, pin: Self::PinType) {
        let pin: u64 = pin.into();

        let &mut PinsBuffer(ref mut previous, ref mut current) = self.pins_buffer_mut();

        *previous = *previous | pin;
        *current = *current & !pin;
    }

    /// Force to save all the bits of the current state to test is_front and so on
    /// (this is call at the syncing step in the motherboard code)
    fn save_pins_state(&mut self) {
        let buffer = self.pins_buffer_mut();
        buffer.0 = buffer.1;
    }

    /// Set the provided value in the provided list of pins. Pin index corresponds to the reversed number [bn, bn-1, ... b1, b0]
    /// panic if the number is larger
    fn set_value_in_pins<V>(&mut self, mut value: V, pins: &[Self::PinType])
    where
        V: num::Integer + num::cast::ToPrimitive + num::cast::FromPrimitive,
    {
        // Hoisted out of the loop: same reasoning as `get_pin`/`set_pin`'s
        // `debug_assert` change just above - `.unwrap()` is a panic-capable
        // call (even though `from_u8(2)` can never actually fail for any
        // real integer type), and calling it fresh every one of up to 16
        // bits per byte/word moved across a simulated bus added it to the
        // same hot path for no behavioural benefit.
        let two = V::from_u8(2).unwrap();
        for pin in pins.iter().rev() {
            self.set_pin(pin.clone(), value.is_odd());
            value = value.div_floor(&two);
        }
        //  assert!(value.to_usize().unwrap() == 0);
    }

    fn get_value_from_pins(&self, pins: &[Self::PinType]) -> u16 {
        debug_assert!(pins.len() <= 16);

        let mut res = 0;
        for pin in pins.iter() {
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

/// This trait needs to be activated for chips other than the GA that are activate in the front state of their clock
pub trait Activable: Chip {
    /// Get the id of the pin that contains the clock
    fn clock_pin() -> Self::PinType;

    fn activated_at_up_front() -> bool {
        true
    }

    /// Return true if the chip is activated (and thus its specific logic expect the wired pins one has to be executed)
    /// Currently activation is done with the font of the chip. it means that we executae the behavior of the chip in one row
    /// even if it takes several cycles of the Gate array chip that is emulated independnetly
    fn is_activated(&self) -> bool {
        if Self::activated_at_up_front() {
            self.is_upfront(Self::clock_pin())
        } else {
            self.is_downfront(Self::clock_pin())
        }
        //self.get_pin(Self::clock_pin())
    }
}

/// Synchronize the pins of the first chip to the pins of the second chip
pub fn sync_pins<C1: Chip, C2: Chip, const COUNT: usize>(
    chip1: &C1,
    pins1: &[C1::PinType; COUNT],
    chip2: &mut C2,
    pins2: &[C2::PinType; COUNT],
) {
    //   assert_eq!(pins1.len(), pins2.len());

    use itertools::zip;
    for (pin1, pin2) in zip(pins1.iter(), pins2.iter()) {
        chip2.set_pin(*pin2, chip1.get_pin(*pin1));
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[derive(Copy, Clone, Debug, PartialEq, Default)]
    struct MyChip {
        pins: PinsBuffer,
    }

    #[allow(non_camel_case_types)]
    #[derive(Copy, Clone, Debug, PartialEq)]
    #[repr(u64)]
    enum MyChipPin {
        A = 1 << 0,
        B = 1 << 1,
        C = 1 << 2,
    }

    impl Into<u64> for MyChipPin {
        fn into(self) -> u64 {
            self as u64
        }
    }

    impl Chip for MyChip {
        type PinType = MyChipPin;

        fn pins_buffer_mut(&mut self) -> &mut PinsBuffer {
            &mut self.pins
        }
        fn pins_buffer(&self) -> &PinsBuffer {
            &self.pins
        }

        fn must_execute_tick(&self) -> bool {
            unimplemented!()
        }
        fn reset(&mut self) {
            unimplemented!()
        }
        fn execute_tick(&mut self) {
            unimplemented!()
        }
    }

    #[test]
    fn test_setters() {
        let mut chip = MyChip::default();

        chip.set_pin(MyChipPin::A, false);
        chip.set_pin(MyChipPin::B, true);
        chip.set_pin(MyChipPin::C, false);

        assert_eq!(chip.pins.1, 0b010);

        chip.set_pin(MyChipPin::A, true);
        chip.set_pin(MyChipPin::B, false);
        chip.set_pin(MyChipPin::C, false);

        assert_eq!(chip.pins.1, 0b001);

        assert_eq!(
            chip.get_value_from_pins(&[MyChipPin::A, MyChipPin::B, MyChipPin::C]),
            0b100
        );

        chip.set_value_in_pins(0b010, &[MyChipPin::A, MyChipPin::B, MyChipPin::C]);
        assert_eq!(
            chip.get_value_from_pins(&[MyChipPin::A, MyChipPin::B, MyChipPin::C]),
            0b010
        );
    }

    #[test]
    fn test_front() {
        let mut chip = MyChip::default();

        chip.set_pin(MyChipPin::A, false);
        chip.save_pins_state();
        chip.set_pin(MyChipPin::A, true);

        assert_ne!(chip.pins.0, chip.pins.1);

        dbg!(&chip);
        assert!(chip.is_upfront(MyChipPin::A));

        chip.save_pins_state();
        chip.set_pin(MyChipPin::A, false);
        assert!(chip.is_downfront(MyChipPin::A));
    }
}
