//! NVDA through its own controller client, which says when speech ends.
//!
//! `nvdaController_speakSsml`, called with `asynchronous` false, returns
//! `ERROR_SUCCESS` when the speech is completed and `ERROR_CANCELLED` when
//! it is cancelled (NVDA 2024.1 and later; older versions answer
//! `RPC_S_UNKNOWN_IF`). Each call waits on a thread of its own, so the
//! speech thread stays free to cancel it.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{Command, Link, Speaker, SpeechEnd};
use crate::log::Log;

pub const ERROR_SUCCESS: u32 = 0;
pub const ERROR_CANCELLED: u32 = 1223;
/// What NVDA older than 2024.1 answers to `speakSsml`.
pub const RPC_S_UNKNOWN_IF: u32 = 1717;

/// The controller client NV Access ships since NVDA 2024.1, by its own
/// name. Tolk's older `nvdaControllerClient64.dll` lacks `speakSsml`.
#[cfg(windows)]
pub const CLIENT_DLL: &str = "nvdaControllerClient.dll";

/// While a cancelled call has not returned, the cancel is sent again this
/// often: a cancel that reached NVDA before the speech did cancels
/// nothing.
const RETRY: Duration = Duration::from_millis(100);
/// A cancelled call that has not returned after this long is given up:
/// NVDA drops speech when its speech mode is off, and then neither ends
/// nor cancels it.
const GRACE: Duration = Duration::from_secs(2);
/// After this many calls given up, NVDA is not called directly again.
const MAX_ABANDONED: usize = 3;

/// The two calls of NVDA's controller client the game makes. The real one
/// is the DLL; tests use a fake.
pub trait Controller: Send + Sync {
    /// `nvdaController_speakSsml(ssml, SYMBOL_LEVEL_UNCHANGED,
    /// SPEECH_PRIORITY_NORMAL, FALSE)`: returns when the speech is
    /// completed or cancelled, with a Windows error code.
    fn speak_ssml(&self, ssml: &str) -> u32;
    /// `nvdaController_cancelSpeech()`. Called from another thread while
    /// `speak_ssml` waits.
    fn cancel_speech(&self) -> u32;
}

/// `text` as SSML: the characters XML gives a meaning escaped, those it
/// forbids made spaces. `None` when nothing would be said: NVDA never ends
/// an empty speech sequence.
pub fn ssml(text: &str) -> Option<String> {
    let mut body = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => body.push_str("&amp;"),
            '<' => body.push_str("&lt;"),
            '>' => body.push_str("&gt;"),
            '"' => body.push_str("&quot;"),
            '\'' => body.push_str("&apos;"),
            '\t' | '\n' | '\r' => body.push(c),
            '\u{0}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}' => body.push(' '),
            _ => body.push(c),
        }
    }
    (!body.trim().is_empty()).then(|| format!("<speak>{body}</speak>"))
}

/// A call to `speak_ssml` that has not returned.
struct Call {
    id: u64,
    text: String,
    /// When it was cancelled, if it was.
    cancelled: Option<Instant>,
}

/// Speech through NVDA's controller client, with another speaker (Tolk)
/// for what NVDA cannot take.
pub struct DirectSpeaker {
    nvda: Arc<dyn Controller>,
    fallback: Box<dyn Speaker>,
    link: Link,
    log: Log,
    /// False once NVDA is known not to take SSML: everything then goes
    /// to the fallback.
    direct: bool,
    calling: Option<Call>,
    /// The utterance to say once the current call has returned.
    next: Option<(u64, String)>,
    abandoned: usize,
    grace: Duration,
    /// The error codes already logged.
    logged: Vec<u32>,
}

