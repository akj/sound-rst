//! Puts text on the system clipboard, for the player to paste elsewhere.

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
