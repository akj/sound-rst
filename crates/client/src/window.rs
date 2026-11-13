//! The window: it takes keyboard focus, carries the title, and turns key
//! presses into [`RawInput`] for the game thread.
//!
//! The window's event loop runs on the main thread; the game runs on its
//! own thread, reads keys from a channel and sends titles and its end back
//! through an event loop proxy.
//!
//! On Windows the window keeps the keys the game binds: a lone Alt or F10
//! does not open the system menu, Alt+letter does not beep, and Alt+F4
//! reaches the game instead of closing the window.

use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key as WKey, KeyCode, KeyLocation, ModifiersState, NamedKey, PhysicalKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::{Window, WindowId};

use crate::client::Desktop;
use crate::input::{InputSource, Key, KeyDown, KeyUp, Mods, RawInput};
use crate::log::Log;

/// What the game thread asks of the window.
#[derive(Debug)]
pub enum WindowCommand {
    SetTitle(String),
    /// The game is over: close the window and end the program.
    Exit,
}

/// The key under a physical position, named by its US label.
pub fn physical_key(code: KeyCode) -> Key {
    use KeyCode as C;
    match code {
        C::KeyA => Key::A,
        C::KeyB => Key::B,
        C::KeyC => Key::C,
        C::KeyD => Key::D,
        C::KeyE => Key::E,
        C::KeyF => Key::F,
        C::KeyG => Key::G,
        C::KeyH => Key::H,
        C::KeyI => Key::I,
        C::KeyJ => Key::J,
        C::KeyK => Key::K,
        C::KeyL => Key::L,
        C::KeyM => Key::M,
        C::KeyN => Key::N,
        C::KeyO => Key::O,
        C::KeyP => Key::P,
        C::KeyQ => Key::Q,
        C::KeyR => Key::R,
        C::KeyS => Key::S,
        C::KeyT => Key::T,
        C::KeyU => Key::U,
        C::KeyV => Key::V,
        C::KeyW => Key::W,
        C::KeyX => Key::X,
        C::KeyY => Key::Y,
        C::KeyZ => Key::Z,
        C::Digit0 => Key::Num0,
        C::Digit1 => Key::Num1,
        C::Digit2 => Key::Num2,
        C::Digit3 => Key::Num3,
        C::Digit4 => Key::Num4,
        C::Digit5 => Key::Num5,
        C::Digit6 => Key::Num6,
        C::Digit7 => Key::Num7,
        C::Digit8 => Key::Num8,
        C::Digit9 => Key::Num9,
        C::Enter => Key::Return,
        C::Escape => Key::Escape,
        C::Backspace => Key::Backspace,
        C::Tab => Key::Tab,
        C::Space => Key::Space,
        C::Minus => Key::Minus,
        C::Equal => Key::Equals,
        C::BracketLeft => Key::LeftBracket,
        C::BracketRight => Key::RightBracket,
        C::Backslash => Key::Backslash,
        C::Semicolon => Key::Semicolon,
        C::Quote => Key::Quote,
        C::Backquote => Key::Backquote,
        C::Comma => Key::Comma,
        C::Period => Key::Period,
        C::Slash => Key::Slash,
        C::IntlBackslash => Key::NonUsBackslash,
        C::CapsLock => Key::CapsLock,
        C::F1 => Key::F1,
        C::F2 => Key::F2,
        C::F3 => Key::F3,
        C::F4 => Key::F4,
        C::F5 => Key::F5,
        C::F6 => Key::F6,
        C::F7 => Key::F7,
        C::F8 => Key::F8,
        C::F9 => Key::F9,
        C::F10 => Key::F10,
        C::F11 => Key::F11,
        C::F12 => Key::F12,
        C::F13 => Key::F13,
        C::F14 => Key::F14,
        C::F15 => Key::F15,
        C::F16 => Key::F16,
        C::F17 => Key::F17,
        C::F18 => Key::F18,
        C::F19 => Key::F19,
        C::F20 => Key::F20,
        C::F21 => Key::F21,
        C::F22 => Key::F22,
        C::F23 => Key::F23,
        C::F24 => Key::F24,
        C::PrintScreen => Key::PrintScreen,
        C::ScrollLock => Key::ScrollLock,
        C::Pause => Key::Pause,
        C::Insert => Key::Insert,
        C::Home => Key::Home,
        C::PageUp => Key::PageUp,
        C::Delete => Key::Delete,
        C::End => Key::End,
        C::PageDown => Key::PageDown,
        C::ArrowRight => Key::Right,
        C::ArrowLeft => Key::Left,
        C::ArrowDown => Key::Down,
        C::ArrowUp => Key::Up,
        C::NumLock => Key::NumLock,
        C::NumpadDivide => Key::KpDivide,
        C::NumpadMultiply => Key::KpMultiply,
        C::NumpadSubtract => Key::KpMinus,
        C::NumpadAdd => Key::KpPlus,
        C::NumpadEnter => Key::KpEnter,
        C::NumpadDecimal => Key::KpPeriod,
        C::NumpadEqual => Key::KpEquals,
        C::Numpad0 => Key::Kp0,
        C::Numpad1 => Key::Kp1,
        C::Numpad2 => Key::Kp2,
        C::Numpad3 => Key::Kp3,
        C::Numpad4 => Key::Kp4,
        C::Numpad5 => Key::Kp5,
        C::Numpad6 => Key::Kp6,
        C::Numpad7 => Key::Kp7,
        C::Numpad8 => Key::Kp8,
        C::Numpad9 => Key::Kp9,
        C::ContextMenu => Key::Application,
        C::ControlLeft => Key::LCtrl,
        C::ControlRight => Key::RCtrl,
        C::ShiftLeft => Key::LShift,
        C::ShiftRight => Key::RShift,
        C::AltLeft => Key::LAlt,
        C::AltRight => Key::RAlt,
        C::SuperLeft => Key::LGui,
        C::SuperRight => Key::RGui,
        _ => Key::Unknown,
    }
}

