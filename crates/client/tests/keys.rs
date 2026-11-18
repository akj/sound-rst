//! Keys as a player presses them in a game: a modifier pressed alone is a
//! command of its own when it is released, a modifier held with another
//! key only modifies it, and held keys repeat as the system repeats them.
//! Control or Alt alone stops speech, and Shift alone does nothing.

use soundrts::clock::Clock;
use soundrts::input::{Key, Mods};
use soundrts::message::{Category, News, Policy, Subject};
use soundrts::testkit::{Harness, LiveKeys, Script, TestGame, render};
use soundrts::tts::{Progress, SpeechEnd};
use soundrts::voice::{Heard, VoiceEvent};
use soundrts_world::msg::MsgPart;

/// A paused training game on jl1 with nothing left to say, whose
/// keyboard repeats a held key after `delay` seconds, then every
/// `interval`.
fn game(delay: f64, interval: f64) -> (Harness, TestGame, LiveKeys) {
    let live = LiveKeys::default();
    let script = Script::new()
        .keep_open()
        .repeat(delay, interval)
        .with_live(live.clone());
    let mut h = Harness::new("en", script);
    let mut g = h.start_training("jl1", &["easy"], 1);
    h.run_until_turn(&mut g, 2);
    g.game.execute_command(&mut h.client, "p");
    h.run_until_voice_idle(&mut g);
    (h, g, live)
}

/// What the voice started from its `from`th event on, each utterance
/// with its time in milliseconds after `start`.
fn said(h: &Harness, from: usize, start: f64) -> Vec<String> {
    h.utterance_log
        .events_from(from)
        .into_iter()
        .filter_map(|e| match e {
            VoiceEvent::Started { time, parts, .. } => Some(format!(
                "{} {}",
                ((time - start) * 1000.0).round(),
                render(&parts)
            )),
            VoiceEvent::Cut { .. } => None,
        })
        .collect()
}

/// Runs the game for `seconds`, then until the voice is idle.
fn run(h: &mut Harness, g: &mut TestGame, seconds: f64) {
    h.run_for(g, seconds);
    h.run_until_voice_idle(g);
}

/// Production news of these words, spoken.
fn news(h: &mut Harness, words: &str) {
    let news = News::new(Category::Production, vec![MsgPart::literal(words)]);
    h.client.voice.news(news, Policy::Speak);
}

/// The first of three production messages starts; the two others wait.
fn three_news(h: &mut Harness) {
    for m in ["aaaaaaaaaa", "bbbbbbbbbb", "cccccccccc"] {
        news(h, m);
    }
}

#[test]
fn shift_c_examines_and_control_alone_no_longer_does() {
    let (mut h, mut g, live) = game(0.5, 0.1);
    let (from, start) = (h.utterance_log.len(), h.clock.now());
    live.down_at(start, Key::LCtrl, Mods::CTRL);
    live.up_at(start + 1.0, Key::LCtrl);
    live.down_at(start + 2.0, Key::LShift, Mods::SHIFT);
    live.tap_at(start + 2.1, Key::C, Mods::SHIFT);
    live.up_at(start + 2.2, Key::LShift);
    run(&mut h, &mut g, 2.5);
    assert_eq!(
        said(&h, from, start),
        ["2100 b 1 , meadows , 1 town hall , 1 farm and 1 peasant"]
    );
}

#[test]
fn control_or_alt_alone_silences_and_what_did_i_miss_counts_what_it_silenced() {
    for key in [Key::LCtrl, Key::RCtrl, Key::LAlt, Key::RAlt] {
        let (mut h, mut g, live) = game(0.5, 0.1);
        let (from, start) = (h.utterance_log.len(), h.clock.now());
        three_news(&mut h);
        let mods = if matches!(key, Key::LCtrl | Key::RCtrl) {
            Mods::CTRL
        } else {
            Mods::ALT
        };
        live.down_at(start + 0.3, key, mods);
        live.up_at(start + 0.5, key);
        run(&mut h, &mut g, 5.0);
        assert_eq!(said(&h, from, start), ["0 aaaaaaaaaa"], "{key:?}");
        assert_eq!(
            h.key(&mut g, Key::F4, Mods::NONE),
            ["production 3"],
            "{key:?}"
        );
    }
}

