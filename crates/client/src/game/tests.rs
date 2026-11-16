//! Nothing said stops the loop: the world goes on while news is said, and
//! in single-player it waits only on purpose, while a story is told or
//! the game menu or the console is open.

use soundrts_protocol::{Computer, Login, Skill, SlotPlayer, Speed};
use soundrts_world::msg::{Msg, MsgPart};

use super::testing::training;
use super::{Mode, SrvEvent};
use crate::clock::Clock;
use crate::input::{Key, Mods};
use crate::message::{Category, News};
use crate::testkit::{FakeStream, Harness, Script, TestGame, slot};
use crate::voice::Heard;

fn t(s: &str) -> Msg {
    vec![MsgPart::from(s)]
}

/// Runs the loop until `n` utterances have started since the `from`th.
fn run_until_said(h: &mut Harness, g: &mut TestGame, from: usize, n: usize) {
    let end = h.clock.now() + 60.0;
    while h.said_since(from).len() < n {
        assert!(h.clock.now() < end, "said only {:?}", h.said_since(from));
        h.step(g);
    }
}

/// Runs the loop while the voice says the message it is saying, until
/// the next one starts or it has nothing left to say: the turns played
/// meanwhile.
fn turns_while_said(h: &mut Harness, g: &mut TestGame) -> i64 {
    let turn = g.game.view().turn();
    let n = h.utterances().len();
    while h.utterances().len() == n && !h.client.voice.is_idle() {
        h.step(g);
    }
    g.game.view().turn() - turn
}

fn story(texts: &[&str]) -> SrvEvent {
    SrvEvent::Sequence(texts.iter().map(|s| s.to_string()).collect())
}

#[test]
fn an_important_message_is_said_after_the_news_being_said_and_before_the_news_waiting_while_the_world_goes_on()
 {
    let (mut h, mut g) = training();
    h.run_until_voice_idle(&mut g);
    let from = h.utterances().len();
    h.client
        .news(News::new(Category::Production, t("the news being said")));
    h.client
        .news(News::new(Category::Production, t("news waiting")));
    let before = h.clock.now();
    let important = SrvEvent::VoiceImportant(t("an important message"));
    g.game.process_srv_event(&mut h.client, important);
    assert_eq!(h.clock.now(), before, "nothing waits");
    run_until_said(&mut h, &mut g, from, 2);
    // 20 characters: 2 seconds, nearly 7 turns.
    let turns = turns_while_said(&mut h, &mut g);
    assert!(turns >= 6, "{turns} turns while it was said");
    h.run_until_voice_idle(&mut g);
    assert_eq!(
        h.said_since(from),
        [
            "the news being said",
            "an important message",
            "news waiting"
        ]
    );
    let kept = h
        .client
        .voice
        .log()
        .find(|e| e.parts() == &t("an important message"))
        .unwrap();
    assert_eq!(kept.category(), Some(Category::System));
}

#[test]
fn a_cut_scene_stops_the_world_but_not_the_keys_and_once_skipped_the_world_goes_on() {
    let (mut h, mut g) = training();
    h.run_until_voice_idle(&mut g);
    let from = h.utterances().len();
    g.game
        .srv_queue
        .push_front(story(&["once upon a time", "the end"]));
    h.step(&mut g);
    assert!(matches!(g.game.mode, Mode::Story(_)));
    assert!(g.game.world_waits());
    let turn = g.game.view().turn();
    h.run_for(&mut g, 1.0);
    assert_eq!(g.game.view().turn(), turn, "the world waits");
    // The game menu opens during the story, and Escape goes back to it.
    h.press(Key::F10, Mods::NONE);
    h.step(&mut g);
    assert!(matches!(g.game.mode, Mode::GameMenu(_)));
    h.press(Key::Escape, Mods::NONE);
    h.step(&mut g);
    assert!(matches!(g.game.mode, Mode::Story(_)));
    // The message cut off by the menu is said again.
    run_until_said(&mut h, &mut g, from, 4);
    assert_eq!(g.game.view().turn(), turn, "the world still waits");
    // A key skips each message; once the last is skipped, the world goes
    // on.
    h.press(Key::Down, Mods::NONE);
    run_until_said(&mut h, &mut g, from, 5);
    h.press(Key::Down, Mods::NONE);
    h.step(&mut g);
    h.step(&mut g);
    assert!(matches!(g.game.mode, Mode::Playing));
    h.run_for(&mut g, 1.0);
    assert!(g.game.view().turn() > turn);
    assert_eq!(
        h.said_since(from),
        [
            "once upon a time",
            "Menu",
            "continue game",
            "once upon a time",
            "the end"
        ]
    );
}