fn sided(location: KeyLocation, left: Key, right: Key) -> Key {
    if location == KeyLocation::Right {
        right
    } else {
        left
    }
}

fn named_key(n: NamedKey, location: KeyLocation) -> Key {
    use NamedKey as N;
    match n {
        N::Enter if location == KeyLocation::Numpad => Key::KpEnter,
        N::Enter => Key::Return,
        N::Escape => Key::Escape,
        N::Backspace => Key::Backspace,
        N::Tab => Key::Tab,
        N::Space => Key::Space,
        N::Delete => Key::Delete,
        N::Insert => Key::Insert,
        N::Home => Key::Home,
        N::End => Key::End,
        N::PageUp => Key::PageUp,
        N::PageDown => Key::PageDown,
        N::ArrowLeft => Key::Left,
        N::ArrowRight => Key::Right,
        N::ArrowUp => Key::Up,
        N::ArrowDown => Key::Down,
        N::CapsLock => Key::CapsLock,
        N::NumLock => Key::NumLock,
        N::ScrollLock => Key::ScrollLock,
        N::PrintScreen => Key::PrintScreen,
        N::Pause => Key::Pause,
        N::ContextMenu => Key::Application,
        N::Shift => sided(location, Key::LShift, Key::RShift),
        N::Control => sided(location, Key::LCtrl, Key::RCtrl),
        N::Alt => sided(location, Key::LAlt, Key::RAlt),
        N::AltGraph => Key::RAlt,
        N::Super => sided(location, Key::LGui, Key::RGui),
        N::F1 => Key::F1,
        N::F2 => Key::F2,
        N::F3 => Key::F3,
        N::F4 => Key::F4,
        N::F5 => Key::F5,
        N::F6 => Key::F6,
        N::F7 => Key::F7,
        N::F8 => Key::F8,
        N::F9 => Key::F9,
        N::F10 => Key::F10,
        N::F11 => Key::F11,
        N::F12 => Key::F12,
        N::F13 => Key::F13,
        N::F14 => Key::F14,
        N::F15 => Key::F15,
        N::F16 => Key::F16,
        N::F17 => Key::F17,
        N::F18 => Key::F18,
        N::F19 => Key::F19,
        N::F20 => Key::F20,
        N::F21 => Key::F21,
        N::F22 => Key::F22,
        N::F23 => Key::F23,
        N::F24 => Key::F24,
        _ => Key::Unknown,
    }
}

