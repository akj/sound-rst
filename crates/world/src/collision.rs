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