#[test]
fn shift_alone_cuts_nothing_and_the_news_waiting_is_said() {
    let (mut h, mut g, live) = game(0.5, 0.1);
    let (from, start) = (h.utterance_log.len(), h.clock.now());
    three_news(&mut h);
    live.down_at(start + 0.3, Key::LShift, Mods::SHIFT);
    live.up_at(start + 0.5, Key::LShift);
    run(&mut h, &mut g, 1.0);
    assert_eq!(
        said(&h, from, start),
        ["0 aaaaaaaaaa", "1000 bbbbbbbbbb", "2000 cccccccccc"]
    );
}

#[test]
fn control_with_an_arrow_moves_the_cursor_and_then_the_news_waiting_is_said() {
    let (mut h, mut g, live) = game(0.5, 0.1);
    let (from, start) = (h.utterance_log.len(), h.clock.now());
    three_news(&mut h);
    live.down_at(start + 0.3, Key::LCtrl, Mods::CTRL);
    live.tap_at(start + 0.4, Key::Up, Mods::CTRL);
    live.up_at(start + 0.5, Key::LCtrl);
    run(&mut h, &mut g, 1.0);
    // The answer cuts off the news being said, which is not said again.
    assert_eq!(
        said(&h, from, start),
        [
            "0 aaaaaaaaaa",
            "400 b 2 , unknown",
            "1900 bbbbbbbbbb",
            "2900 cccccccccc"
        ]
    );
}

#[test]
fn a_message_the_screen_reader_cancels_while_control_is_held_is_unheard_and_none_starts_until_control_is_released()
 {
    let (mut h, mut g, live) = game(0.5, 0.1);
    let (from, start) = (h.utterance_log.len(), h.clock.now());
    news(&mut h, "aaaaaaaaaa");
    news(&mut h, "bbbbbbbbbb");
    live.down_at(start + 0.3, Key::LCtrl, Mods::CTRL);
    h.run_for(&mut g, 0.4);
    // NVDA silences its speech as Control goes down, and says so.
    h.speech.report(Progress::Ended(SpeechEnd::Cancelled));
    h.run_for(&mut g, 2.0);
    assert_eq!(said(&h, from, start), ["0 aaaaaaaaaa"]);
    // A chord, then the release: the chord's answer, then the queue.
    let now = h.clock.now();
    live.tap_at(now, Key::Up, Mods::CTRL);
    live.up_at(now + 0.1, Key::LCtrl);
    run(&mut h, &mut g, 0.2);
    let said = said(&h, from, start);
    assert_eq!(said.len(), 3, "{said:?}");
    assert!(said[1].ends_with(" b 2 , unknown"), "{said:?}");
    assert!(said[2].ends_with(" bbbbbbbbbb"), "{said:?}");
    let a = h
        .client
        .voice
        .log()
        .find(|e| e.parts() == &[MsgPart::literal("aaaaaaaaaa")])
        .map(|e| e.heard());
    assert_eq!(a, Some(Heard::No));
}

#[test]
fn an_alert_the_screen_reader_cancels_during_a_chord_is_said_once_more() {
    let (mut h, mut g, live) = game(0.5, 0.1);
    let (from, start) = (h.utterance_log.len(), h.clock.now());
    let alert = News::new(Category::Alert, vec![MsgPart::literal("alert at a 2")]);
    h.client.voice.news(alert, Policy::Speak);
    live.down_at(start + 0.3, Key::LCtrl, Mods::CTRL);
    h.run_for(&mut g, 0.35);
    // NVDA silences the alert as Control goes down.
    h.speech.report(Progress::Ended(SpeechEnd::Cancelled));
    live.tap_at(start + 0.4, Key::Up, Mods::CTRL);
    live.up_at(start + 0.5, Key::LCtrl);
    run(&mut h, &mut g, 1.0);
    assert_eq!(
        said(&h, from, start),
        ["0 alert at a 2", "400 b 2 , unknown", "1900 alert at a 2"]
    );
}

