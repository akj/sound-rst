//! The zoom: one of the nine subsquares of the current square.

use soundrts_world::MapSquare;
use soundrts_world::msg::{Msg, Phrase, msg};

/// The subsquares in the order the zoom tries them when it looks for a
/// unit, with their names.
pub const SUBZONES: [((i64, i64), Phrase); 9] = [
    ((0, 0), Phrase::AtTheCenter),
    ((0, 1), Phrase::North),
    ((0, -1), Phrase::South),
    ((1, 0), Phrase::East),
    ((-1, 0), Phrase::West),
    ((1, 1), Phrase::Northeast),
    ((-1, 1), Phrase::Northwest),
    ((1, -1), Phrase::Southeast),
    ((-1, -1), Phrase::Southwest),
];

fn subzone_name(sub: (i64, i64)) -> Msg {
    SUBZONES
        .iter()
        .find(|(k, _)| *k == sub)
        .map(|(_, m)| msg(*m))
        .unwrap_or_default()
}

/// A subsquare of a square: `sub_x` and `sub_y` go from -1 to 1. The
/// bounds are in millimeters.
#[derive(Clone, Debug, PartialEq)]
pub struct Zoom {
    pub sub_x: i64,
    pub sub_y: i64,
    xstep: f64,
    ystep: f64,
    pub xmin: f64,
    pub xmax: f64,
    pub ymin: f64,
    pub ymax: f64,
}

impl Zoom {
    /// The center of `sq`.
    pub fn new(sq: MapSquare) -> Zoom {
        let ((xmin, ymin), (xmax, ymax)) = (sq.min(), sq.max());
        let mut z = Zoom {
            sub_x: 0,
            sub_y: 0,
            xstep: (xmax - xmin) as f64 / 3.0,
            ystep: (ymax - ymin) as f64 / 3.0,
            xmin: 0.0,
            xmax: 0.0,
            ymin: 0.0,
            ymax: 0.0,
        };
        z.update_coords(sq);
        z
    }

    /// The center of this subsquare, in millimeters.
    pub fn point(&self, sq: MapSquare) -> (i64, i64) {
        let (cx, cy) = sq.center();
        let x = cx as f64 + self.sub_x as f64 * self.xstep;
        let y = cy as f64 + self.sub_y as f64 * self.ystep;
        (x.trunc() as i64, y.trunc() as i64)
    }

    pub fn title(&self, sq: MapSquare) -> Msg {
        let mut t = sq.title().clone();
        t.extend(subzone_name((self.sub_x, self.sub_y)));
        t
    }

    /// Moves by `dx`, `dy` subsquares. Past an edge, the zoom goes to the
    /// facing subsquare of the next square: the step to that square is
    /// returned (one square at most, east or west first).
    pub fn step(&mut self, dx: i64, dy: i64) -> Option<(i64, i64)> {
        self.sub_x += dx;
        self.sub_y += dy;
        if self.sub_x >= 2 {
            self.sub_x = -1;
            Some((1, 0))
        } else if self.sub_x <= -2 {
            self.sub_x = 1;
            Some((-1, 0))
        } else if self.sub_y >= 2 {
            self.sub_y = -1;
            Some((0, 1))
        } else if self.sub_y <= -2 {
            self.sub_y = 1;
            Some((0, -1))
        } else {
            None
        }
    }

    /// Goes to the first subsquare of `sq` (in [`SUBZONES`] order) that
    /// contains the point. Returns false, at the last subsquare tried, if
    /// none does.
    pub fn move_to(&mut self, sq: MapSquare, x: i64, y: i64) -> bool {
        for ((sx, sy), _) in SUBZONES {
            self.sub_x = sx;
            self.sub_y = sy;
            self.update_coords(sq);
            if self.contains(x, y) {
                return true;
            }
        }
        false
    }

    pub fn update_coords(&mut self, sq: MapSquare) {
        let (xmin, ymin) = sq.min();
        self.xmin = xmin as f64 + (self.sub_x + 1) as f64 * self.xstep;
        self.xmax = self.xmin + self.xstep;
        self.ymin = ymin as f64 + (self.sub_y + 1) as f64 * self.ystep;
        self.ymax = self.ymin + self.ystep;
    }

    /// Whether a point (in millimeters) is in the subsquare.
    pub fn contains(&self, x: i64, y: i64) -> bool {
        let (x, y) = (x as f64, y as f64);
        self.xmin <= x && x < self.xmax && self.ymin <= y && y < self.ymax
    }

    /// Where the listener stands, in meters: centered, near the south side.
    pub fn obs_pos(&self) -> (f64, f64) {
        let x = (self.xmin + self.xmax) / 2.0;
        let y = self.ymin + (self.ymax - self.ymin) / 8.0;
        (x / 1000.0, y / 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A game on a map of 3 by 3 squares of 12 m.
    fn host() -> soundrts_world::Host {
        soundrts_world::Host::new(&soundrts_world::GameSetup {
            rules: "def human_faction\nclass faction\n".into(),
            ai: String::new(),
            map: "square_width 12\nnb_columns 3\nnb_lines 3\nnb_players_min 1\nnb_players_max 1\nstarting_squares a1\n".into(),
            seed: 0,
            controllers: vec![soundrts_world::Controller::human("me", true)],
            default_triggers: Vec::new(),
            allow_cheatmode: false,
            random_starts: false,
            equivalents: false,
        })
        .unwrap()
    }

    #[test]
    fn the_zoom_cuts_a_square_in_nine_and_crosses_into_the_next() {
        let host = host();
        let view = soundrts_world::SeatView::new(host.game_start(), host.message(host.seats()[0]));
        // b3
        let sq = view.square(view.square_at(1, 2).unwrap());
        let mut z = Zoom::new(sq);
        assert_eq!(
            (z.xmin, z.xmax, z.ymin, z.ymax),
            (16000.0, 20000.0, 28000.0, 32000.0)
        );
        assert_eq!(
            z.title(sq),
            [sq.title().clone(), msg(Phrase::AtTheCenter)].concat()
        );
        assert_eq!(z.point(sq), (18000, 30000));
        assert!(z.contains(16000, 28000) && !z.contains(20000, 30000));
        assert_eq!(z.obs_pos(), (18.0, 28.5));
        assert_eq!(z.step(1, 0), None);
        z.update_coords(sq);
        assert_eq!(z.point(sq), (22000, 30000));
        assert_eq!(
            z.title(sq),
            [sq.title().clone(), msg(Phrase::East)].concat()
        );
        // Past the east edge: the west subsquare of the next square.
        assert_eq!(z.step(1, 0), Some((1, 0)));
        assert_eq!((z.sub_x, z.sub_y), (-1, 0));
        // Five steps at once still cross one square only.
        assert_eq!(z.step(0, -5), Some((0, -1)));
        assert_eq!((z.sub_x, z.sub_y), (-1, 1));
        // A point in the north-east subsquare.
        assert!(z.move_to(sq, 23000, 33000));
        assert_eq!((z.sub_x, z.sub_y), (1, 1));
        assert!(!z.move_to(sq, 0, 0));
        assert_eq!((z.sub_x, z.sub_y), (-1, -1));
    }
}
