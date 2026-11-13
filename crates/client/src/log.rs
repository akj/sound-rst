//! The client log: warnings go to `<user>/tmp/client.log`, and to standard
//! error. With `debug_mode`, every message the voice says is written as text.

use std::fmt::Display;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// After this many records the file stops growing, so a loop that warns
/// forever cannot fill the disk.
const RECORDS_LIMIT: usize = 1_000_000;

#[derive(Default)]
struct Inner {
    file: Option<File>,
    records: usize,
    stopped: bool,
    to_stderr: bool,
}

/// A cheap handle to the client log. Clones write to the same file.
#[derive(Clone, Default)]
pub struct Log(Arc<Mutex<Inner>>);

impl Log {
    /// A log that writes nowhere.
    pub fn silent() -> Self {
        Self::default()
    }

    /// A log that writes to standard error only.
    pub fn stderr() -> Self {
        Log(Arc::new(Mutex::new(Inner {
            to_stderr: true,
            ..Inner::default()
        })))
    }

    /// A log that empties `path`, then writes to it and to standard error.
    pub fn to_file(path: &Path) -> Self {
        let file = File::create(path);
        let log = Self::stderr();
        match file {
            Ok(file) => log.lock().file = Some(file),
            Err(e) => log.warning(format!("cannot write {}: {e}", path.display())),
        }
        log
    }

    /// A log that empties `path`, then writes to it only.
    pub fn file_only(path: &Path) -> Self {
        let log = Self::to_file(path);
        log.lock().to_stderr = false;
        log
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn warning(&self, message: impl Display) {
        self.record("WARNING", &message.to_string());
    }

    pub fn error(&self, message: impl Display) {
        self.record("ERROR", &message.to_string());
    }

    /// Information for a developer at the console; not written to the file.
    pub fn info(&self, message: impl Display) {
        if self.lock().to_stderr {
            eprintln!("INFO: {message}");
        }
    }

    /// Records what the voice said, as text.
    pub fn said(&self, text: &str) {
        self.record("SAID", text);
    }

    fn record(&self, level: &str, message: &str) {
        let mut inner = self.lock();
        if inner.to_stderr {
            eprintln!("{level}: {message}");
        }
        if inner.stopped {
            return;
        }
        inner.records += 1;
        let line = if inner.records <= RECORDS_LIMIT {
            format!("{} {level}: {message}\n", timestamp())
        } else {
            inner.stopped = true;
            "*** too many records, logging stopped ***\n".to_owned()
        };
        if let Some(file) = inner.file.as_mut() {
            // A log that cannot be written must not stop the game.
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
    }
}

/// The local time as `YYYY-MM-DD HH:MM:SS,mmm`, like Python's logging.
#[cfg(windows)]
fn timestamp() -> String {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    // SAFETY: GetLocalTime only writes the SYSTEMTIME it is given.
    let t = unsafe {
        let mut t = std::mem::zeroed();
        GetLocalTime(&mut t);
        t
    };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02},{:03}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}

/// The UTC time as `YYYY-MM-DD HH:MM:SS,mmm` (no time zone database here).
#[cfg(not(windows))]
fn timestamp() -> String {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = since.as_secs() as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02},{:03}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        since.subsec_millis()
    )
}