#[test]
fn escape_skips_the_rest_of_a_story_which_stays_in_the_story_log() {
    let (mut h, mut g) = training();
    h.run_until_voice_idle(&mut g);
    let from = h.utterances().len();
    g.game
        .srv_queue
        .push_front(story(&["once upon a time", "there was", "the end"]));
    h.step(&mut g);
    run_until_said(&mut h, &mut g, from, 1);
    let turn = g.game.view().turn();
    h.press(Key::Escape, Mods::NONE);
    h.step(&mut g);
    h.step(&mut g);
    assert!(matches!(g.game.mode, Mode::Playing));
    h.run_for(&mut g, 1.0);
    assert!(g.game.view().turn() > turn, "the world goes on");
    assert_eq!(h.said_since(from), ["once upon a time"]);
    // The story's lines can still be reviewed.
    let story: Vec<String> = h
        .client
        .voice
        .log()
        .filter(|e| e.category() == Some(Category::Story))
        .map(|e| format!("{:?}", e.parts()))
        .collect();
    assert_eq!(
        story[story.len() - 3..],
        [
            format!("{:?}", t("once upon a time")),
            format!("{:?}", t("there was")),
            format!("{:?}", t("the end"))
        ]
    );
    // Slash goes from the alerts to the story; Comma says its newest.
    for _ in 0..6 {
        h.press(Key::Slash, Mods::NONE);
        h.step(&mut g);
    }
    let from = h.utterances().len();
    h.press(Key::Comma, Mods::NONE);
    h.step(&mut g);
    assert_eq!(h.said_since(from), ["the end"]);
}

#[test]
fn control_shift_c_copies_during_a_story_without_skipping_a_line() {
    let (mut h, mut g) = training();
    h.run_until_voice_idle(&mut g);
    let from = h.utterances().len();
    g.game
        .srv_queue
        .push_front(story(&["once upon a time", "the end"]));
    h.step(&mut g);
    run_until_said(&mut h, &mut g, from, 1);
    h.press(
        Key::C,
        Mods {
            shift: true,
            ..Mods::CTRL
        },
    );
    h.step(&mut g);
    assert!(matches!(g.game.mode, Mode::Story(_)));
    h.run_until_voice_idle(&mut g);
    // The line cut off by the answer is said again.
    assert_eq!(
        h.said_since(from),
        ["once upon a time", "copied", "once upon a time", "the end"]
    );
    assert_eq!(
        h.desktop
            .clipboard
            .lock()
            .unwrap()
            .last()
            .map(String::as_str),
        Some("once upon a time")
    );
}

#[test]
fn escape_during_a_story_still_lets_the_news_waiting_with_it_be_said() {
    let (mut h, mut g) = training();
    h.run_until_voice_idle(&mut g);
    let from = h.utterances().len();
    g.game
        .srv_queue
        .push_front(story(&["once upon a time", "there was", "the end"]));
    h.step(&mut g);
    run_until_said(&mut h, &mut g, from, 1);
    h.client
        .news(News::new(Category::Production, t("news waiting")));
    h.press(Key::Escape, Mods::NONE);
    h.step(&mut g);
    h.step(&mut g);
    assert!(matches!(g.game.mode, Mode::Playing));
    h.run_until_voice_idle(&mut g);
    // The lines skipped are not started.
    assert_eq!(h.said_since(from), ["once upon a time", "news waiting"]);
    let story: Vec<Heard> = h
        .client
        .voice
        .log()
        .filter(|e| e.category() == Some(Category::Story))
        .map(|e| e.heard())
        .collect();
    assert_eq!(story[story.len() - 3..], [Heard::No; 3]);
}

#[test]
fn escape_after_walking_the_history_during_a_story_says_no_line_of_it() {
    let (mut h, mut g) = training();
    h.run_until_voice_idle(&mut g);
    let from = h.utterances().len();
    g.game
        .srv_queue
        .push_front(story(&["once upon a time", "there was", "the end"]));
    h.step(&mut g);
    run_until_said(&mut h, &mut g, from, 1);
    h.client
        .news(News::new(Category::Production, t("news waiting")));
    h.press(Key::F5, Mods::NONE);
    h.step(&mut g);
    h.press(Key::Escape, Mods::NONE);
    h.step(&mut g);
    h.step(&mut g);
    assert!(matches!(g.game.mode, Mode::Playing));
    h.run_until_voice_idle(&mut g);
    let said = h.said_since(from);
    assert_eq!(said[0], "once upon a time");
    assert_eq!(said.last().map(String::as_str), Some("news waiting"));
    assert!(
        !said.iter().any(|s| s == "there was" || s == "the end"),
        "{said:?}"
    );
}

