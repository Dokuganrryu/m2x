pub struct XorShift(u64);

impl XorShift {
    pub fn seeded(seed: u64) -> Self {
        Self(seed | 1)
    }

    pub fn below(&mut self, bound: u32) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % bound as u64) as u32
    }
}
