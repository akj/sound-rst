//! Squares of the map grid and the search for free space (`worldroom.py`).

use crate::entity::{COLLISION_RADIUS, EntityId, SquareId};
use crate::msg::{Msg, MsgPart, nb2msg};
use crate::rules::AirGround;

/// The most units of one layer a square can receive by teleportation or
/// recall.
pub const SPACE_LIMIT: usize = 144;

/// One square of the map.
#[derive(Clone, Debug)]
pub struct Square {
    /// The id in the text protocol.
    pub id: u32,
    pub col: i64,
    pub row: i64,
    /// `a1`, `b3`...: the column letter and the row number.
    pub name: String,
    /// The column sound and the row number.
    pub title: Msg,
    /// What is on the square, in arrival order.
    pub objects: Vec<EntityId>,
    /// The exits of the square, in creation order.
    pub exits: Vec<EntityId>,
    pub xmin: i64,
    pub ymin: i64,
    pub xmax: i64,
    pub ymax: i64,
    /// The center.
    pub x: i64,
    pub y: i64,
    pub high_ground: bool,
    /// The terrain style: a map `terrain`, or `_meadows`, `_forest`,
    /// `_dense_forest` or nothing, set from the resources.
    pub type_name: String,
    /// Speed percentage for ground and air units.
    pub terrain_speed: (i64, i64),
    /// Cover percentage for ground and air units.
    pub terrain_cover: (i64, i64),
    pub is_ground: bool,
    pub is_water: bool,
    pub is_air: bool,
    /// The four squares sharing a side, in the order north, south, east,
    /// west.
    pub strict_neighbors: Vec<SquareId>,
    /// The eight squares around.
    pub neighbors: Vec<SquareId>,
    /// The spirals of free space searches from the center, one per layer,
    /// started at the first search during world time 0.
    pub(crate) spirals: Option<[Spiral; 3]>,
}

impl Square {
    pub(crate) fn new(id: u32, col: i64, row: i64, width: i64) -> Square {
        let letter = (b'a' + col as u8) as char;
        let name = format!("{letter}{}", row + 1);
        let mut title = vec![MsgPart::Num(5000 + col as u64)];
        title.extend(nb2msg(row + 1));
        let xmin = col * width;
        let ymin = row * width;
        let xmax = xmin + width;
        let ymax = ymin + width;
        Square {
            id,
            col,
            row,
            name,
            title,
            objects: Vec::new(),
            exits: Vec::new(),
            xmin,
            ymin,
            xmax,
            ymax,
            x: (xmax + xmin) / 2,
            y: (ymax + ymin) / 2,
            high_ground: false,
            type_name: String::new(),
            terrain_speed: (100, 100),
            terrain_cover: (0, 0),
            is_ground: true,
            is_water: false,
            is_air: true,
            strict_neighbors: Vec::new(),
            neighbors: Vec::new(),
            spirals: None,
        }
    }

    pub fn height(&self) -> i64 {
        i64::from(self.high_ground)
    }

    pub fn contains(&self, x: i64, y: i64) -> bool {
        self.xmin <= x && x < self.xmax && self.ymin <= y && y < self.ymax
    }

    /// The side positions where exits are placed: square, x, y, angle.
    pub fn north_side(&self) -> (i64, i64, i64) {
        (self.x, self.ymax - 1, -90)
    }

    pub fn south_side(&self) -> (i64, i64, i64) {
        (self.x, self.ymin, 90)
    }

    pub fn east_side(&self) -> (i64, i64, i64) {
        (self.xmax - 1, self.y, 180)
    }

    pub fn west_side(&self) -> (i64, i64, i64) {
        (self.xmin, self.y, 0)
    }

    /// The centers of the nine subsquares, in the original's order: center
    /// column first, then east, then west; in each, center, north, south.
    pub fn subsquare_centers(&self) -> [(i64, i64); 9] {
        let k = (self.xmax - self.xmin) / 3;
        let mut out = [(0, 0); 9];
        let mut i = 0;
        for dx in [0, 1, -1] {
            for dy in [0, 1, -1] {
                out[i] = (self.x + dx * k, self.y + dy * k);
                i += 1;
            }
        }
        out
    }
}

/// The points tried, in order, when looking for free space around a point:
/// the point, then a square spiral of 24 turns with a step of 2.5 collision
/// radii.
#[derive(Clone, Debug)]
pub(crate) struct Spiral {
    x: i64,
    y: i64,
    started: bool,
    sign: i64,
    delta: i64,
    /// 0: moving in x, 1: moving in y.
    leg: u8,
    done_in_leg: i64,
    finished: bool,
}

const SPIRAL_STEP: i64 = COLLISION_RADIUS * 25 / 10;

impl Spiral {
    pub(crate) fn new(x: i64, y: i64) -> Spiral {
        Spiral {
            x,
            y,
            started: false,
            sign: 1,
            delta: 1,
            leg: 0,
            done_in_leg: 0,
            finished: false,
        }
    }
}

impl Iterator for Spiral {
    type Item = (i64, i64);

    fn next(&mut self) -> Option<(i64, i64)> {
        if !self.started {
            self.started = true;
            return Some((self.x, self.y));
        }
        loop {
            if self.finished || self.delta >= 25 {
                self.finished = true;
                return None;
            }
            if self.done_in_leg < self.delta {
                self.done_in_leg += 1;
                if self.leg == 0 {
                    self.x += self.sign * SPIRAL_STEP;
                } else {
                    self.y += self.sign * SPIRAL_STEP;
                }
                return Some((self.x, self.y));
            }
            self.done_in_leg = 0;
            if self.leg == 0 {
                self.leg = 1;
            } else {
                self.leg = 0;
                self.delta += 1;
                self.sign = -self.sign;
            }
        }
    }
}

pub(crate) fn spiral_index(a: AirGround) -> usize {
    match a {
        AirGround::Ground => 0,
        AirGround::Air => 1,
        AirGround::Water => 2,
    }
}

/// The subsquare a point belongs to: the map cut in thirds of squares.
pub fn subsquare_id(x: i64, y: i64, square_width: i64) -> (i64, i64) {
    (
        crate::nofloat::floordiv(x * 3, square_width),
        crate::nofloat::floordiv(y * 3, square_width),
    )
}

#[cfg(test)]
mod tests {
    use super::Spiral;

    #[test]
    fn the_spiral_starts_on_the_point_and_turns_with_growing_legs() {
        let points: Vec<(i64, i64)> = Spiral::new(0, 0).take(7).collect();
        assert_eq!(
            points,
            [
                (0, 0),
                (437, 0),
                (437, 437),
                (0, 437),
                (-437, 437),
                (-437, 0),
                (-437, -437)
            ]
        );
        assert_eq!(Spiral::new(0, 0).count(), 601);
    }
}