#[test]
fn escape_during_a_story_stops_an_answer_being_said() {
    let (mut h, mut g) = training();
    h.run_until_voice_idle(&mut g);
    let from = h.utterances().len();
    g.game
        .srv_queue
        .push_front(story(&["once upon a time", "the end"]));
    h.step(&mut g);
    run_until_said(&mut h, &mut g, from, 1);
    h.press(Key::F1, Mods::NONE);
    h.step(&mut g);
    assert!(h.client.voice.is_busy(), "the help is said");
    h.press(Key::Escape, Mods::NONE);
    h.step(&mut g);
    assert!(!h.client.voice.is_busy(), "Escape stops it");
    h.run_until_voice_idle(&mut g);
    assert_eq!(h.said_since(from).len(), 2, "{:?}", h.said_since(from));
}

#[test]
fn the_game_menu_and_the_console_do_not_stop_the_loop_and_the_world_waits_while_they_are_open() {
    // Were the loop held in a menu, the window would close at the
    // deadline and the step fail.
    let mut h = Harness::new("en", Script::new().keep_open().deadline(1000.0));
    let mut g = h.start_training("jl1", &["quiet"], 1);
    h.run_until_turn(&mut g, 3);
    h.run_until_voice_idle(&mut g);

    let turn = g.game.view().turn();
    h.press(Key::F10, Mods::NONE);
    h.step(&mut g);
    assert!(matches!(g.game.mode, Mode::GameMenu(_)));
    h.run_for(&mut g, 2.0);
    assert_eq!(g.game.view().turn(), turn, "the world waits");
    h.press(Key::Escape, Mods::NONE);
    h.step(&mut g);
    assert!(matches!(g.game.mode, Mode::Playing));
    h.run_for(&mut g, 1.0);
    assert!(g.game.view().turn() > turn);

    let turn = g.game.view().turn();
    let gold = g.game.resources()[0];
    h.press(Key::Backquote, Mods::NONE);
    h.step(&mut g);
    assert!(matches!(g.game.mode, Mode::Console(_)));
    h.run_for(&mut g, 2.0);
    assert_eq!(g.game.view().turn(), turn, "the world waits");
    let from = h.utterances().len();
    h.press(Key::R, Mods::NONE);
    h.step(&mut g);
    h.press(Key::Return, Mods::NONE);
    h.step(&mut g);
    assert!(matches!(g.game.mode, Mode::Playing));
    assert_eq!(h.said_since(from), ["r . r", "r"]);
    assert_eq!(g.game.resources()[0], gold + 1000);
    h.run_for(&mut g, 1.0);
    assert!(g.game.view().turn() > turn);
}

#[test]
fn on_a_server_neither_a_cut_scene_nor_an_important_message_stops_the_turns() {
    let mut h = Harness::new("en", Script::new().keep_open());
    let server = h.join_fake_server("andrew", &[]);
    let slots = [
        slot(SlotPlayer::Human(Login::new("andrew").unwrap()), 1),
        slot(SlotPlayer::Computer(Computer::Quiet(Skill::NORMAL)), 2),
    ];
    let mut stream = FakeStream::new(&mut h, "jl1", &slots, 0, 1);
    let mut g = h.join_fake_game(&server, &stream, "jl1", Speed::Normal);
    let t0 = h.clock.now();
    for n in 0..100 {
        for frame in stream.turn(Vec::new()) {
            server.frame_at(t0 + 0.3 * f64::from(n), frame);
        }
    }
    h.run_for(&mut g, 2.0);
    h.run_until_voice_idle(&mut g);
    let from = h.utterances().len();
    g.game.srv_queue.push_back(story(&["once upon a time"]));
    g.game
        .srv_queue
        .push_back(SrvEvent::VoiceImportant(t("an important message")));
    run_until_said(&mut h, &mut g, from, 1);
    assert!(!g.game.world_waits());
    // 16 characters: 1.6 seconds, more than 5 turns.
    let turns = turns_while_said(&mut h, &mut g);
    assert!(turns >= 4, "{turns} turns while the story was told");
    assert_eq!(
        h.said_since(from),
        ["once upon a time", "an important message"]
    );
    let turns = turns_while_said(&mut h, &mut g);
    assert!(turns >= 5, "{turns} turns while the message was said");
}

