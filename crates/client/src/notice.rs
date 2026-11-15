//! Messages shown outside the game: the answer to a command-line request,
//! and the reason the game cannot start.
//!
//! On Windows the game is a windowed program, so it has no terminal of its
//! own. A message goes to the standard stream when one was given (a pipe or
//! a file), else to the terminal that launched the game. A start-up error
//! with nowhere to go opens a message box, which a screen reader reads, so
//! starting the game from the desktop never fails silently. In that same
//! case the game can ask the player a question in a message box.

/// Prints the answer to a command-line request such as `--probe`.
pub fn print(text: &str) {
    imp::print(text);
}

/// Tells the player why the game cannot start.
pub fn fatal(text: &str) {
    imp::fatal(text);
}

/// Whether the game can ask the player in a message box: exactly when
/// [`fatal`] would use one, so a game started from a terminal, a script or
/// a pipe never waits on a dialog.
pub fn can_ask() -> bool {
    imp::can_ask()
}

/// Asks a question in a message box with Yes and No. Whether the answer is
/// Yes.
pub fn ask_yes_no(text: &str) -> bool {
    imp::ask_yes_no(text)
}

/// Tells why something failed in a message box with Retry and Cancel.
/// Whether the answer is Retry.
pub fn ask_retry(text: &str) -> bool {
    imp::ask_retry(text)
}

#[cfg(not(windows))]
mod imp {
    pub fn print(text: &str) {
        print!("{text}");
    }

    pub fn fatal(text: &str) {
        eprintln!("{text}");
    }

    pub fn can_ask() -> bool {
        false
    }

    pub fn ask_yes_no(_text: &str) -> bool {
        false
    }

    pub fn ask_retry(_text: &str) -> bool {
        false
    }
}

#[cfg(windows)]
mod imp {
    use std::fs::{File, OpenOptions};
    use std::io::Write;

    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_HANDLE,
        STD_OUTPUT_HANDLE,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IDRETRY, IDYES, MB_ICONERROR, MB_ICONQUESTION, MB_OK, MB_RETRYCANCEL, MB_YESNO,
        MESSAGEBOX_RESULT, MESSAGEBOX_STYLE, MessageBoxW,
    };

    pub fn print(text: &str) {
        if has_stream(STD_OUTPUT_HANDLE) {
            print!("{text}");
        } else if let Some(mut terminal) = launching_terminal() {
            // The shell has already printed its prompt: start on a new line.
            let _ = write!(terminal, "\n{text}");
        }
    }

    pub fn fatal(text: &str) {
        if has_stream(STD_ERROR_HANDLE) {
            eprintln!("{text}");
        } else if let Some(mut terminal) = launching_terminal() {
            let _ = writeln!(terminal, "\n{text}");
        } else {
            message_box(text, MB_OK | MB_ICONERROR);
        }
    }

    pub fn can_ask() -> bool {
        !has_stream(STD_ERROR_HANDLE) && launching_terminal().is_none()
    }

    pub fn ask_yes_no(text: &str) -> bool {
        message_box(text, MB_YESNO | MB_ICONQUESTION) == IDYES
    }

    pub fn ask_retry(text: &str) -> bool {
        message_box(text, MB_RETRYCANCEL | MB_ICONERROR) == IDRETRY
    }

    /// Whether the process was given this standard stream.
    fn has_stream(which: STD_HANDLE) -> bool {
        // SAFETY: GetStdHandle only reads the process's own handle table.
        let handle = unsafe { GetStdHandle(which) };
        handle != 0 && handle != INVALID_HANDLE_VALUE
    }

    /// The terminal the game was started from, if there is one. Asking
    /// twice gives the same answer: a second attach fails, but the console
    /// attached by the first is still there to open.
    fn launching_terminal() -> Option<File> {
        // SAFETY: AttachConsole takes no pointer; failing is harmless.
        unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
        OpenOptions::new().write(true).open("CONOUT$").ok()
    }

    fn message_box(text: &str, style: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
        let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
        let (text, title) = (wide(text), wide("SoundRTS"));
        // SAFETY: both pointers are NUL-terminated UTF-16 buffers that live
        // until the call returns; a null owner window is allowed.
        unsafe { MessageBoxW(0, text.as_ptr(), title.as_ptr(), style) }
    }
}
