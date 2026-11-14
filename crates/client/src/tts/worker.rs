//! The speech thread: screen reader calls run there, so a slow one never
//! delays a key or a sound. The game's side sends commands and reads end
//! reports; it never waits for the thread.

use std::cell::Cell;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use super::{Progress, SpeechBackend, SpeechEnd};

/// What the speech thread is asked to do.
pub enum Command {
    /// Say the text as the utterance of this id, interrupting.
    Speak(u64, String),
    Stop,
    /// A call that waited for the end of utterance `id` returned this
    /// Windows error code.
    Returned(u64, u32),
    Quit,
}

/// How utterance `id` ended; `None` when the screen reader cannot tell.
pub type Report = (u64, Option<SpeechEnd>);

/// A screen reader as the speech thread drives it.
pub trait Speaker {
    /// Says `text` as utterance `id`, interrupting what was being said,
    /// and reports its end.
    fn speak(&mut self, id: u64, text: &str);
    fn stop(&mut self);
    /// A call that waited for the end of utterance `id` returned `code`.
    fn returned(&mut self, _id: u64, _code: u32) {}
    /// Called when `wait` has passed with no command.
    fn tick(&mut self) {}
    /// How long to wait for a command before calling `tick`; `None` to
    /// wait for the next command.
    fn wait(&self) -> Option<Duration> {
        None
    }
}

/// The speech thread's channels, given to the speaker it runs.
pub struct Link {
    /// The thread's own commands, for calls that report from other
    /// threads.
    pub commands: Sender<Command>,
    pub reports: Sender<Report>,
}

/// The game's side of the speech thread.
pub struct SpeechThread {
    commands: Sender<Command>,
    reports: Receiver<Report>,
    /// The id of the last command: utterance ids grow, and a stop takes an
    /// id too, so a late report of an older utterance is ignored.
    last: u64,
    progress: Cell<Progress>,
    /// Whether the speaker can report ends. If so, each utterance is
    /// speaking until its own report comes, which may say it cannot tell.
    reporting: bool,
}

impl SpeechThread {
    /// Starts the speech thread with the speaker that `make` returns,
    /// made on that thread. `reporting`: whether the speaker can report
    /// how utterances end.
    pub fn spawn(
        reporting: bool,
        make: impl FnOnce(Link) -> Box<dyn Speaker> + Send + 'static,
    ) -> SpeechThread {
        let (commands, received) = mpsc::channel();
        let (report, reports) = mpsc::channel();
        let link = Link {
            commands: commands.clone(),
            reports: report,
        };
        // If the thread cannot start, every command fails and says so.
        let _ = std::thread::Builder::new()
            .name("speech".to_owned())
            .spawn(move || {
                let mut speaker = make(link);
                serve(&received, speaker.as_mut());
            });
        SpeechThread {
            commands,
            reports,
            last: 0,
            progress: Cell::new(Progress::Unknown),
            reporting,
        }
    }

    fn send(&self, command: Command) -> Result<(), String> {
        self.commands
            .send(command)
            .map_err(|_| "the speech thread has stopped".to_owned())
    }
}

impl SpeechBackend for SpeechThread {
    fn speak(&mut self, text: &str) -> Result<(), String> {
        self.last += 1;
        self.progress.set(if self.reporting {
            Progress::Speaking
        } else {
            Progress::Unknown
        });
        self.send(Command::Speak(self.last, text.to_owned()))
    }

    fn stop(&mut self) -> Result<(), String> {
        self.last += 1;
        self.progress.set(Progress::Unknown);
        self.send(Command::Stop)
    }

    fn progress(&self) -> Progress {
        while let Ok((id, end)) = self.reports.try_recv() {
            if id == self.last {
                self.progress
                    .set(end.map_or(Progress::Unknown, Progress::Ended));
            }
        }
        self.progress.get()
    }
}

impl Drop for SpeechThread {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Quit);
    }
}

/// Runs `speaker` on the commands until `Quit`. Of the commands waiting,
/// only the last `Speak` and those after it are carried out, with every
/// `Returned`: an utterance already replaced need not be said.
fn serve(commands: &Receiver<Command>, speaker: &mut dyn Speaker) {
    loop {
        let first = match speaker.wait() {
            Some(wait) => match commands.recv_timeout(wait) {
                Ok(c) => c,
                Err(RecvTimeoutError::Timeout) => {
                    speaker.tick();
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            },
            None => match commands.recv() {
                Ok(c) => c,
                Err(_) => return,
            },
        };
        let batch: Vec<Command> = std::iter::once(first).chain(commands.try_iter()).collect();
        let last_speak = batch
            .iter()
            .rposition(|c| matches!(c, Command::Speak(..)))
            .unwrap_or(0);
        for (i, command) in batch.into_iter().enumerate() {
            match command {
                Command::Returned(id, code) => speaker.returned(id, code),
                Command::Quit => return,
                _ if i < last_speak => {}
                Command::Speak(id, text) => speaker.speak(id, &text),
                Command::Stop => speaker.stop(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    /// Waits up to two seconds of real time for `done`.
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

    /// Records what it is asked; reports a finished end for each
    /// utterance if `reports`.
    struct Fake {
        seen: Arc<Mutex<Vec<String>>>,
        link: Link,
        reports: bool,
    }

    impl Speaker for Fake {
        fn speak(&mut self, id: u64, text: &str) {
            self.seen.lock().unwrap().push(format!("speak {text}"));
            let end = self.reports.then_some(SpeechEnd::Finished);
            let _ = self.link.reports.send((id, end));
        }

        fn stop(&mut self) {
            self.seen.lock().unwrap().push("stop".to_owned());
        }
    }

    fn fake(reports: bool) -> (SpeechThread, Arc<Mutex<Vec<String>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        let thread = SpeechThread::spawn(reports, move |link| {
            Box::new(Fake {
                seen: s,
                link,
                reports,
            })
        });
        (thread, seen)
    }

    #[test]
    fn commands_reach_the_speaker_in_order_on_its_thread() {
        let (mut thread, seen) = fake(false);
        thread.speak("a").unwrap();
        thread.stop().unwrap();
        assert!(eventually(|| seen.lock().unwrap().len() == 2));
        assert_eq!(*seen.lock().unwrap(), ["speak a", "stop"]);
        // A speaker that cannot tell leaves the end to the estimate.
        assert_eq!(thread.progress(), Progress::Unknown);
    }

    #[test]
    fn a_reported_end_is_the_progress_of_the_last_utterance() {
        let (mut thread, _seen) = fake(true);
        thread.speak("a").unwrap();
        assert!(eventually(
            || thread.progress() == Progress::Ended(SpeechEnd::Finished)
        ));
        // A stop forgets it: the end of a stopped utterance is not
        // reported as the end of anything newer.
        thread.stop().unwrap();
        assert_eq!(thread.progress(), Progress::Unknown);
    }
}
