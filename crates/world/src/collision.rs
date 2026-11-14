//! The occupied cells of one layer of the map (`lib/collision.py`).

use std::collections::HashSet;

use crate::nofloat::floordiv;

/// The cells an object occupies around its own cell: a plus sign.
const SHAPE: [(i64, i64); 5] = [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)];

/// A set of occupied cells. It is a set, not a count: when two objects
/// share a cell, removing one of them frees the cell, as in the original.
///
/// Only membership is ever asked, so the hash set's order never matters.
#[derive(Clone, Debug, Default)]
pub struct CollisionMatrix {
    cells: HashSet<i64>,
    res: i64,
    amax: i64,
}

/// The occupied cells in ascending order, so that two matrices with the
/// same cells hash alike.
impl std::hash::Hash for CollisionMatrix {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let CollisionMatrix { cells, res, amax } = self;
        let mut cells: Vec<i64> = cells.iter().copied().collect();
        cells.sort_unstable();
        cells.hash(state);
        res.hash(state);
        amax.hash(state);
    }
}

impl CollisionMatrix {
    pub fn new(xmax: i64, res: i64) -> Self {
        CollisionMatrix {
            cells: HashSet::new(),
            res,
            amax: floordiv(xmax, res),
        }
    }

    fn key(&self, x: i64, y: i64) -> i64 {
        floordiv(x, self.res) + self.amax * floordiv(y, self.res)
    }

    fn shape(&self, x: i64, y: i64) -> impl Iterator<Item = i64> + '_ {
        let k = self.key(x, y);
        SHAPE.iter().map(move |&(a, b)| k + a + self.amax * b)
    }

    pub fn would_collide(&self, x: i64, y: i64) -> bool {
        self.shape(x, y).any(|k| self.cells.contains(&k))
    }

    pub fn add(&mut self, x: i64, y: i64) {
        let keys: Vec<i64> = self.shape(x, y).collect();
        self.cells.extend(keys);
    }

    pub fn remove(&mut self, x: i64, y: i64) {
        let keys: Vec<i64> = self.shape(x, y).collect();
        for k in keys {
            self.cells.remove(&k);
        }
    }

    /// The number of occupied cells.
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::CollisionMatrix;

    #[test]
    fn an_object_occupies_a_plus_sign_of_cells() {
        let mut m = CollisionMatrix::new(200, 2);
        assert!(!m.would_collide(6, 6));
        m.add(6, 6);
        assert_eq!(m.len(), 5);
        assert!(m.would_collide(6, 6));
        // two cells to the right still touches the right arm
        assert!(m.would_collide(10, 6));
        assert!(!m.would_collide(12, 6));
        m.remove(6, 6);
        assert!(!m.would_collide(6, 6));
    }

    fn hash_of(m: &CollisionMatrix) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        m.hash(&mut h);
        h.finish()
    }

    #[test]
    fn the_hash_depends_on_the_cells_and_not_on_their_history() {
        let mut a = CollisionMatrix::new(200, 2);
        let mut b = CollisionMatrix::new(200, 2);
        a.add(6, 6);
        a.add(60, 6);
        b.add(60, 6);
        b.add(20, 20);
        b.add(6, 6);
        assert_ne!(hash_of(&a), hash_of(&b));
        b.remove(20, 20);
        assert_eq!(hash_of(&a), hash_of(&b));
    }

    #[test]
    fn removing_one_of_two_overlapping_objects_frees_the_shared_cells() {
        let mut m = CollisionMatrix::new(200, 2);
        m.add(6, 6);
        m.add(8, 6);
        m.remove(8, 6);
        assert!(!m.would_collide(10, 6));
        assert!(m.would_collide(4, 6));
    }
}
