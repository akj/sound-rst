//! The system clipboard: text the game puts there for the player to paste
//! elsewhere, and text the player pastes into the game.

/// The most UTF-16 units read from the clipboard: past them, the text is
/// cut. A text entry keeps far fewer characters.
pub const MAX_READ_UNITS: usize = 4096;

/// Replaces the clipboard's content with `text`.
#[cfg(windows)]
pub fn set_text(text: &str) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{GlobalFree, HANDLE};
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock,
    };
    use windows_sys::Win32::System::Ole::CF_UNICODETEXT;

    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = wide.len() * std::mem::size_of::<u16>();
    // SAFETY: the clipboard is opened and closed here; the memory handed to
    // it is a movable global block filled while locked, and freed only if
    // the clipboard did not take it.
    unsafe {
        if OpenClipboard(0) == 0 {
            return Err("cannot open the clipboard".into());
        }
        let result = (|| {
            if EmptyClipboard() == 0 {
                return Err("cannot empty the clipboard".to_owned());
            }
            let block = GlobalAlloc(GMEM_MOVEABLE, bytes);
            if block.is_null() {
                return Err("out of memory".to_owned());
            }
            let target = GlobalLock(block) as *mut u16;
            if target.is_null() {
                GlobalFree(block);
                return Err("cannot lock the clipboard memory".to_owned());
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), target, wide.len());
            GlobalUnlock(block);
            if SetClipboardData(u32::from(CF_UNICODETEXT), block as HANDLE) == 0 {
                GlobalFree(block);
                return Err("cannot set the clipboard".to_owned());
            }
            Ok(())
        })();
        CloseClipboard();
        result
    }
}

/// Replaces the clipboard's content with `text`.
#[cfg(not(windows))]
pub fn set_text(_text: &str) -> Result<(), String> {
    Err("no clipboard on this system".into())
}

/// The clipboard's text, cut to [`MAX_READ_UNITS`]; `None` when it holds
/// no text.
#[cfg(windows)]
pub fn get_text() -> Result<Option<String>, String> {
    use windows_sys::Win32::Foundation::HGLOBAL;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    };
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    use windows_sys::Win32::System::Ole::CF_UNICODETEXT;

    // SAFETY: the clipboard is opened and closed here; its block is read
    // only while locked, and no further than its size.
    unsafe {
        if IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT)) == 0 {
            return Ok(None);
        }
        if OpenClipboard(0) == 0 {
            return Err("cannot open the clipboard".into());
        }
        let result = (|| {
            let block = GetClipboardData(u32::from(CF_UNICODETEXT)) as HGLOBAL;
            if block.is_null() {
                return Ok(None);
            }
            let source = GlobalLock(block) as *const u16;
            if source.is_null() {
                return Err("cannot lock the clipboard memory".to_owned());
            }
            let units = (GlobalSize(block) / std::mem::size_of::<u16>()).min(MAX_READ_UNITS);
            let wide = std::slice::from_raw_parts(source, units);
            let mut end = wide.iter().position(|&u| u == 0).unwrap_or(units);
            // A cut must not split a character in two.
            if end == units && end > 0 && (0xD800..0xDC00).contains(&wide[end - 1]) {
                end -= 1;
            }
            let text = String::from_utf16_lossy(&wide[..end]);
            GlobalUnlock(block);
            Ok(Some(text))
        })();
        CloseClipboard();
        result
    }
}

/// The clipboard's text.
#[cfg(not(windows))]
pub fn get_text() -> Result<Option<String>, String> {
    Err("no clipboard on this system".into())
}
