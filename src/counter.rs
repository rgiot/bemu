//! Counting related stuff

#[derive(Copy, Clone, Debug, Default)]
pub struct Counter(usize);

impl From<usize> for Counter {
    fn from(val: usize) -> Self {
        Counter(val)
    }
}

impl From<&Counter> for usize {
    fn from(val: &Counter) -> Self {
        val.0
    }
}

impl Counter {
    pub fn inc(&mut self) {
        self.0 = self.0.wrapping_add(1);
    }

    pub fn reset(&mut self) {
        self.0 = 0;
    }

    pub fn value(&self) -> usize {
        self.into()
    }
}
