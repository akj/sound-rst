//! What a renderer is asked to play: a listener, and sources with a sound,
//! a position and a loudness. The renderer works out what each ear hears.

use std::sync::Arc;

/// Samples per second of every sound and every rendered take.
pub const RATE: u32 = 48_000;

/// A sound as the renderers take it: mono samples at [`RATE`]. Sample 0
/// plays at the start of the take.
pub type Sound = Arc<[f32]>;

/// A position in metres: `x` east, `y` north, `z` up.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64, z: f64) -> Point {
        Point { x, y, z }
    }
}

/// Where the player listens from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener {
    pub position: Point,
    /// The way the listener faces, in degrees: 0 is east, 90 is north, as
    /// in the game.
    pub facing: f64,
}

impl Listener {
    /// At the origin, facing north.
    pub const NORTH: Listener = Listener {
        position: Point::new(0.0, 0.0, 0.0),
        facing: 90.0,
    };

    /// `p` in the listener's own frame: metres to the right, ahead and up.
    pub fn relative(&self, p: Point) -> (f64, f64, f64) {
        let (dx, dy) = (p.x - self.position.x, p.y - self.position.y);
        let (sin, cos) = self.facing.to_radians().sin_cos();
        let ahead = dx * cos + dy * sin;
        let right = dx * sin - dy * cos;
        (right, ahead, p.z - self.position.z)
    }
}

/// A sound playing somewhere.
#[derive(Clone, Debug)]
pub struct Source {
    /// Stays the same while the same source plays, so a renderer can keep
    /// its state (smoothing, filters) from one block to the next.
    pub id: u32,
    pub sound: Sound,
    pub position: Point,
    /// 1 is the sound as recorded.
    pub loudness: f64,
    /// Heard through something closed (a neighbour with no open exit).
    pub blocked: bool,
}

/// The terrain the listener stands in, which a renderer may give an echo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Place {
    /// No echo at all.
    #[default]
    Dry,
    Meadow,
    Forest,
    MountainPass,
    Town,
}

impl Place {
    pub fn name(self) -> &'static str {
        match self {
            Place::Dry => "no echo",
            Place::Meadow => "an open meadow",
            Place::Forest => "a forest",
            Place::MountainPass => "a mountain pass",
            Place::Town => "inside a town",
        }
    }

    /// How long the place's echo takes to die away by 60 dB, in seconds,
    /// in a low, a middle and a high band. `None` for [`Place::Dry`].
    pub fn decay_times(self) -> Option<[f32; 3]> {
        match self {
            Place::Dry => None,
            Place::Meadow => Some([0.4, 0.3, 0.2]),
            Place::Forest => Some([1.0, 0.8, 0.4]),
            Place::MountainPass => Some([3.0, 2.5, 1.5]),
            Place::Town => Some([1.4, 1.2, 0.8]),
        }
    }
}

/// Everything that sounds at one moment.
#[derive(Clone, Debug)]
pub struct Scene {
    pub listener: Listener,
    pub sources: Vec<Source>,
    pub place: Place,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_positions_follow_the_facing() {
        let close = |a: (f64, f64, f64), b: (f64, f64, f64)| {
            (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9 && (a.2 - b.2).abs() < 1e-9
        };
        let north = Listener::NORTH;
        assert!(close(
            north.relative(Point::new(0.0, 2.0, 1.0)),
            (0.0, 2.0, 1.0)
        ));
        assert!(close(
            north.relative(Point::new(3.0, 0.0, 0.0)),
            (3.0, 0.0, 0.0)
        ));
        let east = Listener {
            position: Point::new(1.0, 1.0, 0.0),
            facing: 0.0,
        };
        // South of an east-facing listener is to their right.
        assert!(close(
            east.relative(Point::new(1.0, -1.0, 0.0)),
            (2.0, 0.0, 0.0)
        ));
    }
}