fn numpad_char(c: char) -> Option<Key> {
    Some(match c {
        '0' => Key::Kp0,
        '1' => Key::Kp1,
        '2' => Key::Kp2,
        '3' => Key::Kp3,
        '4' => Key::Kp4,
        '5' => Key::Kp5,
        '6' => Key::Kp6,
        '7' => Key::Kp7,
        '8' => Key::Kp8,
        '9' => Key::Kp9,
        '.' | ',' => Key::KpPeriod,
        '+' => Key::KpPlus,
        '-' => Key::KpMinus,
        '*' => Key::KpMultiply,
        '/' => Key::KpDivide,
        '=' => Key::KpEquals,
        _ => return None,
    })
}

/// The key as the layout names it, ignoring modifiers, as pygame key codes
/// did; the physical key if the layout gives no name.
fn logical_key(event: &KeyEvent, physical: Key) -> Key {
    let key = match event.key_without_modifiers() {
        WKey::Named(n) => named_key(n, event.location),
        WKey::Character(s) => {
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if event.location == KeyLocation::Numpad => {
                    numpad_char(c).unwrap_or_else(|| Key::from_char(c))
                }
                (Some(c), None) => Key::from_char(c),
                _ => Key::Unknown,
            }
        }
        _ => Key::Unknown,
    };
    if key == Key::Unknown { physical } else { key }
}

fn mods_of(state: ModifiersState) -> Mods {
    Mods {
        ctrl: state.control_key(),
        alt: state.alt_key(),
        shift: state.shift_key(),
    }
}

/// Turns a winit key event into the game's input. Repeats from the system
/// are dropped: the event queue makes its own.
pub fn translate(event: &KeyEvent, modifiers: ModifiersState) -> Option<RawInput> {
    if event.repeat {
        return None;
    }
    let physical = match event.physical_key {
        PhysicalKey::Code(code) => physical_key(code),
        PhysicalKey::Unidentified(_) => Key::Unknown,
    };
    let key = logical_key(event, physical);
    Some(match event.state {
        ElementState::Pressed => RawInput::KeyDown(KeyDown {
            key,
            physical,
            mods: mods_of(modifiers),
            unicode: event
                .text
                .as_ref()
                .map(|t| t.to_string())
                .unwrap_or_default(),
        }),
        ElementState::Released => RawInput::KeyUp(KeyUp { key, physical }),
    })
}

/// The game thread's keyboard: what the window sent.
pub struct ChannelInput {
    rx: Receiver<(Instant, RawInput)>,
    start: Instant,
}

impl ChannelInput {
    /// `start` is the instant the game clock counts from.
    pub fn new(rx: Receiver<(Instant, RawInput)>, start: Instant) -> ChannelInput {
        ChannelInput { rx, start }
    }
}

impl InputSource for ChannelInput {
    fn next(&mut self, now: f64) -> Option<(f64, RawInput)> {
        let (at, raw) = self.rx.try_recv().ok()?;
        let time = at
            .saturating_duration_since(self.start)
            .as_secs_f64()
            .min(now);
        Some((time, raw))
    }
}

/// The desktop as the game thread sees it: the window's title, and the
/// system's file browser.
pub struct WindowDesktop {
    proxy: EventLoopProxy<WindowCommand>,
    log: Log,
}

impl Desktop for WindowDesktop {
    fn set_title(&self, title: &str) {
        let _ = self
            .proxy
            .send_event(WindowCommand::SetTitle(title.to_owned()));
    }

    fn open_folder(&self, path: &Path) {
        let program = if cfg!(windows) {
            "explorer"
        } else if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        if let Err(e) = std::process::Command::new(program).arg(path).spawn() {
            self.log
                .warning(format!("cannot open {}: {e}", path.display()));
        }
    }
    fn set_clipboard(&self, text: &str) {
        if let Err(e) = crate::clipboard::set_text(text) {
            self.log
                .warning(format!("cannot copy to the clipboard: {e}"));
        }
    }
}

/// What the game thread gets to talk to the window.
pub struct WindowLink {
    pub input: ChannelInput,
    pub desktop: WindowDesktop,
    /// The instant key times count from; the game clock must count from it
    /// too.
    pub start: Instant,
    /// Send [`WindowCommand::Exit`] here when the game is over.
    pub proxy: EventLoopProxy<WindowCommand>,
}

struct App {
    title: String,
    window: Option<Window>,
    modifiers: ModifiersState,
    tx: Sender<(Instant, RawInput)>,
    log: Log,
}

