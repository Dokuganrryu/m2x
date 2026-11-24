#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputList<T, const CAPACITY: usize> {
    items: [T; CAPACITY],
    len: usize,
}

pub trait Filler: Copy {
    const FILLER: Self;
}

impl<T, const CAPACITY: usize> OutputList<T, CAPACITY> {
    pub fn as_slice(&self) -> &[T] {
        self.items.get(..self.len).unwrap_or(&[])
    }

    pub(crate) fn clear(&mut self) {
        self.len = 0;
    }
}

impl<T: Filler, const CAPACITY: usize> OutputList<T, CAPACITY> {
    pub(crate) const NONE: Self = Self::of([]);

    pub(crate) const fn of<const N: usize>(outputs: [T; N]) -> Self {
        const { assert!(N <= CAPACITY) };
        let mut items = [T::FILLER; CAPACITY];
        let mut index = 0;
        while index < N {
            items[index] = outputs[index];
            index += 1;
        }
        Self { items, len: N }
    }

    pub(crate) fn push(&mut self, item: T) {
        debug_assert!(self.len < CAPACITY, "the output list holds every output");
        if let Some(slot) = self.items.get_mut(self.len) {
            *slot = item;
            self.len += 1;
        }
    }
}