impl DirectSpeaker {
    /// The direct speaker if the controller client could be loaded, else
    /// the fallback alone.
    pub fn choose(
        nvda: Result<Arc<dyn Controller>, String>,
        fallback: Box<dyn Speaker>,
        link: Link,
        log: Log,
    ) -> Box<dyn Speaker> {
        match nvda {
            Ok(nvda) => {
                log.info("speech: NVDA's controller client is called directly");
                Box::new(DirectSpeaker::new(nvda, fallback, link, log))
            }
            Err(e) => {
                log.warning(format!("speech: through Tolk only: {e}"));
                fallback
            }
        }
    }

    fn new(
        nvda: Arc<dyn Controller>,
        fallback: Box<dyn Speaker>,
        link: Link,
        log: Log,
    ) -> DirectSpeaker {
        DirectSpeaker {
            nvda,
            fallback,
            link,
            log,
            direct: true,
            calling: None,
            next: None,
            abandoned: 0,
            grace: GRACE,
            logged: Vec::new(),
        }
    }

    /// Cancels NVDA's speech, and the fallback's, which said what NVDA
    /// could not.
    fn cancel(&mut self) {
        self.nvda.cancel_speech();
        self.fallback.stop();
        if let Some(call) = &mut self.calling {
            call.cancelled.get_or_insert_with(Instant::now);
        }
    }

    /// Says the next utterance, if any; the previous call has returned.
    fn start_next(&mut self) {
        let Some((id, text)) = self.next.take() else {
            return;
        };
        let ssml = match ssml(&text) {
            Some(ssml) if self.direct => ssml,
            _ => return self.fallback.speak(id, &text),
        };
        let nvda = self.nvda.clone();
        let commands = self.link.commands.clone();
        let spawned = std::thread::Builder::new()
            .name("speech call".to_owned())
            .spawn(move || {
                let code = nvda.speak_ssml(&ssml);
                let _ = commands.send(Command::Returned(id, code));
            });
        match spawned {
            Ok(_) => {
                self.calling = Some(Call {
                    id,
                    text,
                    cancelled: None,
                })
            }
            Err(_) => self.fallback.speak(id, &text),
        }
    }

    fn log_once(&mut self, code: u32, message: String) {
        if !self.logged.contains(&code) {
            self.logged.push(code);
            self.log.warning(message);
        }
    }
}

impl Speaker for DirectSpeaker {
    fn speak(&mut self, id: u64, text: &str) {
        if !self.direct {
            return self.fallback.speak(id, text);
        }
        self.next = Some((id, text.to_owned()));
        self.cancel();
        if self.calling.is_none() {
            self.start_next();
        }
    }

    fn stop(&mut self) {
        self.next = None;
        self.cancel();
    }

    fn returned(&mut self, id: u64, code: u32) {
        let call = match self.calling.take() {
            Some(call) if call.id == id => call,
            // A call given up on has come back.
            other => {
                self.calling = other;
                return;
            }
        };
        let end = match code {
            ERROR_SUCCESS => Some(SpeechEnd::Finished),
            ERROR_CANCELLED => Some(SpeechEnd::Cancelled),
            RPC_S_UNKNOWN_IF => {
                self.direct = false;
                self.log_once(
                    code,
                    "speech: NVDA is older than 2024.1 and cannot say when speech ends; \
                     speaking through Tolk"
                        .to_owned(),
                );
                None
            }
            _ => {
                self.log_once(
                    code,
                    format!("speech: NVDA's speakSsml failed with error {code}; using Tolk"),
                );
                None
            }
        };
        match end {
            Some(end) => {
                let _ = self.link.reports.send((id, Some(end)));
            }
            // Not said: the fallback says it, unless it was replaced or
            // stopped meanwhile.
            None if call.cancelled.is_none() => self.fallback.speak(id, &call.text),
            None => {}
        }
        self.start_next();
    }