impl App {
    fn send(&self, raw: RawInput) {
        let _ = self.tx.send((Instant::now(), raw));
    }
}

impl ApplicationHandler<WindowCommand> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title(self.title.clone())
            .with_inner_size(LogicalSize::new(400.0, 75.0))
            .with_active(true);
        match event_loop.create_window(attributes) {
            Ok(window) => {
                #[cfg(windows)]
                keep_system_keys(&window, &self.log);
                window.focus_window();
                self.window = Some(window);
            }
            Err(e) => {
                self.log.error(format!("cannot open the window: {e}"));
                self.send(RawInput::Quit);
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, command: WindowCommand) {
        match command {
            WindowCommand::SetTitle(title) => {
                if let Some(w) = &self.window {
                    w.set_title(&title);
                }
                self.title = title;
            }
            WindowCommand::Exit => event_loop.exit(),
        }
    }

    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                // Windows replays held keys when the window gets the focus
                // back; those are not presses.
                if !is_synthetic && let Some(raw) = translate(&event, self.modifiers) {
                    self.send(raw);
                }
            }
            WindowEvent::Focused(false) => self.send(RawInput::FocusLost),
            WindowEvent::CloseRequested => self.send(RawInput::Quit),
            _ => {}
        }
    }
}

/// Opens the window on this (main) thread, runs `game` on its own thread
/// with the link to the window, and returns when the game is over.
pub fn run<G>(title: String, log: Log, game: G) -> Result<(), String>
where
    G: FnOnce(WindowLink) + Send + 'static,
{
    let event_loop = EventLoop::<WindowCommand>::with_user_event()
        .build()
        .map_err(|e| e.to_string())?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let (tx, rx) = channel();
    let proxy = event_loop.create_proxy();
    let start = Instant::now();
    let link = WindowLink {
        input: ChannelInput::new(rx, start),
        desktop: WindowDesktop {
            proxy: proxy.clone(),
            log: log.clone(),
        },
        start,
        proxy,
    };
    let game_thread = std::thread::Builder::new()
        .name("game".to_owned())
        .spawn(move || game(link))
        .map_err(|e| e.to_string())?;
    let mut app = App {
        title,
        window: None,
        modifiers: ModifiersState::empty(),
        tx,
        log,
    };
    let result = event_loop.run_app(&mut app).map_err(|e| e.to_string());
    let _ = game_thread.join();
    result
}

#[cfg(windows)]
fn keep_system_keys(window: &Window, log: &Log) {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let hwnd = match window.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
        _ => {
            log.warning("cannot find the window handle; Alt and F10 may open the system menu");
            return;
        }
    };
    if !windows_keys::install(hwnd) {
        log.warning("cannot watch the window's messages; Alt and F10 may open the system menu");
    }
}

#[cfg(windows)]
mod windows_keys {
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_F4;
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SC_KEYMENU, WM_KEYDOWN, WM_SYSCOMMAND, WM_SYSKEYDOWN,
    };

    /// Runs before winit's window procedure.
    ///
    /// - `WM_SYSCOMMAND`/`SC_KEYMENU` is dropped: that is how Windows enters
    ///   the system menu after a lone Alt, F10, or Alt+letter (whose failed
    ///   menu lookup is also what beeps).
    /// - `WM_SYSKEYDOWN` for F4 is passed on as `WM_KEYDOWN`: winit then
    ///   reports the key (it drops Alt+F4 otherwise) and does not let
    ///   Windows turn it into a close request.
    unsafe extern "system" fn subclass_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        if msg == WM_SYSCOMMAND && (wparam & 0xFFF0) == SC_KEYMENU as usize {
            return 0;
        }
        let msg = if msg == WM_SYSKEYDOWN && wparam == usize::from(VK_F4) {
            WM_KEYDOWN
        } else {
            msg
        };
        // SAFETY: forwards a message of this window to the next procedure
        // of its subclass chain, as SetWindowSubclass requires.
        unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
    }

    pub fn install(hwnd: isize) -> bool {
        // SAFETY: `hwnd` is the live window winit created on this thread;
        // the procedure stays valid for the life of the program.
        unsafe { SetWindowSubclass(hwnd, Some(subclass_proc), 1, 0) != 0 }
    }
}
