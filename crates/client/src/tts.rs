//! Speech: through the running screen reader, else the system voice.
//!
//! A screen reader does not say when it has finished speaking, so the end of
//! an utterance is estimated from its length, as upstream did.

use crate::clock::ClockRef;
use crate::log::Log;

/// Where speech goes.
pub trait SpeechBackend {
    /// Says `text`, interrupting what was being said.
    fn speak(&mut self, text: &str) -> Result<(), String>;
    /// Silences speech.
    fn stop(&mut self) -> Result<(), String>;
}

/// Speech with an estimated end: each character takes
/// `wait_delay_per_character` seconds, a digit `tts_digit_coefficient`
/// times as long.
pub struct Tts {
    backend: Box<dyn SpeechBackend>,
    clock: ClockRef,
    log: Log,
    wait_delay_per_character: f64,
    digit_coefficient: f64,
    end_time: Option<f64>,
}

impl Tts {
    pub fn new(
        backend: Box<dyn SpeechBackend>,
        clock: ClockRef,
        log: Log,
        wait_delay_per_character: f64,
        digit_coefficient: f64,
    ) -> Tts {
        Tts {
            backend,
            clock,
            log,
            wait_delay_per_character,
            digit_coefficient,
            end_time: None,
        }
    }

    /// How long `text` is expected to take, in seconds.
    pub fn estimate(&self, text: &str) -> f64 {
        let chars = text.chars().count() as f64;
        let digits = text.chars().filter(|c| c.is_numeric()).count() as f64;
        let delay = self.wait_delay_per_character;
        chars * delay + digits * (self.digit_coefficient - 1.0) * delay
    }

    pub fn speak(&mut self, text: &str) {
        if let Err(e) = self.backend.speak(text) {
            self.log.error(format!("error during speak({text:?}): {e}"));
        }
        self.end_time = Some(self.clock.now() + self.estimate(text));
    }

    /// Silences speech, unless the estimate says it has already ended.
    pub fn stop(&mut self) {
        if self.is_speaking() {
            // An error here shows again at the next speak.
            let _ = self.backend.stop();
            self.end_time = None;
        }
    }

    pub fn is_speaking(&self) -> bool {
        self.end_time.is_some_and(|end| end > self.clock.now())
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
pub use windows_speech::{ScreenReaderSpeech, probe_speech};

#[cfg(windows)]
mod windows_speech {
    use super::{SpeechBackend, SpeechReport};
    use std::sync::Arc;

    /// Speech through Tolk: JAWS, NVDA and other screen readers, with SAPI
    /// as the fallback when none runs. Tolk loads
    /// `nvdaControllerClient64.dll` from the executable's folder.
    pub struct ScreenReaderSpeech {
        tolk: Arc<tolk::Tolk>,
    }

    impl ScreenReaderSpeech {
        pub fn new() -> ScreenReaderSpeech {
            let tolk = tolk::Tolk::new();
            tolk.try_sapi(true);
            tolk.prefer_sapi(false);
            ScreenReaderSpeech { tolk }
        }

        /// The screen reader Tolk speaks through ("SAPI" for the system
        /// voice), without saying anything.
        pub fn detect(&self) -> Option<String> {
            // SAFETY: Tolk returns null or a pointer to a static,
            // nul-terminated wide string naming the active driver.
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
    }

    impl Default for ScreenReaderSpeech {
        fn default() -> Self {
            Self::new()
        }
    }

    impl SpeechBackend for ScreenReaderSpeech {
        fn speak(&mut self, text: &str) -> Result<(), String> {
            self.tolk.output(text, true);
            Ok(())
        }

        fn stop(&mut self) -> Result<(), String> {
            self.tolk.silence();
            Ok(())
        }
    }

    pub fn probe_speech() -> SpeechReport {
        let mut notes = Vec::new();
        let dll = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|d| d.join("nvdaControllerClient64.dll")));
        match dll {
            Some(p) if p.is_file() => {
                notes.push(format!("NVDA controller client: {}", p.display()))
            }
            _ => notes.push(
                "NVDA controller client: nvdaControllerClient64.dll is not next to the program; \
                 NVDA cannot be used"
                    .to_owned(),
            ),
        }
        let speech = ScreenReaderSpeech::new();
        SpeechReport {
            backend: "Tolk (screen reader, else SAPI)".to_owned(),
            output: speech.detect(),
            notes,
        }
    }
}

#[cfg(not(windows))]
pub use other_speech::{ScreenReaderSpeech, probe_speech};

#[cfg(not(windows))]
mod other_speech {
    use super::{SpeechBackend, SpeechReport};

    /// Speech through the system's speech service.
    pub struct ScreenReaderSpeech {
        tts: Option<tts::Tts>,
    }

    impl ScreenReaderSpeech {
        pub fn new() -> ScreenReaderSpeech {
            ScreenReaderSpeech {
                tts: tts::Tts::default().ok(),
            }
        }
    }

    impl Default for ScreenReaderSpeech {
        fn default() -> Self {
            Self::new()
        }
    }

    impl SpeechBackend for ScreenReaderSpeech {
        fn speak(&mut self, text: &str) -> Result<(), String> {
            match self.tts.as_mut() {
                Some(t) => t.speak(text, true).map(|_| ()).map_err(|e| e.to_string()),
                None => Err("no speech service".to_owned()),
            }
        }

        fn stop(&mut self) -> Result<(), String> {
            match self.tts.as_mut() {
                Some(t) => t.stop().map(|_| ()).map_err(|e| e.to_string()),
                None => Ok(()),
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
        let (backend, log) = RecordingSpeech::new();
        let mut tts = Tts::new(Box::new(backend), clock.clone(), Log::silent(), 0.1, 3.0);
        assert!((tts.estimate("ab12") - 0.8).abs() < 1e-9);
        tts.speak("ab12");
        clock.advance(0.79);
        assert!(tts.is_speaking());
        clock.advance(0.02);
        assert!(!tts.is_speaking());
        // Past the estimate, stop does not reach the screen reader.
        tts.stop();
        assert_eq!(log.events(), vec![SpeechEvent::Speak("ab12".into())]);
        tts.speak("x");
        tts.stop();
        assert_eq!(
            log.events(),
            vec![
                SpeechEvent::Speak("ab12".into()),
                SpeechEvent::Speak("x".into()),
                SpeechEvent::Stop
            ]
        );
    }
}
