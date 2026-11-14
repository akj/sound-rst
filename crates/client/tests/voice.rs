//! The voice's queue, subjects, policies, interruption and log rules, with
//! a recording speech backend and the test clock. Speech lasts 0.1 s per
//! character (the default `wait_delay_per_character`).

use soundrts::clock::Clock;
use soundrts::input::Key;
use soundrts::message::{Category, News, Policy, Subject};
use soundrts::testkit::{Harness, Script, SpeechEvent};
use soundrts::tts::{Progress, SpeechEnd};
use soundrts::voice::{Heard, LOG_LIMIT, Voice};
use soundrts_world::SquareId;
use soundrts_world::msg::{Msg, MsgPart};

fn t(s: &str) -> Msg {
    vec![MsgPart::from(s)]
}

/// News of `category` that is spoken.
fn tell(v: &mut Voice, category: Category, s: &str) {
    v.news(News::new(category, t(s)), Policy::Speak);
}

fn info(v: &mut Voice, s: &str) {
    tell(v, Category::Production, s);
}

fn chat(v: &mut Voice, s: &str) {
    tell(v, Category::Chat, s);
}

/// Lets the voice run for `seconds`, updating it every 0.1 s.
fn run(h: &mut Harness, seconds: f64) {
    for _ in 0..(seconds * 10.0).round() as usize {
        h.clock.advance(0.1);
        h.client.voice.update();
    }
}

/// The log, as each message's texts and whether it was heard.
fn log(h: &Harness) -> Vec<(String, Heard)> {
    h.client
        .voice
        .log()
        .map(|e| {
            let texts: Vec<String> = e
                .parts()
                .iter()
                .map(|p| match p {
                    MsgPart::Text(s) | MsgPart::Literal(s) => s.clone(),
                    MsgPart::Num(n) => n.to_string(),
                })
                .collect();
            (texts.join(" "), e.heard())
        })
        .collect()
}

#[test]
fn newer_news_about_a_subject_replaces_the_unheard_news_about_it_and_not_another() {
    let mut h = Harness::new("en", Script::new());
    let v = &mut h.client.voice;
    info(v, "aaaaaaaaaa");
    let gold = |s: &str| News::new(Category::Economy, t(s)).about(Subject::Resource(0));
    let wood = |s: &str| News::new(Category::Economy, t(s)).about(Subject::Resource(1));
    v.news(gold("10 gold"), Policy::Speak);
    v.news(wood("10 wood"), Policy::Speak);
    v.news(gold("20 gold"), Policy::Speak);
    run(&mut h, 6.0);
    // The newer count takes the older one's turn.
    assert_eq!(h.spoken(), ["aaaaaaaaaa", "20 gold", "10 wood"]);
    // The count replaced is not kept: the newer one says it all.
    let texts: Vec<String> = log(&h).into_iter().map(|(s, _)| s).collect();
    assert_eq!(texts, ["aaaaaaaaaa", "20 gold", "10 wood"]);
}

#[test]
fn news_about_a_subject_being_said_does_not_replace_it() {
    let mut h = Harness::new("en", Script::new());
    let gold = |s: &str| News::new(Category::Economy, t(s)).about(Subject::Resource(0));
    h.client.voice.news(gold("10 gold"), Policy::Speak);
    run(&mut h, 0.3);
    h.client.voice.news(gold("20 gold"), Policy::Speak);
    run(&mut h, 3.0);
    assert_eq!(h.spoken(), ["10 gold", "20 gold"]);
}

#[test]
fn news_about_distinct_things_is_all_said_even_when_its_words_are_the_same() {
    let mut h = Harness::new("en", Script::new());
    let v = &mut h.client.voice;
    info(v, "aaaaaaaaaa");
    info(v, "peasant ready");
    info(v, "peasant ready");
    run(&mut h, 5.0);
    assert_eq!(h.spoken(), ["aaaaaaaaaa", "peasant ready", "peasant ready"]);
}

#[test]
fn news_waiting_a_long_time_is_still_said() {
    let mut h = Harness::new("en", Script::new());
    info(&mut h.client.voice, "aaaaaaaaaa");
    info(&mut h.client.voice, "bbbbb");
    // The screen reader goes on speaking for a minute.
    h.speech.report(Progress::Speaking);
    run(&mut h, 60.0);
    h.speech.report(Progress::Ended(SpeechEnd::Finished));
    h.client.voice.update();
    assert_eq!(h.spoken(), ["aaaaaaaaaa", "bbbbb"]);
}

