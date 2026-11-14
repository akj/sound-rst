//! Speech: through the running screen reader, else the system voice.
//!
//! Screen reader calls run on a thread of their own ([`worker`]). Most
//! screen readers do not say when they have finished speaking, so the end
//! of an utterance is estimated from its length, as upstream did. NVDA
//! called directly ([`nvda`]) says when speech has finished or was
//! cancelled, and that report replaces the estimate.

mod nvda;
mod worker;

pub use nvda::{Controller, DirectSpeaker, ERROR_CANCELLED, ERROR_SUCCESS, RPC_S_UNKNOWN_IF, ssml};
pub use worker::{Command, Link, Report, Speaker, SpeechThread};

use std::cell::Cell;
use std::rc::Rc;

use crate::clock::ClockRef;
use crate::log::Log;

/// How an utterance ended, when the screen reader can tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeechEnd {
    /// It was said to the end.
    Finished,
    /// It was silenced by something other than the game: the player's
    /// key at the screen reader.
    Cancelled,
}

/// What a backend knows of the last utterance it was given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Progress {
    /// It cannot tell: the end is estimated.
    #[default]
    Unknown,
    /// It is being said, and the backend will report its end.
    Speaking,
    Ended(SpeechEnd),
}

/// Where speech goes.
pub trait SpeechBackend {
    /// Says `text`, interrupting what was being said.
    fn speak(&mut self, text: &str) -> Result<(), String>;
    /// Silences speech.
    fn stop(&mut self) -> Result<(), String>;
    /// What is known of the last utterance given to `speak`. Only
    /// meaningful until the next `speak` or `stop`.
    fn progress(&self) -> Progress {
        Progress::Unknown
    }
}

/// A reported end is waited for until three times the estimate and two
/// seconds more. Past that, the speech counts as ended: a screen reader
/// that drops the text (NVDA with speech off) never reports, and the voice
/// must not wait for ever.
const REPORT_WAIT_FACTOR: i64 = 3;
const REPORT_WAIT_EXTRA_US: i64 = 2_000_000;

/// The utterance in progress, in whole microseconds of the clock.
#[derive(Clone, Copy)]
struct Said {
    /// The estimated end.
    end_us: i64,
    /// The latest end a backend's report is waited for.
    limit_us: i64,
}

/// How long the screen reader takes to say one character, in seconds.
/// Clones share the value, so that a new measurement applies to the speech
/// at once.
#[derive(Clone, Debug)]
pub struct CharDelay(Rc<Cell<f64>>);

impl CharDelay {
    pub fn new(seconds: f64) -> CharDelay {
        CharDelay(Rc::new(Cell::new(seconds)))
    }

    pub fn get(&self) -> f64 {
        self.0.get()
    }

    pub fn set(&self, seconds: f64) {
        self.0.set(seconds);
    }
}

/// Speech with an end reported by the backend, or else estimated: each
/// character takes `wait_delay_per_character` seconds, a digit
/// `tts_digit_coefficient` times as long.
pub struct Tts {
    backend: Box<dyn SpeechBackend>,
    clock: ClockRef,
    log: Log,
    wait_delay_per_character: CharDelay,
    digit_coefficient: f64,
    said: Option<Said>,
}

fn microseconds(seconds: f64) -> i64 {
    (seconds * 1e6).round() as i64
}

impl Tts {
    pub fn new(
        backend: Box<dyn SpeechBackend>,
        clock: ClockRef,
        log: Log,
        wait_delay_per_character: CharDelay,
        digit_coefficient: f64,
    ) -> Tts {
        Tts {
            backend,
            clock,
            log,
            wait_delay_per_character,
            digit_coefficient,
            said: None,
        }
    }

    /// How long `text` is expected to take, in microseconds: the
    /// characters, then the extra time of the digits, each rounded.
    pub fn estimate_us(&self, text: &str) -> i64 {
        let chars = text.chars().count() as f64;
        let digits = text.chars().filter(|c| c.is_numeric()).count() as f64;
        let delay = self.wait_delay_per_character.get();
        microseconds(chars * delay) + microseconds(digits * (self.digit_coefficient - 1.0) * delay)
    }

    /// How long `text` is expected to take, in seconds.
    pub fn estimate(&self, text: &str) -> f64 {
        self.estimate_us(text) as f64 / 1e6
    }

    pub fn speak(&mut self, text: &str) {
        if let Err(e) = self.backend.speak(text) {
            self.log.error(format!("error during speak({text:?}): {e}"));
        }
        let now = microseconds(self.clock.now());
        let estimate = self.estimate_us(text);
        self.said = Some(Said {
            end_us: now + estimate,
            limit_us: now + estimate * REPORT_WAIT_FACTOR + REPORT_WAIT_EXTRA_US,
        });
    }

