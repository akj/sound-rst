//! Keys as a player presses them in a game: a modifier pressed alone is a
//! command of its own when it is released, a modifier held with another
//! key only modifies it, and held keys repeat as the system repeats them.

use soundrts::clock::Clock;
use soundrts::input::{Key, Mods};
use soundrts::message::{Category, News, Policy};
use soundrts::testkit::{Harness, LiveKeys, Script, TestGame, render};
use soundrts::voice::VoiceEvent;
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
    g.game.execute_command(&mut h.client, "p").unwrap();
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

#[test]
fn control_alone_examines_when_it_is_released() {
    let (mut h, mut g, live) = game(0.5, 0.1);
    let (from, start) = (h.utterance_log.len(), h.clock.now());
    live.down_at(start, Key::LCtrl, Mods::CTRL);
    live.up_at(start + 1.0, Key::LCtrl);
    run(&mut h, &mut g, 1.5);
    assert_eq!(
        said(&h, from, start),
        ["1000 b 1 , meadows , 1 town hall , 1 farm and 1 peasant"]
    );
}

#[test]
fn control_held_with_tab_does_not_examine() {
    let (mut h, mut g, live) = game(0.5, 0.1);
    let (from, start) = (h.utterance_log.len(), h.clock.now());
    live.down_at(start, Key::LCtrl, Mods::CTRL);
    live.tap_at(start + 0.2, Key::Tab, Mods::CTRL);
    live.up_at(start + 0.4, Key::LCtrl);
    run(&mut h, &mut g, 1.0);
    assert_eq!(
        said(&h, from, start),
        ["200 <1003> a meadow to the northwest"]
    );
}

#[test]
fn a_held_modifier_does_not_repeat() {
    let (mut h, mut g, live) = game(0.5, 0.1);
    let (from, start) = (h.utterance_log.len(), h.clock.now());
    live.down_at(start, Key::LCtrl, Mods::CTRL);
    live.up_at(start + 2.0, Key::LCtrl);
    run(&mut h, &mut g, 2.5);
    assert_eq!(
        said(&h, from, start),
        ["2000 b 1 , meadows , 1 town hall , 1 farm and 1 peasant"]
    );
}

#[test]
fn shift_and_an_arrow_do_not_skip_the_news_being_said() {
    let (mut h, mut g, live) = game(0.5, 0.1);
    let (from, start) = (h.utterance_log.len(), h.clock.now());
    let news = News::new(Category::Production, vec![MsgPart::from("aaaaaaaaaa")]);
    h.client.voice.news(news, Policy::Speak);
    live.down_at(start + 0.3, Key::LShift, Mods::SHIFT);
    live.tap_at(start + 0.4, Key::Up, Mods::SHIFT);
    live.up_at(start + 0.5, Key::LShift);
    run(&mut h, &mut g, 1.0);
    // The arrow's answer cuts the news, which is said again after it.
    assert_eq!(
        said(&h, from, start),
        [
            "0 aaaaaaaaaa",
            "400 <1027> b 1 , meadows , 1 town hall , 1 farm and 1 peasant",
            "6394 aaaaaaaaaa"
        ]
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
            "0 <1091> a 1 , meadows , in the fog of war , 1 gold mine",
            "300 <1027> a 1 , meadows , in the fog of war , 1 gold mine",
            "500 <1027> a 1 , meadows , in the fog of war , 1 gold mine",
            "700 <1027> a 1 , meadows , in the fog of war , 1 gold mine"
        ]
    );
}