const LONG: &str = "a message of fifty characters, said for 5 seconds";

#[test]
fn a_key_press_cuts_an_important_message_short_and_is_used_up() {
    let mut h = Harness::new("en", Script::new().at(0.5).tap(Key::Down));
    h.client.voice.important(&t(LONG));
    assert!(h.clock.now() < 1.0);
    assert!(h.client.input.get_keydowns().is_empty());
    // The message first stops whatever was being said.
    assert_eq!(
        h.speech.events(),
        [SpeechEvent::Stop, SpeechEvent::Speak(LONG.into())]
    );
}

#[test]
fn a_key_press_cuts_a_confirmation_short_and_stays_queued() {
    let mut h = Harness::new("en", Script::new().at(0.5).tap(Key::Down));
    h.client.voice.confirmation(&t(LONG));
    assert!(h.clock.now() < 1.0);
    assert_eq!(h.client.input.get_keydowns().len(), 1);
}

#[test]
fn in_a_game_a_message_said_now_does_not_wait_and_the_key_stays_for_the_game() {
    let mut h = Harness::new("en", Script::new().at(0.5).tap(Key::Down));
    h.client.voice.set_in_game(true);
    let before = h.clock.now();
    h.client.voice.confirmation(&t(LONG));
    h.client.voice.alert(&t(LONG));
    assert_eq!(h.clock.now(), before);
    assert_eq!(h.spoken(), [LONG, LONG]);
    assert!(h.client.voice.is_busy());
    run(&mut h, 1.0);
    assert_eq!(h.client.input.get_keydowns().len(), 1);
}

#[test]
fn the_end_of_a_game_gives_up_its_news_but_what_it_is_about() {
    let mut h = Harness::new("en", Script::new());
    let v = &mut h.client.voice;
    info(v, "said");
    info(v, "production");
    tell(v, Category::Scouting, "scouting");
    tell(v, Category::Story, "mission complete");
    tell(v, Category::System, "player quits");
    v.end_game();
    run(&mut h, 10.0);
    // The system news first, as it was ahead of the story.
    assert_eq!(h.spoken(), ["said", "player quits", "mission complete"]);
    assert_eq!(
        log(&h),
        [
            ("said".to_owned(), Heard::Yes),
            ("production".to_owned(), Heard::No),
            ("scouting".to_owned(), Heard::No),
            ("mission complete".to_owned(), Heard::Yes),
            ("player quits".to_owned(), Heard::Yes),
        ]
    );
}

#[test]
fn the_end_of_a_game_ends_a_walk_through_the_history() {
    let mut h = Harness::new("en", Script::new());
    for m in ["one", "two", "three"] {
        info(&mut h.client.voice, m);
    }
    run(&mut h, 3.0);
    h.client.voice.previous();
    h.client.voice.previous();
    h.client.voice.end_game();
    run(&mut h, 3.0);
    // "two" again from the history, and not "three" after it.
    assert_eq!(h.spoken(), ["one", "two", "three", "three", "two"]);
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
    for m in ["one", "two", "three"] {
        h.client.voice.important(&t(m));
    }
    let v = &mut h.client.voice;
    v.previous();
    v.previous();
    v.say_next(false);
    run(&mut h, 2.0);
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
    info(v, "xxxxxxxxxx");
    info(v, "y");
    v.say_next(true);
    assert_eq!(h.spoken(), ["xxxxxxxxxx"]);
    h.client.voice.say_next(false);
    assert_eq!(h.spoken(), ["xxxxxxxxxx", "y"]);
}

#[test]
fn items_are_not_kept_in_the_log() {
    let mut h = Harness::new("en", Script::new());
    h.client.voice.important(&t("kept"));
    h.client.voice.item(&t("not kept"));
    assert_eq!(log(&h), [("kept".to_owned(), Heard::Yes)]);
}

#[test]
fn the_log_drops_only_its_oldest_message_past_its_limit() {
    let mut h = Harness::new("en", Script::new());
    for i in 0..LOG_LIMIT + 2 {
        h.client.voice.news(
            News::new(Category::Production, t(&format!("m{i}"))),
            Policy::Log,
        );
    }
    let log = log(&h);
    assert_eq!(log.len(), LOG_LIMIT);
    assert_eq!(log[0].0, "m2");
    assert_eq!(log[LOG_LIMIT - 1].0, format!("m{}", LOG_LIMIT + 1));
}