    fn tick(&mut self) {
        let Some(cancelled) = self.calling.as_ref().and_then(|c| c.cancelled) else {
            return;
        };
        if cancelled.elapsed() < self.grace {
            self.nvda.cancel_speech();
            return;
        }
        // The call may never return; its thread is left waiting.
        self.calling = None;
        self.abandoned += 1;
        self.log.warning(format!(
            "speech: NVDA did not end a cancelled utterance within {} s",
            self.grace.as_secs_f64()
        ));
        if self.abandoned >= MAX_ABANDONED {
            self.direct = false;
            self.log
                .warning("speech: NVDA is no longer called directly; speaking through Tolk");
        }
        self.start_next();
    }

    fn wait(&self) -> Option<Duration> {
        self.calling
            .as_ref()
            .and_then(|c| c.cancelled)
            .map(|_| RETRY)
    }
}

#[cfg(windows)]
pub use dll::NvdaDll;

/// The controller client DLL, loaded at run time. All the unsafe code of
/// the direct route is here.
#[cfg(windows)]
mod dll {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

    use super::Controller;

    /// `error_status_t __stdcall nvdaController_speakSsml(const wchar_t*
    /// ssml, SYMBOL_LEVEL, SPEECH_PRIORITY, boolean asynchronous)`. Both
    /// enums are `v1_enum` (32 bits); `boolean` is an unsigned char.
    type SpeakSsml = unsafe extern "system" fn(*const u16, i32, i32, u8) -> u32;
    /// `error_status_t __stdcall nvdaController_cancelSpeech(void)`.
    type CancelSpeech = unsafe extern "system" fn() -> u32;

    const SYMBOL_LEVEL_UNCHANGED: i32 = -1;
    const SPEECH_PRIORITY_NORMAL: i32 = 0;

    /// NVDA's controller client. The DLL is never unloaded: a call may
    /// still be waiting inside it when the game ends.
    pub struct NvdaDll {
        speak_ssml: SpeakSsml,
        cancel_speech: CancelSpeech,
    }

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    impl NvdaDll {
        /// Loads the DLL at `path` and finds its two functions. Fails if
        /// either is missing, as in clients older than NVDA 2024.1.
        pub fn load(path: &Path) -> Result<NvdaDll, String> {
            if !path.is_file() {
                return Err(format!("{} is missing", path.display()));
            }
            let name = wide(path.as_os_str());
            // SAFETY: `name` is a nul-terminated wide path that outlives
            // the call. Loading runs the DLL's DllMain, which only makes
            // RPC binding handles (nvdaHelper/client/client.cpp).
            let module = unsafe { LoadLibraryW(name.as_ptr()) };
            if module == 0 {
                // SAFETY: reads the calling thread's last error; no
                // arguments.
                let code = unsafe { GetLastError() };
                return Err(format!("cannot load {}: error {code}", path.display()));
            }
            let find = |symbol: &[u8]| {
                // SAFETY: `module` is loaded and never freed; `symbol` is
                // a nul-terminated ASCII name.
                unsafe { GetProcAddress(module, symbol.as_ptr()) }.ok_or_else(|| {
                    let name = String::from_utf8_lossy(&symbol[..symbol.len() - 1]);
                    format!(
                        "{} has no {name}: it is older than NVDA 2024.1",
                        path.display()
                    )
                })
            };
            let speak = find(b"nvdaController_speakSsml\0")?;
            let cancel = find(b"nvdaController_cancelSpeech\0")?;
            // SAFETY: the exports have the signatures of `SpeakSsml` and
            // `CancelSpeech` (nvdaController.h of the client 2.0 and
            // later), and the module stays loaded for the process's life.
            let (speak_ssml, cancel_speech) = unsafe {
                (
                    std::mem::transmute::<unsafe extern "system" fn() -> isize, SpeakSsml>(speak),
                    std::mem::transmute::<unsafe extern "system" fn() -> isize, CancelSpeech>(
                        cancel,
                    ),
                )
            };
            Ok(NvdaDll {
                speak_ssml,
                cancel_speech,
            })
        }
    }

