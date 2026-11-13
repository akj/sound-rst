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

/// A clock that only moves when something sleeps or a test advances it.
#[derive(Default)]
pub struct ManualClock {
    now: Cell<f64>,
    real_pause: Cell<f64>,
}

impl ManualClock {
    pub fn new() -> Self {
        Self::default()
    }

    /// Each sleep also waits this long in real time, so that a client
    /// that talks to a real server does not spin.
    pub fn set_real_pause(&self, seconds: f64) {
        self.real_pause.set(seconds);
    }

    pub fn advance(&self, seconds: f64) {
        self.now.set(self.now.get() + seconds);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> f64 {
        self.now.get()
    }

    fn sleep(&self, seconds: f64) {
        self.advance(seconds.max(0.0));
        let pause = self.real_pause.get();
        if pause > 0.0 {
            std::thread::sleep(Duration::from_secs_f64(pause));
        }
    }
}
