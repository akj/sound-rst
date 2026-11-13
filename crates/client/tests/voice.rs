//! The voice's queue, expiry, interruption and history rules, with a
//! recording speech backend and the test clock. Speech lasts 0.1 s per
//! character (the default `wait_delay_per_character`).

use soundrts::clock::Clock;
use soundrts::input::Key;
use soundrts::message::InfoOptions;
use soundrts::testkit::{Harness, Script, SpeechEvent};
use soundrts_world::msg::{Msg, MsgPart};

fn t(s: &str) -> Msg {
    vec![MsgPart::from(s)]
}

#[test]
fn info_messages_queue_and_expire() {
    let mut h = Harness::new("en", Script::new());
    let clock = h.clock.clone();
    let v = &mut h.client.voice;
    v.info(&t("aaaaaaaaaa"));
    v.info_with(
        &t("bbbbb"),
        InfoOptions {
            expiration_delay: 0.5,
            ..InfoOptions::default()
        },
    );
    let food = || InfoOptions {
        update_type: Some("food".into()),
        ..InfoOptions::default()
    };
    v.info_with(&t("ccccc"), food());
    v.info_with(&t("ddddd"), food());
    v.info(&t("eeeee"));
    v.info(&t("eeeee"));
    // Only the first message is said while it lasts.
    assert_eq!(h.spoken(), ["aaaaaaaaaa"]);
    for _ in 0..30 {
        clock.advance(0.1);
        h.client.voice.update();
    }
    // "bbbbb" expired, "ccccc" was replaced by a newer "food" message, and
    // the first "eeeee" by an identical newer one.
    assert_eq!(h.spoken(), ["aaaaaaaaaa", "ddddd", "eeeee"]);
}

const LONG: &str = "a message of fifty characters, said for 5 seconds";

#[test]
fn a_key_press_cuts_an_important_message_short_and_is_used_up() {
    let mut h = Harness::new("en", Script::new().at(0.5).tap(Key::Down));
    h.client.voice.important(&t(LONG));
    assert!(h.clock.now() < 1.0);
    assert!(h.client.input.get_keydowns().is_empty());
    assert_eq!(h.speech.events(), [SpeechEvent::Speak(LONG.into())]);
}

#[test]
fn a_key_press_cuts_a_confirmation_short_and_stays_queued() {
    let mut h = Harness::new("en", Script::new().at(0.5).tap(Key::Down));
    h.client.voice.confirmation(&t(LONG));
    assert!(h.clock.now() < 1.0);
    assert_eq!(h.client.input.get_keydowns().len(), 1);
}

#[test]
fn a_key_press_does_not_cut_an_alert_and_is_dropped() {
    let mut h = Harness::new("en", Script::new().at(0.5).tap(Key::Down));
    h.client.voice.alert(&t(LONG));
    // 49 characters, one of them a digit counted 3 times: 5.1 s.
    assert!(h.clock.now() >= 5.1 - 1e-9, "{}", h.clock.now());
    assert!(h.client.input.get_keydowns().is_empty());
}

#[test]
fn the_history_replays_previous_and_next_messages() {
    let mut h = Harness::new("en", Script::new());
    let clock = h.clock.clone();
    for m in ["one", "two", "three"] {
        h.client.voice.important(&t(m));
    }
    let v = &mut h.client.voice;
    v.previous();
    v.previous();
    v.say_next(false);
    for _ in 0..20 {
        clock.advance(0.1);
        h.client.voice.update();
    }
    h.client.voice.previous();
    assert_eq!(
        h.spoken(),
        ["one", "two", "three", "three", "two", "three", "three"]
    );
}

#[test]
fn say_next_in_history_only_mode_does_not_skip_an_unsaid_message() {
    let mut h = Harness::new("en", Script::new());
    let v = &mut h.client.voice;
    v.info(&t("xxxxxxxxxx"));
    v.info(&t("y"));
    v.say_next(true);
    assert_eq!(h.spoken(), ["xxxxxxxxxx"]);
    h.client.voice.say_next(false);
    assert_eq!(h.spoken(), ["xxxxxxxxxx", "y"]);
}

#[test]
fn items_are_not_kept_in_the_history() {
    let mut h = Harness::new("en", Script::new());
    h.client.voice.important(&t("kept"));
    h.client.voice.item(&t("not kept"));
    let history: Vec<_> = h
        .client
        .voice
        .history()
        .map(|(m, said)| (m.clone(), said))
        .collect();
    assert_eq!(history, [(t("kept"), true)]);
}

#[test]
fn the_history_drops_the_oldest_hundred_past_two_hundred_messages() {
    let mut h = Harness::new("en", Script::new());
    for i in 0..201 {
        h.client.voice.important(&t(&format!("m{i}")));
    }
    assert_eq!(h.client.voice.history().count(), 201);
    h.client.voice.update();
    let history: Vec<_> = h.client.voice.history().map(|(m, _)| m.clone()).collect();
    assert_eq!(history.len(), 101);
    assert_eq!(history[0], t("m100"));
}