    /// Silences speech. The stop always reaches the screen reader: the
    /// estimate may say speech has ended when it has not.
    pub fn stop(&mut self) {
        // An error here shows again at the next speak.
        let _ = self.backend.stop();
        self.said = None;
    }

    pub fn is_speaking(&self) -> bool {
        let Some(said) = self.said else {
            return false;
        };
        let now = microseconds(self.clock.now());
        match self.backend.progress() {
            Progress::Unknown => said.end_us > now,
            Progress::Speaking => said.limit_us > now,
            Progress::Ended(_) => false,
        }
    }

    /// Whether the last utterance was silenced at the screen reader, and
    /// not by the game.
    pub fn was_cancelled(&self) -> bool {
        self.said.is_some() && self.backend.progress() == Progress::Ended(SpeechEnd::Cancelled)
    }
}

/// What the probe reports about speech.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeechReport {
    pub backend: String,
    /// The screen reader in use, or the system voice.
    pub output: Option<String>,
    pub notes: Vec<String>,
}

#[cfg(windows)]
pub use windows_speech::{probe_speech, screen_reader_speech};

#[cfg(windows)]
mod windows_speech {
    use std::sync::Arc;
    use std::sync::mpsc::Sender;

    use super::nvda::{CLIENT_DLL, NvdaDll};
    use super::{DirectSpeaker, Report, Speaker, SpeechReport, SpeechThread};
    use crate::log::Log;

    /// Speech through Tolk: JAWS, NVDA and other screen readers, with SAPI
    /// as the fallback when none runs, on the speech thread. Tolk loads
    /// `nvdaControllerClient64.dll` from the executable's folder. With
    /// `nvda_direct`, speech goes to NVDA through its own controller client
    /// first, and Tolk is the fallback.
    pub fn screen_reader_speech(nvda_direct: bool, log: Log) -> SpeechThread {
        // Tolk is made on the speech thread, and called only from it.
        SpeechThread::spawn(nvda_direct, move |link| {
            let tolk = Box::new(TolkSpeaker::new(link.reports.clone()));
            if !nvda_direct {
                return tolk as Box<dyn Speaker>;
            }
            let dll = std::env::current_exe()
                .map_err(|e| e.to_string())
                .and_then(|exe| {
                    let path = exe.with_file_name(CLIENT_DLL);
                    NvdaDll::load(&path).map(|dll| Arc::new(dll) as _)
                });
            DirectSpeaker::choose(dll, tolk, link, log)
        })
    }

    /// The screen reader Tolk speaks through ("SAPI" for the system
    /// voice), without saying anything.
    fn detect() -> Option<String> {
        let tolk = tolk::Tolk::new();
        tolk.try_sapi(true);
        // SAFETY: Tolk is loaded while `tolk` lives. It returns null or a
        // pointer to a static, nul-terminated wide string naming the
        // active driver.
        unsafe {
            let p = tolk_sys::Tolk_DetectScreenReader();
            if p.is_null() {
                return None;
            }
            let mut len = 0;
            while *p.add(len) != 0 && len < 256 {
                len += 1;
            }
            let units = std::slice::from_raw_parts(p.cast::<u16>(), len);
            Some(String::from_utf16_lossy(units))
        }
    }

    /// Tolk on the speech thread. It cannot tell when speech ends.
    struct TolkSpeaker {
        tolk: Arc<tolk::Tolk>,
        reports: Sender<Report>,
    }

    impl TolkSpeaker {
        fn new(reports: Sender<Report>) -> TolkSpeaker {
            let tolk = tolk::Tolk::new();
            tolk.try_sapi(true);
            tolk.prefer_sapi(false);
            TolkSpeaker { tolk, reports }
        }
    }

    impl Speaker for TolkSpeaker {
        fn speak(&mut self, id: u64, text: &str) {
            self.tolk.output(text, true);
            let _ = self.reports.send((id, None));
        }

        fn stop(&mut self) {
            self.tolk.silence();
        }
    }

    pub fn probe_speech() -> SpeechReport {
        let mut notes = Vec::new();
        let folder = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|d| d.to_path_buf()));
        let file = |name: &str| {
            folder
                .as_ref()
                .map(|d| d.join(name))
                .filter(|p| p.is_file())
        };
        match file("nvdaControllerClient64.dll") {
            Some(p) => notes.push(format!("NVDA controller client: {}", p.display())),
            None => notes.push(
                "NVDA controller client: nvdaControllerClient64.dll is not next to the program; \
                 NVDA cannot be used"
                    .to_owned(),
            ),
        }
        match file(CLIENT_DLL) {
            Some(p) => notes.push(format!(
                "NVDA controller client for nvda_direct: {}",
                p.display()
            )),
            None => notes.push(format!(
                "NVDA controller client for nvda_direct: {CLIENT_DLL} is not next to the \
                 program; nvda_direct = 1 falls back to Tolk"
            )),
        }
        SpeechReport {
            backend: "Tolk (screen reader, else SAPI)".to_owned(),
            output: detect(),
            notes,
        }
    }
}

