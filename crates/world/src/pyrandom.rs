//! A random number generator that gives the same numbers as CPython 3.11's
//! `random.Random`, so a game can be compared tick by tick with the
//! original.

const N: usize = 624;
const M: usize = 397;
const MATRIX_A: u32 = 0x9908_b0df;
const UPPER_MASK: u32 = 0x8000_0000;
const LOWER_MASK: u32 = 0x7fff_ffff;

/// The Mersenne Twister of CPython, with the derived methods the game uses.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PyRandom {
    mt: Box<[u32; N]>,
    index: usize,
}

impl PyRandom {
    /// `random.Random(); r.seed(seed)` for an integer seed.
    pub fn new(seed: i64) -> Self {
        let mut r = PyRandom {
            mt: Box::new([0; N]),
            index: N + 1,
        };
        r.seed(seed);
        r
    }

    /// `seed(seed)` for an integer seed. Python uses the absolute value.
    pub fn seed(&mut self, seed: i64) {
        let n = seed.unsigned_abs();
        let mut key = vec![n as u32];
        if n >> 32 != 0 {
            key.push((n >> 32) as u32);
        }
        self.init_by_array(&key);
    }

    fn init_genrand(&mut self, s: u32) {
        self.mt[0] = s;
        for i in 1..N {
            let prev = self.mt[i - 1];
            self.mt[i] = 1_812_433_253u32
                .wrapping_mul(prev ^ (prev >> 30))
                .wrapping_add(i as u32);
        }
        self.index = N;
    }

    fn init_by_array(&mut self, key: &[u32]) {
        self.init_genrand(19_650_218);
        let mut i = 1;
        let mut j = 0;
        for _ in 0..N.max(key.len()) {
            let prev = self.mt[i - 1];
            self.mt[i] = (self.mt[i] ^ (prev ^ (prev >> 30)).wrapping_mul(1_664_525))
                .wrapping_add(key[j])
                .wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= N {
                self.mt[0] = self.mt[N - 1];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
        }
        for _ in 0..N - 1 {
            let prev = self.mt[i - 1];
            self.mt[i] = (self.mt[i] ^ (prev ^ (prev >> 30)).wrapping_mul(1_566_083_941))
                .wrapping_sub(i as u32);
            i += 1;
            if i >= N {
                self.mt[0] = self.mt[N - 1];
                i = 1;
            }
        }
        self.mt[0] = 0x8000_0000;
    }

    fn genrand_u32(&mut self) -> u32 {
        if self.index >= N {
            for kk in 0..N {
                let y = (self.mt[kk] & UPPER_MASK) | (self.mt[(kk + 1) % N] & LOWER_MASK);
                let mag = if y & 1 == 1 { MATRIX_A } else { 0 };
                self.mt[kk] = self.mt[(kk + M) % N] ^ (y >> 1) ^ mag;
            }
            self.index = 0;
        }
        let mut y = self.mt[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^= y >> 18;
        y
    }

    /// `random()`: a float in `[0, 1)` with 53 random bits.
    pub fn random(&mut self) -> f64 {
        let a = self.genrand_u32() >> 5;
        let b = self.genrand_u32() >> 6;
        (f64::from(a) * 67_108_864.0 + f64::from(b)) * (1.0 / 9_007_199_254_740_992.0)
    }

    /// `getrandbits(k)` for `k <= 128`.
    pub fn getrandbits(&mut self, k: u32) -> u128 {
        assert!(k <= 128, "getrandbits is limited to 128 bits here");
        if k == 0 {
            return 0;
        }
        if k <= 32 {
            return u128::from(self.genrand_u32() >> (32 - k));
        }
        let mut result: u128 = 0;
        let mut remaining = k;
        let mut shift = 0;
        while remaining > 0 {
            let mut r = self.genrand_u32();
            if remaining < 32 {
                r >>= 32 - remaining;
            }
            result |= u128::from(r) << shift;
            shift += 32;
            remaining = remaining.saturating_sub(32);
        }
        result
    }

    /// `_randbelow(n)`: an integer in `[0, n)` for `n > 0`.
    pub fn randbelow(&mut self, n: u64) -> u64 {
        let k = 64 - n.leading_zeros();
        let mut r = self.getrandbits(k) as u64;
        while r >= n {
            r = self.getrandbits(k) as u64;
        }
        r
    }

    /// `randrange(start, stop)`. Panics if the range is empty, where Python
    /// raises `ValueError`.
    pub fn randrange(&mut self, start: i64, stop: i64) -> i64 {
        let width = stop - start;
        assert!(width > 0, "empty range for randrange({start}, {stop})");
        start + self.randbelow(width as u64) as i64
    }

    /// `randint(a, b)`: an integer in `[a, b]`.
    pub fn randint(&mut self, a: i64, b: i64) -> i64 {
        self.randrange(a, b + 1)
    }

    /// The index `choice(seq)` picks in a sequence of `len` items, or `None`
    /// for an empty sequence, where Python raises `IndexError`.
    pub fn choice_index(&mut self, len: usize) -> Option<usize> {
        if len == 0 {
            return None;
        }
        Some(self.randbelow(len as u64) as usize)
    }

    /// `choice(seq)`.
    pub fn choice<'a, T>(&mut self, seq: &'a [T]) -> Option<&'a T> {
        self.choice_index(seq.len()).map(|i| &seq[i])
    }

