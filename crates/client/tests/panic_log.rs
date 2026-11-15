//! A panic is written to the client log with the call stack that led to it,
//! naming the file and line of each frame. Run it with `--release` too: that
//! is the profile players get.

use soundrts::log::Log;

#[inline(never)]
fn order_the_footman_to_panic() {
    panic!("the footman refused");
}

#[test]
fn a_panic_is_logged_with_the_file_and_line_it_came_from() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("client.log");
    let log = Log::file_only(&path);
    log.record_panics();

    let caught = std::panic::catch_unwind(order_the_footman_to_panic);
    let _ = std::panic::take_hook();
    assert!(caught.is_err());

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("ERROR: error: panicked at "), "{text}");
    assert!(text.contains("the footman refused"), "{text}");
    let stack = text
        .split_once("stack backtrace:")
        .unwrap_or_else(|| panic!("no stack in the log:\n{text}"))
        .1;
    let lines: Vec<&str> = stack.lines().collect();
    let frame = lines
        .iter()
        .position(|l| l.contains("panic_log::order_the_footman_to_panic"))
        .unwrap_or_else(|| panic!("the panicking function is not in the stack:\n{stack}"));
    let place = lines.get(frame + 1).copied().unwrap_or_default();
    assert!(
        place.trim_start().starts_with("at ") && place.trim_end().ends_with("panic_log.rs:9"),
        "the frame does not name panic_log.rs:9, but {place:?}:\n{stack}"
    );
}
