//! The client's own randomness: ambient volumes, noise variants, footstep
//! timing and the seed of a new game. It never touches the simulation's
//! generator. It draws as the original's global `random` module does
//! (CPython's Mersenne Twister), so a client seeded alike plays its noises
//! alike.

use std::cell::RefCell;
use std::rc::Rc;

use soundrts_world::pyrandom::PyRandom;

/// A shared generator. Clones draw from the same sequence.
#[derive(Clone)]
pub struct ClientRng(Rc<RefCell<PyRandom>>);

impl ClientRng {
    /// A generator seeded as `random.seed(seed)`.
    pub fn new(seed: u64) -> ClientRng {
        ClientRng(Rc::new(RefCell::new(PyRandom::new(seed as i64))))
    }

    /// A number in `[0, 1)`: `random.random()`.
    pub fn random(&self) -> f64 {
        self.0.borrow_mut().random()
    }

    /// The index `random.choice` picks among `len` items (0 if `len` is 0,
    /// without drawing).
    pub fn below(&self, len: usize) -> usize {
        self.0.borrow_mut().choice_index(len).unwrap_or(0)
    }

    /// An integer from `a` to `b` included: `random.randint(a, b)`.
    pub fn randint(&self, a: i64, b: i64) -> i64 {
        self.0.borrow_mut().randint(a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_are_cpythons() {
        // random.seed(42); random.random(); random.choice(range(3));
        // random.randint(0, 10000)
        let r = ClientRng::new(42);
        assert_eq!(r.random(), 0.6394267984578837);
        assert_eq!(r.below(3), 0);
        assert_eq!(r.randint(0, 10000), 4506);
    }
}
