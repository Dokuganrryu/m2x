pub const SEQUENCES: u64 = 10_000;
pub const EVENTS_PER_SEQUENCE: usize = 1_000;

pub struct XorShift(u64);

impl XorShift {
    pub fn seeded(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }

    pub fn next_word(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub fn below(&mut self, bound: u32) -> u32 {
        (self.next_word() % u64::from(bound)) as u32
    }
}