#[test]
fn cancelling_the_game_from_the_menu_during_a_cut_scene_ends_the_game() {
    let (mut h, mut g) = training();
    h.run_until_voice_idle(&mut g);
    g.game
        .srv_queue
        .push_front(story(&["once upon a time there was a long story"]));
    h.step(&mut g);
    assert!(matches!(g.game.mode, Mode::Story(_)));
    for key in [Key::F10, Key::Down, Key::Return] {
        h.press(key, Mods::NONE);
        h.step(&mut g);
    }
    assert!(matches!(g.game.mode, Mode::Playing));
    h.run_for(&mut g, 2.0);
    assert!(g.game.end_loop, "the game is over");
}

#[test]
fn key_help_turns_on_during_a_story_and_the_story_is_still_said() {
    let (mut h, mut g) = training();
    h.run_until_voice_idle(&mut g);
    g.game
        .srv_queue
        .push_front(story(&["once upon a time", "the end"]));
    h.step(&mut g);
    assert!(matches!(g.game.mode, Mode::Story(_)));
    let from = h.utterances().len();
    h.press(Key::F1, Mods::CTRL);
    h.step(&mut g);
    assert!(matches!(g.game.mode, Mode::KeyHelp));
    assert!(!g.game.world_waits());
    run_until_said(&mut h, &mut g, from, 3);
    assert_eq!(
        h.said_since(from)[..3],
        [
            "key help on: press a key to hear what it does, Escape to leave",
            "once upon a time",
            "the end"
        ]
    );
}

/// What the interface knows of a turn: its message from the moment the
/// turn is taken, its own copies of the entities once the end of the turn
/// is handled. On the turn that ends the game, the end of the turn is as
/// it was before the players left, and the answers as they are after.
#[test]
fn the_last_turn_ends_before_the_players_leave_and_is_answered_after() {
    use soundrts_world::{Cheat, Command};
    let (mut h, mut g) = training();
    h.run_until_voice_idle(&mut g);
    let me = g.game.me;
    g.game
        .send_command(&h.client, Command::Cheat(Cheat::Victory));
    let handled = g.game.turns_handled;
    while !g.game.view().players().is_empty() {
        h.step(&mut g);
    }
    // Taken: every player has left, and the turn's end waits, then the
    // player's leaving.
    assert_eq!(g.game.turns_handled, handled);
    assert!(g.game.view().has_victory());
    assert!(g.game.view().own_units().all(|u| u.place.is_none()));
    assert!(matches!(g.game.srv_queue.back(), Some(SrvEvent::Quit)));
    while g.game.turns_handled == handled {
        h.step(&mut g);
    }
    // The end of the turn handled: the player's units are still there.
    let mine = |g: &TestGame| {
        g.game
            .dobjets
            .values()
            .filter(|v| v.player() == Some(me) && v.place().is_some())
            .count()
    };
    assert!(mine(&g) > 0);
    assert!(!g.game.end_loop);
    h.step(&mut g);
    assert!(g.game.end_loop, "the player has left");
    // The statistics come with the end of the game.
    assert!(g.game.view().stats().is_none());
    g.game.end_game();
    assert!(g.game.view().stats().is_some());
}

/// Playing as another player: what the interface reads is that player's
/// at once, from a seat that remembers nothing yet.
#[test]
fn playing_as_another_player_starts_a_new_seat_at_once() {
    use soundrts_world::Cheat;
    let (mut h, mut g) = training();
    h.run_until_voice_idle(&mut g);
    let me = g.game.me;
    let other = *g.game.view().players().iter().find(|&&p| p != me).unwrap();
    let home = g.game.place.unwrap();
    assert_ne!(g.game.view().terrain(home), "");
    let turn = g.game.view().turn();
    g.game.cheat_at_once(Cheat::PlayAs(other));
    assert_eq!((g.game.me, g.game.view().player()), (other, other));
    assert_eq!(g.game.view().turn(), turn);
    assert_eq!(g.game.view().terrain(home), "");
    let theirs = g.game.host().world().player(other).units.clone();
    let told: Vec<_> = g.game.view().own_units().map(|u| u.id).collect();
    assert_eq!(told, theirs);
}
