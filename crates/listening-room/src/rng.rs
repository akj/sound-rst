//! A small seeded random generator (xorshift64*), so a session can be
//! repeated from its seed.

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        // Any seed but zero works; mixing keeps nearby seeds apart.
        Rng((seed ^ 0x9E37_79B9_7F4A_7C15) | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number from 0 to `n - 1`.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }

    /// A sample of white noise, from -1 to 1.
    pub fn noise(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 23) as f32 - 1.0
    }

    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i + 1));
        }
    }
}
