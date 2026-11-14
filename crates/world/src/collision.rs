//! The occupied cells of one layer of the map (`lib/collision.py`).

use std::collections::HashSet;

use crate::nofloat::floordiv;

/// The cells an object occupies around its own cell: a plus sign.
const SHAPE: [(i64, i64); 5] = [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)];

/// A set of occupied cells, by column and row. It is a set, not a count:
/// removing one of two objects that shared a cell would free it, but no two
/// objects share a cell, because an object is placed only where all its
/// cells are free.
///
/// Only membership is ever asked, so the hash set's order never matters.
#[derive(Clone, Debug, Default)]
pub struct CollisionMatrix {
    cells: HashSet<(i64, i64)>,
    res: i64,
}

/// The occupied cells in ascending order, so that two matrices with the
/// same cells hash alike.
impl std::hash::Hash for CollisionMatrix {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let CollisionMatrix { cells, res } = self;
        let mut cells: Vec<(i64, i64)> = cells.iter().copied().collect();
        cells.sort_unstable();
        cells.hash(state);
        res.hash(state);
    }
}

impl CollisionMatrix {
    /// Cells of `res` by `res` millimeters.
    pub fn new(res: i64) -> Self {
        CollisionMatrix {
            cells: HashSet::new(),
            res,
        }
    }

    fn shape(&self, x: i64, y: i64) -> impl Iterator<Item = (i64, i64)> {
        let (col, row) = (floordiv(x, self.res), floordiv(y, self.res));
        SHAPE.iter().map(move |&(a, b)| (col + a, row + b))
    }

    pub fn would_collide(&self, x: i64, y: i64) -> bool {
        self.shape(x, y).any(|k| self.cells.contains(&k))
    }

    /// Whether an object at (`x`, `y`) would collide if the cells of the
    /// object at (`own_x`, `own_y`) were removed. The set is not changed:
    /// removing and adding the cells back would occupy again any cell that
    /// another object had freed.
    pub fn would_collide_without(&self, x: i64, y: i64, own_x: i64, own_y: i64) -> bool {
        let own: Vec<(i64, i64)> = self.shape(own_x, own_y).collect();
        self.shape(x, y)
            .any(|k| !own.contains(&k) && self.cells.contains(&k))
    }

    pub fn add(&mut self, x: i64, y: i64) {
        let keys: Vec<(i64, i64)> = self.shape(x, y).collect();
        self.cells.extend(keys);
    }

    pub fn remove(&mut self, x: i64, y: i64) {
        let keys: Vec<(i64, i64)> = self.shape(x, y).collect();
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
        let mut m = CollisionMatrix::new(2);
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
        let mut a = CollisionMatrix::new(2);
        let mut b = CollisionMatrix::new(2);
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
    fn testing_without_an_object_matches_removing_it_from_a_copy() {
        let mut m = CollisionMatrix::new(2);
        m.add(6, 6);
        m.add(8, 6);
        m.add(30, 30);
        // (8, 6) shares cells with (6, 6); freeing them leaves (6, 6) partial.
        m.remove(8, 6);
        for (x, y) in [(6, 6), (8, 6), (10, 6), (4, 6), (28, 30), (100, 100)] {
            let mut copy = m.clone();
            copy.remove(6, 6);
            assert_eq!(
                m.would_collide_without(x, y, 6, 6),
                copy.would_collide(x, y),
                "({x}, {y})"
            );
        }
        assert!(!m.would_collide_without(8, 6, 6, 6));
        assert!(m.would_collide_without(28, 30, 6, 6));
        assert_eq!(m.len(), 8, "the set is unchanged");
    }

    #[test]
    fn an_object_at_the_right_edge_does_not_occupy_the_left_edge_of_the_next_row() {
        // 100 columns of 2 millimeters.
        let mut m = CollisionMatrix::new(2);
        m.add(199, 6);
        assert!(m.would_collide(201, 6), "its right arm");
        assert!(!m.would_collide(0, 10));
        assert!(!m.would_collide(0, 4));
    }

    #[test]
    fn removing_one_of_two_overlapping_objects_frees_the_shared_cells() {
        let mut m = CollisionMatrix::new(2);
        m.add(6, 6);
        m.add(8, 6);
        m.remove(8, 6);
        assert!(!m.would_collide(10, 6));
        assert!(m.would_collide(4, 6));
    }
}