#[test]
fn the_log_is_trimmed_once_its_oldest_message_has_been_said() {
    let mut h = Harness::new("en", Script::new());
    info(&mut h.client.voice, "the oldest");
    for i in 0..LOG_LIMIT {
        h.client.voice.news(
            News::new(Category::Production, t(&format!("m{i}"))),
            Policy::Log,
        );
    }
    // Being said, the oldest message stays for now.
    assert_eq!(log(&h).len(), LOG_LIMIT + 1);
    run(&mut h, 2.0);
    let log = log(&h);
    assert_eq!(log.len(), LOG_LIMIT);
    assert_eq!(log[0].0, "m0");
}

#[test]
fn the_log_is_trimmed_once_its_oldest_message_is_skipped() {
    let mut h = Harness::new("en", Script::new());
    info(&mut h.client.voice, "the oldest");
    for i in 0..LOG_LIMIT {
        h.client.voice.news(
            News::new(Category::Production, t(&format!("m{i}"))),
            Policy::Log,
        );
    }
    h.client.voice.say_next(false);
    assert_eq!(log(&h).len(), LOG_LIMIT);
}

#[test]
fn chat_comes_after_the_message_being_said_and_before_the_unsaid_ones() {
    let mut h = Harness::new("en", Script::new());
    let v = &mut h.client.voice;
    info(v, "aaaaaaaaaa");
    info(v, "bbbbb");
    info(v, "ccccc");
    chat(v, "chat one");
    chat(v, "chat two");
    // The message being said is not cut.
    assert_eq!(h.spoken(), ["aaaaaaaaaa"]);
    run(&mut h, 6.0);
    assert_eq!(
        h.spoken(),
        ["aaaaaaaaaa", "chat one", "chat two", "bbbbb", "ccccc"]
    );
}

#[test]
fn identical_chat_lines_are_all_said_and_an_idle_voice_says_chat_at_once() {
    let mut h = Harness::new("en", Script::new());
    chat(&mut h.client.voice, "hello");
    assert_eq!(h.spoken(), ["hello"]);
    chat(&mut h.client.voice, "hello");
    chat(&mut h.client.voice, "hello");
    info(&mut h.client.voice, "info");
    run(&mut h, 6.0);
    assert_eq!(h.spoken(), ["hello", "hello", "hello", "info"]);
}

#[test]
fn a_finished_report_starts_the_next_queued_message_without_waiting_for_the_estimate() {
    let mut h = Harness::new("en", Script::new());
    info(&mut h.client.voice, "aaaaaaaaaa");
    info(&mut h.client.voice, "bbbbb");
    h.clock.advance(0.1);
    h.client.voice.update();
    assert_eq!(h.spoken(), ["aaaaaaaaaa"]);
    // The screen reader says it has finished, 0.9 s before the estimate.
    h.speech.report(Progress::Ended(SpeechEnd::Finished));
    h.client.voice.update();
    assert_eq!(h.spoken(), ["aaaaaaaaaa", "bbbbb"]);
}

#[test]
fn a_report_that_speech_goes_on_keeps_the_next_message_waiting_past_the_estimate() {
    let mut h = Harness::new("en", Script::new());
    info(&mut h.client.voice, "aaaaaaaaaa");
    info(&mut h.client.voice, "bbbbb");
    h.speech.report(Progress::Speaking);
    run(&mut h, 2.0);
    assert_eq!(h.spoken(), ["aaaaaaaaaa"]);
    h.speech.report(Progress::Ended(SpeechEnd::Finished));
    h.client.voice.update();
    assert_eq!(h.spoken(), ["aaaaaaaaaa", "bbbbb"]);
}

#[test]
fn a_message_cancelled_at_the_screen_reader_is_dismissed_and_not_repeated() {
    let mut h = Harness::new("en", Script::new());
    info(&mut h.client.voice, "aaaaaaaaaa");
    info(&mut h.client.voice, "bbbbb");
    h.clock.advance(0.5);
    h.client.voice.update();
    // The player silences it at the screen reader; the key also reaches
    // the game, which says an item before the voice update.
    h.speech.report(Progress::Ended(SpeechEnd::Cancelled));
    h.client.voice.item(&t("x"));
    run(&mut h, 2.0);
    assert_eq!(h.spoken(), ["aaaaaaaaaa", "x", "bbbbb"]);
    let heard: Vec<Heard> = log(&h).into_iter().map(|(_, h)| h).collect();
    assert_eq!(heard, [Heard::Yes, Heard::Yes]);
}