#[cfg(not(windows))]
pub use other_speech::{probe_speech, screen_reader_speech};

#[cfg(not(windows))]
mod other_speech {
    use std::sync::mpsc::Sender;

    use super::{Report, Speaker, SpeechReport, SpeechThread};
    use crate::log::Log;

    /// Speech through the system's speech service, on the speech thread.
    /// `nvda_direct` has no effect here.
    pub fn screen_reader_speech(_nvda_direct: bool, log: Log) -> SpeechThread {
        SpeechThread::spawn(false, move |link| {
            Box::new(SystemSpeaker {
                tts: tts::Tts::default().ok(),
                reports: link.reports,
                log,
            }) as Box<dyn Speaker>
        })
    }

    struct SystemSpeaker {
        tts: Option<tts::Tts>,
        reports: Sender<Report>,
        log: Log,
    }

    impl Speaker for SystemSpeaker {
        fn speak(&mut self, id: u64, text: &str) {
            let result = match self.tts.as_mut() {
                Some(t) => t.speak(text, true).map(|_| ()).map_err(|e| e.to_string()),
                None => Err("no speech service".to_owned()),
            };
            if let Err(e) = result {
                self.log.error(format!("error during speak({text:?}): {e}"));
            }
            let _ = self.reports.send((id, None));
        }

        fn stop(&mut self) {
            if let Some(t) = self.tts.as_mut() {
                let _ = t.stop();
            }
        }
    }

    pub fn probe_speech() -> SpeechReport {
        let ok = tts::Tts::default().is_ok();
        SpeechReport {
            backend: "tts crate (system speech service)".to_owned(),
            output: ok.then(|| "system speech".to_owned()),
            notes: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualClock;
    use crate::testkit::{RecordingSpeech, SpeechEvent};
    use std::rc::Rc;

    #[test]
    fn the_end_of_speech_is_estimated_from_characters_and_digits() {
        let clock = Rc::new(ManualClock::new());
        let (backend, _) = RecordingSpeech::new();
        let mut tts = Tts::new(
            Box::new(backend),
            clock.clone(),
            Log::silent(),
            CharDelay::new(0.1),
            3.0,
        );
        assert!((tts.estimate("ab12") - 0.8).abs() < 1e-9);
        tts.speak("ab12");
        clock.advance(0.79);
        assert!(tts.is_speaking());
        clock.advance(0.02);
        assert!(!tts.is_speaking());
    }

    #[test]
    fn stop_reaches_the_screen_reader_when_the_estimate_says_speech_is_over() {
        let clock = Rc::new(ManualClock::new());
        let (backend, log) = RecordingSpeech::new();
        let mut tts = Tts::new(
            Box::new(backend),
            clock.clone(),
            Log::silent(),
            CharDelay::new(0.1),
            3.0,
        );
        tts.speak("ab");
        clock.advance(1.0);
        assert!(!tts.is_speaking());
        tts.stop();
        assert_eq!(
            log.events(),
            vec![SpeechEvent::Speak("ab".into()), SpeechEvent::Stop]
        );
    }

    #[test]
    fn a_reported_end_replaces_the_estimate() {
        let clock = Rc::new(ManualClock::new());
        let (backend, log) = RecordingSpeech::new();
        let mut tts = Tts::new(
            Box::new(backend),
            clock.clone(),
            Log::silent(),
            CharDelay::new(0.1),
            3.0,
        );
        // Ended before the estimate.
        tts.speak("abcdefghij");
        log.report(Progress::Ended(SpeechEnd::Finished));
        assert!(!tts.is_speaking());
        assert!(!tts.was_cancelled());
        // Still speaking past the estimate, up to three times it and two
        // seconds.
        tts.speak("abcdefghij");
        log.report(Progress::Speaking);
        clock.advance(2.0);
        assert!(tts.is_speaking());
        clock.advance(2.9);
        assert!(tts.is_speaking());
        clock.advance(0.2);
        assert!(!tts.is_speaking());
        // Cancelled at the screen reader.
        tts.speak("abc");
        log.report(Progress::Ended(SpeechEnd::Cancelled));
        assert!(!tts.is_speaking());
        assert!(tts.was_cancelled());
        // A stop by the game is not a cancel at the screen reader.
        tts.stop();
        assert!(!tts.was_cancelled());
    }
}
