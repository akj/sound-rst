//! Messages shown outside the game: the answer to a command-line request,
//! and the reason the game cannot start.
//!
//! On Windows the game is a windowed program, so it has no terminal of its
//! own. A message goes to the standard stream when one was given (a pipe or
//! a file), else to the terminal that launched the game. A start-up error
//! with nowhere to go opens a message box, which a screen reader reads, so
//! starting the game from the desktop never fails silently.

/// Prints the answer to a command-line request such as `--probe`.
pub fn print(text: &str) {
    imp::print(text);
}

/// Tells the player why the game cannot start.
pub fn fatal(text: &str) {
    imp::fatal(text);
}

#[cfg(not(windows))]
mod imp {
    pub fn print(text: &str) {
        print!("{text}");
    }

    pub fn fatal(text: &str) {
        eprintln!("{text}");
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
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};

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
            message_box(text);
        }
    }

    /// Whether the process was given this standard stream.
    fn has_stream(which: STD_HANDLE) -> bool {
        // SAFETY: GetStdHandle only reads the process's own handle table.
        let handle = unsafe { GetStdHandle(which) };
        handle != 0 && handle != INVALID_HANDLE_VALUE
    }

    /// The terminal the game was started from, if there is one.
    fn launching_terminal() -> Option<File> {
        // SAFETY: AttachConsole takes no pointer; failing is harmless.
        if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
            return None;
        }
        OpenOptions::new().write(true).open("CONOUT$").ok()
    }

    fn message_box(text: &str) {
        let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
        let (text, title) = (wide(text), wide("SoundRTS"));
        // SAFETY: both pointers are NUL-terminated UTF-16 buffers that live
        // until the call returns; a null owner window is allowed.
        unsafe { MessageBoxW(0, text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR) };
    }
}