/// Says a 3 s item, queues news with `queue` half a second in, then says
/// another item 1.5 s in (as arrowing to the next square does), and lets
/// the voice run. Returns everything spoken.
fn item_then_queued_then_item(queue: impl FnOnce(&mut Voice)) -> Vec<String> {
    let mut h = Harness::new("en", Script::new());
    h.client.voice.item(&t("a long description of 30 chars"));
    run(&mut h, 0.5);
    queue(&mut h.client.voice);
    run(&mut h, 1.0);
    h.client.voice.item(&t("next"));
    run(&mut h, 10.0);
    h.spoken()
}

#[test]
fn chat_queued_while_an_item_is_said_is_said_after_the_next_item() {
    let spoken = item_then_queued_then_item(|v| chat(v, "chat line"));
    assert_eq!(
        spoken,
        ["a long description of 30 chars", "next", "chat line"]
    );
}

#[test]
fn news_queued_while_an_item_is_said_is_said_after_the_next_item() {
    let spoken = item_then_queued_then_item(|v| info(v, "alert line"));
    assert_eq!(
        spoken,
        ["a long description of 30 chars", "next", "alert line"]
    );
}

#[test]
fn news_cut_off_after_more_than_a_second_is_said_again() {
    let mut h = Harness::new("en", Script::new());
    info(&mut h.client.voice, "news of thirty characters, 3 s");
    info(&mut h.client.voice, "more");
    run(&mut h, 1.5);
    h.client.voice.item(&t("x"));
    run(&mut h, 10.0);
    assert_eq!(
        h.spoken(),
        [
            "news of thirty characters, 3 s",
            "x",
            "news of thirty characters, 3 s",
            "more"
        ]
    );
}

/// A harness whose log holds, unheard: production "p1", alert "a1" at
/// square 1, production "p2", alert "a2" at square 2, chat "c1".
fn unheard_news() -> Harness {
    let mut h = Harness::new("en", Script::new());
    let v = &mut h.client.voice;
    let alert = |s: &str, square: u32| News::new(Category::Alert, t(s)).at(Some(SquareId(square)));
    v.news(News::new(Category::Production, t("p1")), Policy::Log);
    v.news(alert("a1", 1), Policy::Log);
    v.news(News::new(Category::Production, t("p2")), Policy::Log);
    v.news(alert("a2", 2), Policy::Log);
    v.news(News::new(Category::Chat, t("c1")), Policy::Log);
    h
}

/// What the voice says when `f` is done to it.
fn said<T>(h: &mut Harness, f: impl FnOnce(&mut Voice) -> T) -> String {
    let from = h.utterances().len();
    f(&mut h.client.voice);
    h.said_since(from).join(" | ")
}

#[test]
fn what_did_i_miss_counts_the_unheard_news_by_category_then_says_it_newest_alert_first() {
    let mut h = unheard_news();
    // Heard news does not count.
    tell(&mut h.client.voice, Category::Scouting, "s1");
    run(&mut h, 1.0);
    assert_eq!(
        said(&mut h, Voice::unheard),
        "alert 2 , production 2 , chat 1"
    );
    assert_eq!(said(&mut h, Voice::unheard), "a2");
    assert_eq!(said(&mut h, Voice::unheard), "p1");
    assert_eq!(said(&mut h, Voice::unheard), "a1");
    assert_eq!(said(&mut h, Voice::unheard), "p2");
    assert_eq!(said(&mut h, Voice::unheard), "c1");
    assert_eq!(said(&mut h, Voice::unheard), "nothing!");
    assert_eq!(said(&mut h, Voice::unheard), "nothing!");
    assert!(log(&h).iter().all(|(_, heard)| *heard == Heard::Yes));
}

#[test]
fn another_answer_makes_what_did_i_miss_count_again() {
    let mut h = unheard_news();
    assert_eq!(
        said(&mut h, Voice::unheard),
        "alert 2 , production 2 , chat 1"
    );
    assert_eq!(said(&mut h, Voice::unheard), "a2");
    h.client.voice.item(&t("a square"));
    assert_eq!(
        said(&mut h, Voice::unheard),
        "alert 1 , production 2 , chat 1"
    );
    assert_eq!(said(&mut h, Voice::unheard), "a1");
}