#[test]
fn newer_news_that_replaces_a_message_the_screen_reader_cancelled_is_not_skipped_with_it() {
    let (mut h, mut g, live) = game(0.5, 0.1);
    let (from, start) = (h.utterance_log.len(), h.clock.now());
    let gold = |s: &str| {
        News::new(Category::Economy, vec![MsgPart::literal(s)]).about(Subject::Resource(0))
    };
    h.client.voice.news(gold("10 gold"), Policy::Speak);
    live.down_at(start + 0.3, Key::LCtrl, Mods::CTRL);
    h.run_for(&mut g, 0.35);
    // NVDA silences the count as Control goes down, and a newer count
    // takes its place while Control is held.
    h.speech.report(Progress::Ended(SpeechEnd::Cancelled));
    h.run_for(&mut g, 0.05);
    h.client.voice.news(gold("20 gold"), Policy::Speak);
    // A skip, as a binding of `history_stop` gives, skips what was
    // silenced, not the newer count.
    h.client.voice.say_next(false);
    // A chord, so that Control's release does not silence.
    live.tap_at(start + 0.5, Key::Up, Mods::CTRL);
    live.up_at(start + 0.6, Key::LCtrl);
    run(&mut h, &mut g, 2.0);
    let said = said(&h, from, start);
    assert_eq!(said[0], "0 10 gold", "{said:?}");
    assert!(
        said.last().is_some_and(|s| s.ends_with(" 20 gold")),
        "{said:?}"
    );
}

#[test]
fn control_held_with_tab_does_not_silence() {
    let (mut h, mut g, live) = game(0.5, 0.1);
    let (from, start) = (h.utterance_log.len(), h.clock.now());
    live.down_at(start, Key::LCtrl, Mods::CTRL);
    live.tap_at(start + 0.2, Key::Tab, Mods::CTRL);
    live.up_at(start + 0.4, Key::LCtrl);
    run(&mut h, &mut g, 1.0);
    assert_eq!(
        said(&h, from, start),
        ["200 <ui/positional_beep> a meadow to the northwest"]
    );
}

#[test]
fn a_held_modifier_does_not_repeat() {
    let (mut h, mut g, live) = game(0.5, 0.1);
    // Key help says what each key does, once for each press.
    h.key(&mut g, Key::F1, Mods::CTRL);
    let (from, start) = (h.utterance_log.len(), h.clock.now());
    live.down_at(start, Key::LCtrl, Mods::CTRL);
    live.up_at(start + 2.0, Key::LCtrl);
    run(&mut h, &mut g, 2.5);
    assert_eq!(
        said(&h, from, start),
        ["2000 Left Control: stop speech and the news waiting"]
    );
}

#[test]
fn a_held_key_repeats_at_the_systems_delay_and_rate() {
    let (mut h, mut g, live) = game(0.3, 0.2);
    let (from, start) = (h.utterance_log.len(), h.clock.now());
    live.down_at(start, Key::Left, Mods::NONE);
    live.up_at(start + 0.75, Key::Left);
    run(&mut h, &mut g, 1.0);
    // The press moves to a 1; each repeat bumps into the edge of the map.
    assert_eq!(
        said(&h, from, start),
        [
            "0 <move/through_path> a 1 , meadows , in the fog of war , 1 gold mine",
            "300 <ui/blocked> a 1 , meadows , in the fog of war , 1 gold mine",
            "500 <ui/blocked> a 1 , meadows , in the fog of war , 1 gold mine",
            "700 <ui/blocked> a 1 , meadows , in the fog of war , 1 gold mine"
        ]
    );
}