    /// `shuffle(x)`, in place.
    pub fn shuffle<T>(&mut self, x: &mut [T]) {
        for i in (1..x.len()).rev() {
            let j = self.randbelow(i as u64 + 1) as usize;
            x.swap(i, j);
        }
    }

    /// `sample(population, k)`. Returns `None` if `k` is larger than the
    /// population, where Python raises `ValueError`.
    pub fn sample<T: Clone>(&mut self, population: &[T], k: usize) -> Option<Vec<T>> {
        let n = population.len();
        if k > n {
            return None;
        }
        let mut setsize = 21;
        if k > 5 {
            // 4 ** ceil(log(3k, 4)); 3k is never a power of 4, so the
            // smallest power of 4 not below 3k is exact.
            let mut p = 1;
            while p < 3 * k {
                p *= 4;
            }
            setsize += p;
        }
        let mut result = Vec::with_capacity(k);
        if n <= setsize {
            let mut pool: Vec<T> = population.to_vec();
            for i in 0..k {
                let j = self.randbelow((n - i) as u64) as usize;
                result.push(pool[j].clone());
                pool[j] = pool[n - i - 1].clone();
            }
        } else {
            let mut selected = std::collections::BTreeSet::new();
            for _ in 0..k {
                let mut j = self.randbelow(n as u64) as usize;
                while selected.contains(&j) {
                    j = self.randbelow(n as u64) as usize;
                }
                selected.insert(j);
                result.push(population[j].clone());
            }
        }
        Some(result)
    }

    /// A short fingerprint of the generator state, for digests.
    pub fn state_fingerprint(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &w in self.mt.iter() {
            h = (h ^ u64::from(w)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        (h ^ self.index as u64).wrapping_mul(0x0000_0100_0000_01b3)
    }
}

#[cfg(test)]
mod tests {
    use super::PyRandom;

    // Every expected value below was printed by CPython 3.11.15 running the
    // same calls on `random.Random()` seeded the same way, in this order.
    #[test]
    fn the_sequence_after_seed_0_matches_cpython() {
        let mut r = PyRandom::new(0);
        assert_eq!(r.random(), 0.8444218515250481);
        assert_eq!(r.random(), 0.7579544029403025);
        assert_eq!(r.random(), 0.420571580830845);
        assert_eq!(r.getrandbits(32), 1112038970);
        assert_eq!(r.getrandbits(5), 30);
        assert_eq!(r.getrandbits(70), 470132385667577210466);
        assert_eq!(r.getrandbits(0), 0);
        let ints: Vec<i64> = (0..5).map(|_| r.randint(1, 100)).collect();
        assert_eq!(ints, [39, 62, 46, 75, 28]);
        let ranges: Vec<i64> = (0..5).map(|_| r.randrange(0, 7)).collect();
        assert_eq!(ranges, [4, 1, 2, 1, 6]);
        let digits: Vec<i64> = (0..10).collect();
        assert_eq!(r.choice(&digits), Some(&1));
        let mut l: Vec<i64> = (0..10).collect();
        r.shuffle(&mut l);
        assert_eq!(l, [3, 8, 1, 6, 5, 0, 7, 2, 4, 9]);
        let thirty: Vec<i64> = (0..30).collect();
        assert_eq!(r.sample(&thirty, 5).unwrap(), [11, 13, 10, 19, 20]);
        let hundred: Vec<i64> = (0..100).collect();
        assert_eq!(
            r.sample(&hundred, 30).unwrap(),
            [
                26, 70, 61, 56, 66, 33, 7, 98, 1, 11, 51, 85, 80, 0, 78, 63, 42, 31, 41, 8, 24, 72,
                28, 30, 18, 69, 57, 90, 10, 40
            ]
        );
    }

    #[test]
    fn other_seeds_match_cpython() {
        let mut r = PyRandom::new(0);
        r.seed(12345);
        assert_eq!(r.random(), 0.41661987254534116);
        assert_eq!(r.randint(1, 100), 2);
        r.seed((1 << 40) + 7);
        assert_eq!(r.random(), 0.6137037779936511);
        r.seed(-5);
        assert_eq!(r.random(), 0.6229016948897019);
        r.seed(4096);
        assert_eq!(r.getrandbits(32), 2861572656);
    }

    #[test]
    fn empty_choices_and_oversized_samples_are_refused() {
        let mut r = PyRandom::new(1);
        let empty: [u8; 0] = [];
        assert_eq!(r.choice(&empty), None);
        assert_eq!(r.sample(&[1, 2], 3), None);
    }
}
