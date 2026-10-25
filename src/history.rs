//! The last values of a signal, for the per-tick delay lines of the chips.

/// Slots of every history: a power of two, so that the index wraps with a mask (no branch, no
/// bounds check), at least the longest history (22 ticks of palette lookup).
const SLOTS: usize = 32;

/// The last `N` values pushed, newest first ([`Self::get`] by age). A push moves an index, not
/// the values: rotating the arrays a tick at a time took a fifth of the emulation's time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct History<T, const N: usize> {
    values: [T; SLOTS],
    /// Index of the newest value.
    newest: usize
}

impl<T: Copy, const N: usize> History<T, N> {
    const FITS: () = assert!(N > 0 && N <= SLOTS, "a history longer than SLOTS");

    /// A history of `N` times `value`.
    pub fn new(value: T) -> Self {
        let () = Self::FITS;
        Self { values: [value; SLOTS], newest: 0 }
    }

    /// `value` is the newest; the oldest goes.
    pub fn push(&mut self, value: T) {
        self.newest = self.newest.wrapping_sub(1) & (SLOTS - 1);
        self.values[self.newest] = value;
    }

    /// The value pushed `age` pushes ago (0: the newest), `age < N`.
    pub fn get(&self, age: usize) -> T {
        debug_assert!(age < N);
        self.values[(self.newest + age) & (SLOTS - 1)]
    }

    /// Replaces the value pushed `age` pushes ago, `age < N`.
    pub fn set(&mut self, age: usize, value: T) {
        debug_assert!(age < N);
        self.values[(self.newest + age) & (SLOTS - 1)] = value;
    }

    /// The value of the delay line of `N` ticks: what was pushed `N` pushes ago, then `value` is
    /// pushed.
    pub fn delay(&mut self, value: T) -> T {
        let oldest = self.get(N - 1);
        self.push(value);
        oldest
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn values_come_back_by_age() {
        let mut history = History::<u32, 3>::new(0);
        for value in 1..=5 {
            history.push(value);
        }
        assert_eq!([history.get(0), history.get(1), history.get(2)], [5, 4, 3]);
    }

    /// The same as the arrays rotated a tick at a time.
    #[test]
    fn a_delay_line_is_an_array_shifted_each_tick() {
        let mut pipe = [0u32; 4];
        let mut line = History::<u32, 4>::new(0);
        for value in 1..20 {
            let expected = pipe[0];
            pipe.rotate_left(1);
            pipe[3] = value;
            assert_eq!(line.delay(value), expected);
        }
    }
}