    impl Controller for NvdaDll {
        fn speak_ssml(&self, ssml: &str) -> u32 {
            let text: Vec<u16> = ssml.encode_utf16().chain(std::iter::once(0)).collect();
            // SAFETY: `text` is nul-terminated and outlives the call, which
            // only reads it. The RPC stub may be called from any thread.
            unsafe {
                (self.speak_ssml)(
                    text.as_ptr(),
                    SYMBOL_LEVEL_UNCHANGED,
                    SPEECH_PRIORITY_NORMAL,
                    0,
                )
            }
        }

        fn cancel_speech(&self) -> u32 {
            // SAFETY: no arguments; the RPC stub may be called from any
            // thread, also while another thread waits in `speak_ssml`.
            unsafe { (self.cancel_speech)() }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tts::{Progress, SpeechBackend, SpeechThread};
    use std::sync::{Condvar, Mutex};

    fn eventually(mut done: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(2) {
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        done()
    }

    #[derive(Default)]
    struct FakeState {
        /// What was asked, in order: "ssml <ssml>" or "cancel".
        log: Vec<String>,
        /// Each call's result, once it has one.
        results: Vec<Option<u32>>,
        /// A code every call returns at once, without speaking.
        immediate: Option<u32>,
        /// Whether a cancel makes the waiting calls return
        /// `ERROR_CANCELLED`, as NVDA does when it speaks.
        cancel_releases: bool,
    }

    /// NVDA as the controller client's callers see it.
    #[derive(Default)]
    struct FakeNvda {
        state: Mutex<FakeState>,
        changed: Condvar,
    }

    impl FakeNvda {
        fn new(cancel_releases: bool, immediate: Option<u32>) -> Arc<FakeNvda> {
            let fake = FakeNvda::default();
            {
                let mut s = fake.state.lock().unwrap();
                s.cancel_releases = cancel_releases;
                s.immediate = immediate;
            }
            Arc::new(fake)
        }

        /// Ends call `index` with `code`.
        fn release(&self, index: usize, code: u32) {
            self.state.lock().unwrap().results[index] = Some(code);
            self.changed.notify_all();
        }

        fn calls(&self) -> Vec<String> {
            self.state
                .lock()
                .unwrap()
                .log
                .iter()
                .filter(|l| l.starts_with("ssml"))
                .cloned()
                .collect()
        }

        fn result(&self, index: usize) -> Option<u32> {
            self.state
                .lock()
                .unwrap()
                .results
                .get(index)
                .copied()
                .flatten()
        }
    }

    impl Controller for FakeNvda {
        fn speak_ssml(&self, ssml: &str) -> u32 {
            let mut s = self.state.lock().unwrap();
            s.log.push(format!("ssml {ssml}"));
            if let Some(code) = s.immediate {
                return code;
            }
            let index = s.results.len();
            s.results.push(None);
            self.changed.notify_all();
            loop {
                if let Some(code) = s.results[index] {
                    return code;
                }
                s = self.changed.wait(s).unwrap();
            }
        }

        fn cancel_speech(&self) -> u32 {
            let mut s = self.state.lock().unwrap();
            s.log.push("cancel".to_owned());
            if s.cancel_releases {
                for r in s.results.iter_mut().filter(|r| r.is_none()) {
                    *r = Some(ERROR_CANCELLED);
                }
                self.changed.notify_all();
            }
            ERROR_SUCCESS
        }
    }

    /// The fallback: records what it says, and cannot tell when speech
    /// ends.
    struct FakeTolk {
        said: Arc<Mutex<Vec<String>>>,
        reports: std::sync::mpsc::Sender<crate::tts::Report>,
    }

    impl Speaker for FakeTolk {
        fn speak(&mut self, id: u64, text: &str) {
            self.said.lock().unwrap().push(text.to_owned());
            let _ = self.reports.send((id, None));
        }

        fn stop(&mut self) {}
    }

    struct Setup {
        speech: SpeechThread,
        nvda: Arc<FakeNvda>,
        tolk: Arc<Mutex<Vec<String>>>,
    }

    fn setup(nvda: Arc<FakeNvda>, loaded: bool, grace: Duration) -> Setup {
        let tolk = Arc::new(Mutex::new(Vec::new()));
        let (n, t) = (nvda.clone(), tolk.clone());
        let speech = SpeechThread::spawn(true, move |link| {
            let fallback = Box::new(FakeTolk {
                said: t,
                reports: link.reports.clone(),
            });
            if !loaded {
                let missing = Err("no nvdaController_speakSsml".to_owned());
                return DirectSpeaker::choose(missing, fallback, link, Log::silent());
            }
            let mut direct = DirectSpeaker::new(n, fallback, link, Log::silent());
            direct.grace = grace;
            Box::new(direct)
        });
        Setup { speech, nvda, tolk }
    }

    fn said_by_tolk(s: &Setup) -> Vec<String> {
        s.tolk.lock().unwrap().clone()
    }

    #[test]
    fn text_is_escaped_into_ssml() {
        assert_eq!(
            ssml(r#"a < b & "c" 'd' > e"#).unwrap(),
            "<speak>a &lt; b &amp; &quot;c&quot; &apos;d&apos; &gt; e</speak>"
        );
        assert_eq!(ssml("x\u{1}y\u{ffff}").unwrap(), "<speak>x y </speak>");
        assert_eq!(ssml("é\n2"), Some("<speak>é\n2</speak>".to_owned()));
        assert_eq!(ssml(""), None);
        assert_eq!(ssml(" \u{2}"), None);
    }

    #[test]
    fn a_finished_report_ends_the_utterance() {
        let mut s = setup(FakeNvda::new(true, None), true, GRACE);
        s.speech.speak("a < b").unwrap();
        assert!(eventually(|| s.nvda.calls().len() == 1));
        assert_eq!(s.nvda.calls(), ["ssml <speak>a &lt; b</speak>"]);
        assert_eq!(s.speech.progress(), Progress::Speaking);
        s.nvda.release(0, ERROR_SUCCESS);
        assert!(eventually(
            || s.speech.progress() == Progress::Ended(SpeechEnd::Finished)
        ));
        assert!(said_by_tolk(&s).is_empty());
    }

    #[test]
    fn a_cancel_at_the_screen_reader_is_reported() {
        let mut s = setup(FakeNvda::new(true, None), true, GRACE);
        s.speech.speak("a").unwrap();
        assert!(eventually(|| s.nvda.calls().len() == 1));
        // The player's key at NVDA, not the game.
        s.nvda.release(0, ERROR_CANCELLED);
        assert!(eventually(
            || s.speech.progress() == Progress::Ended(SpeechEnd::Cancelled)
        ));
    }

    #[test]
    fn a_new_utterance_cancels_the_waiting_call_and_starts_without_its_end() {
        let mut s = setup(FakeNvda::new(true, None), true, GRACE);
        s.speech.speak("a").unwrap();
        assert!(eventually(|| s.nvda.calls().len() == 1));
        // "a" never ends by itself: only the cancel lets "b" start.
        s.speech.speak("b").unwrap();
        assert!(eventually(|| s.nvda.calls().len() == 2));
        assert_eq!(s.nvda.calls()[1], "ssml <speak>b</speak>");
        assert_eq!(s.nvda.result(0), Some(ERROR_CANCELLED));
        // The cancel of "a" is not the end of "b".
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(s.speech.progress(), Progress::Speaking);
        s.nvda.release(1, ERROR_SUCCESS);
        assert!(eventually(
            || s.speech.progress() == Progress::Ended(SpeechEnd::Finished)
        ));
    }

    #[test]
    fn a_late_return_of_a_stopped_utterance_does_not_end_the_next() {
        // A cancel does not release the call at once.
        let mut s = setup(FakeNvda::new(false, None), true, GRACE);
        s.speech.speak("a").unwrap();
        assert!(eventually(|| s.nvda.calls().len() == 1));
        s.speech.stop().unwrap();
        s.speech.speak("b").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(s.speech.progress(), Progress::Speaking);
        // "a" returns late, and only then "b" goes to NVDA.
        s.nvda.release(0, ERROR_SUCCESS);
        assert!(eventually(|| s.nvda.calls().len() == 2));
        assert_eq!(s.nvda.calls()[1], "ssml <speak>b</speak>");
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(s.speech.progress(), Progress::Speaking);
        s.nvda.release(1, ERROR_CANCELLED);
        assert!(eventually(
            || s.speech.progress() == Progress::Ended(SpeechEnd::Cancelled)
        ));
    }

    #[test]
    fn a_call_that_never_returns_is_given_up_and_then_nvda_is() {
        // NVDA with speech off: calls neither end nor cancel.
        let mut s = setup(FakeNvda::new(false, None), true, Duration::from_millis(50));
        s.speech.speak("a").unwrap();
        assert!(eventually(|| s.nvda.calls().len() == 1));
        for (n, text) in ["b", "c"].into_iter().enumerate() {
            s.speech.speak(text).unwrap();
            assert!(eventually(|| s.nvda.calls().len() == n + 2));
        }
        // The third call given up: Tolk says the rest.
        s.speech.speak("d").unwrap();
        assert!(eventually(|| said_by_tolk(&s) == ["d"]));
        assert_eq!(s.nvda.calls().len(), 3);
        assert!(eventually(|| s.speech.progress() == Progress::Unknown));
    }

    #[test]
    fn an_old_nvda_answering_1717_falls_back_to_tolk_for_good() {
        let mut s = setup(FakeNvda::new(true, Some(RPC_S_UNKNOWN_IF)), true, GRACE);
        s.speech.speak("a").unwrap();
        assert!(eventually(|| said_by_tolk(&s) == ["a"]));
        assert!(eventually(|| s.speech.progress() == Progress::Unknown));
        s.speech.speak("b").unwrap();
        assert!(eventually(|| said_by_tolk(&s) == ["a", "b"]));
        assert_eq!(s.nvda.calls().len(), 1);
        // The end of "b" is left to the estimate.
        assert!(eventually(|| s.speech.progress() == Progress::Unknown));
    }

    #[test]
    fn another_error_falls_back_for_that_utterance_only() {
        // RPC_S_SERVER_UNAVAILABLE: NVDA is not running.
        let mut s = setup(FakeNvda::new(true, Some(1722)), true, GRACE);
        s.speech.speak("a").unwrap();
        assert!(eventually(|| said_by_tolk(&s) == ["a"]));
        s.speech.speak("b").unwrap();
        assert!(eventually(|| said_by_tolk(&s) == ["a", "b"]));
        assert_eq!(s.nvda.calls().len(), 2);
    }

    #[test]
    fn a_client_without_speak_ssml_falls_back_to_tolk() {
        let mut s = setup(FakeNvda::new(true, None), false, GRACE);
        s.speech.speak("a").unwrap();
        assert!(eventually(|| said_by_tolk(&s) == ["a"]));
        assert!(s.nvda.calls().is_empty());
        assert!(eventually(|| s.speech.progress() == Progress::Unknown));
    }

    #[test]
    fn nothing_to_say_goes_to_tolk_and_not_to_a_waiting_call() {
        let mut s = setup(FakeNvda::new(true, None), true, GRACE);
        s.speech.speak("").unwrap();
        assert!(eventually(|| said_by_tolk(&s) == [""]));
        assert!(s.nvda.calls().is_empty());
        // The next utterance still waits for its own report.
        s.speech.speak("b").unwrap();
        assert!(eventually(|| s.nvda.calls().len() == 1));
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(s.speech.progress(), Progress::Speaking);
        s.nvda.release(0, ERROR_SUCCESS);
        assert!(eventually(
            || s.speech.progress() == Progress::Ended(SpeechEnd::Finished)
        ));
    }
}
