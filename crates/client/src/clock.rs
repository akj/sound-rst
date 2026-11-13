//! Time, behind a seam so blocking loops finish instantly under test.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Where the client reads the time and waits.
pub trait Clock {
    /// Seconds since an arbitrary fixed start.
    fn now(&self) -> f64;
    /// Waits `seconds`.
    fn sleep(&self, seconds: f64);
}

/// The clock handle every part of the client shares.
pub type ClockRef = Rc<dyn Clock>;

/// The real, monotonic clock.
pub struct SystemClock {
    start: Instant,
}

impl SystemClock {
    pub fn new() -> Self {
        Self::starting_at(Instant::now())
    }

    /// A clock whose time 0 is `start`.
    pub fn starting_at(start: Instant) -> Self {
        SystemClock { start }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    fn sleep(&self, seconds: f64) {
        if seconds > 0.0 {
            std::thread::sleep(Duration::from_secs_f64(seconds));
        }
    }
}

/// A clock that only moves when something sleeps or a test advances it,
/// or, once told to follow real time, a clock that also runs on its own.
#[derive(Default)]
pub struct ManualClock {
    now: Cell<f64>,
    /// Since when the clock follows real time.
    real_since: Cell<Option<Instant>>,
}

impl ManualClock {
    pub fn new() -> Self {
        Self::default()
    }

    /// From now on the clock runs in real time and a sleep really waits,
    /// for a client that plays against a real server.
    pub fn follow_real_time(&self) {
        self.real_since.set(Some(Instant::now()));
    }

    pub fn advance(&self, seconds: f64) {
        self.now.set(self.now.get() + seconds);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> f64 {
        let real = self
            .real_since
            .get()
            .map_or(0.0, |since| since.elapsed().as_secs_f64());
        self.now.get() + real
    }

    fn sleep(&self, seconds: f64) {
        if self.real_since.get().is_some() {
            if seconds > 0.0 {
                std::thread::sleep(Duration::from_secs_f64(seconds));
            }
        } else {
            self.advance(seconds.max(0.0));
        }
    }
}