#[test]
fn the_review_keys_walk_one_category_and_say_the_category_when_it_changes() {
    let mut h = unheard_news();
    // Alerts first.
    assert_eq!(said(&mut h, Voice::review_previous), "a2");
    assert_eq!(said(&mut h, Voice::review_previous), "a1");
    assert_eq!(said(&mut h, Voice::review_previous), "a1");
    assert_eq!(said(&mut h, Voice::review_next), "a2");
    assert_eq!(said(&mut h, Voice::review_next), "a2");
    assert_eq!(said(&mut h, Voice::review_category), "production");
    assert_eq!(said(&mut h, Voice::review_previous), "p2");
    assert_eq!(said(&mut h, Voice::review_previous), "p1");
    assert_eq!(said(&mut h, Voice::review_category), "economy");
    assert_eq!(said(&mut h, Voice::review_previous), "economy nothing!");
    // What was reviewed is heard.
    assert_eq!(said(&mut h, Voice::unheard), "chat 1");
}

#[test]
fn the_newest_alert_with_a_square_comes_first_then_the_ones_before() {
    let h = unheard_news();
    let v = &h.client.voice;
    let (newest, square) = v.alert_before(None).unwrap();
    assert_eq!(square, SquareId(2));
    let (older, square) = v.alert_before(Some(newest)).unwrap();
    assert_eq!(square, SquareId(1));
    assert_eq!(v.alert_before(Some(older)), None);
}

#[test]
fn each_policy_does_what_it_says() {
    let mut h = Harness::new("en", Script::new());
    // The chat sound is a recorded sound, before the line.
    let line = || {
        let mut m = vec![MsgPart::Num(6114)];
        m.extend(t("hello"));
        News::new(Category::Chat, m)
    };
    assert_eq!(
        said(&mut h, |v| v.news(line(), Policy::Speak)),
        "<6114> hello"
    );
    run(&mut h, 2.0);
    assert_eq!(said(&mut h, |v| v.news(line(), Policy::Sound)), "<6114>");
    run(&mut h, 2.0);
    // Words without a sound are kept silently under the sound policy.
    let words = || News::new(Category::Chat, t("bye"));
    assert_eq!(said(&mut h, |v| v.news(words(), Policy::Sound)), "");
    assert_eq!(said(&mut h, |v| v.news(line(), Policy::Log)), "");
    assert_eq!(said(&mut h, |v| v.news(line(), Policy::Drop)), "");
    run(&mut h, 2.0);
    assert_eq!(
        log(&h),
        [
            ("6114 hello".to_owned(), Heard::Yes),
            ("6114 hello".to_owned(), Heard::No),
            ("bye".to_owned(), Heard::No),
            ("6114 hello".to_owned(), Heard::No),
        ]
    );
}

#[test]
fn a_cut_scene_message_not_spoken_does_not_wait() {
    let mut h = Harness::new("en", Script::new());
    let before = h.clock.now();
    let story = || News::new(Category::Story, t(LONG));
    h.client.voice.important_news(story(), Policy::Log);
    assert_eq!(h.clock.now(), before);
    assert_eq!(h.spoken(), Vec::<String>::new());
    h.client.voice.important_news(story(), Policy::Speak);
    assert!(h.clock.now() >= before + 5.0);
    assert_eq!(
        log(&h),
        [(LONG.to_owned(), Heard::No), (LONG.to_owned(), Heard::Yes)]
    );
}

#[test]
fn news_of_an_earlier_game_is_not_replaced_by_a_new_games() {
    let mut h = Harness::new("en", Script::new());
    let gold = |s: &str| News::new(Category::Economy, t(s)).about(Subject::Resource(0));
    let v = &mut h.client.voice;
    v.news(gold("10 gold"), Policy::Log);
    // The queue given up (as the game menu does) does not end the game.
    v.silent_flush();
    v.news(gold("11 gold"), Policy::Log);
    v.start_game();
    v.news(gold("20 gold"), Policy::Log);
    let texts: Vec<String> = log(&h).into_iter().map(|(s, _)| s).collect();
    assert_eq!(texts, ["11 gold", "20 gold"]);
}

#[test]
fn giving_up_the_queue_keeps_its_news_unheard() {
    let mut h = Harness::new("en", Script::new());
    info(&mut h.client.voice, "aaaaaaaaaa");
    info(&mut h.client.voice, "bbbbb");
    h.client.voice.silent_flush();
    run(&mut h, 3.0);
    assert_eq!(h.spoken(), ["aaaaaaaaaa"]);
    assert_eq!(
        log(&h),
        [
            ("aaaaaaaaaa".to_owned(), Heard::No),
            ("bbbbb".to_owned(), Heard::No)
        ]
    );
}
