//! The options menu's "sound and speech" submenu: the main volume, what the
//! player hears of each category of news, NVDA's report of the end of
//! speech, and the measure of the speech rate. Each item says its name, then
//! its value; Enter changes the value in place, and the change is saved at
//! once.

use soundrts_world::msg::{Msg, msg, nb2msg};
use soundrts_world::msgparts as mp;

use crate::client::Client;
use crate::input::{Event, InputPoint, Key};
use crate::menu::{Action, Choice, Exit, Menu};
use crate::message::{Category, Policy};
use crate::net::words::{self, text};
use crate::voice::category_title;

/// The fastest speech rate a measure may give, in seconds a character:
/// 125 characters a second, faster than any screen reader at its fastest.
pub const FASTEST: f64 = 0.008;
/// The slowest: 2.5 characters a second.
pub const SLOWEST: f64 = 0.4;

/// The submenu, until Back.
pub fn sound_and_speech_menu(cx: &mut Client) -> Result<(), Exit> {
    let mut choices = vec![volume_choice()];
    choices.extend(Category::ALL.into_iter().map(policy_choice));
    choices.push(nvda_direct_choice());
    choices.push(speech_rate_choice());
    choices.push(Choice::new(msg(mp::BACK), Action::CloseMenu));
    Menu::new(text(words::SOUND_AND_SPEECH), choices).run_loop(cx)
}

fn volume_choice() -> Choice {
    Choice {
        explanation: Some(text(words::VOLUME_KEYS)),
        ..Choice::setting(
            text(words::VOLUME),
            |cx| percent(cx.config.main_volume),
            |cx| {
                // Up by a tenth, and from the top back to silence.
                let v = match cx.config.main_volume {
                    100 => 0,
                    v => (v + 10).min(100),
                };
                cx.set_main_volume(v);
                Ok([nb2msg(i64::from(v)), msg(mp::PERCENT_VOLUME)].concat())
            },
        )
    }
}

fn percent(v: u8) -> Msg {
    [nb2msg(i64::from(v)), text(words::PERCENT)].concat()
}

/// What a policy means, said plainly.
fn policy_words(p: Policy) -> Msg {
    text(match p {
        Policy::Speak => words::SPOKEN,
        Policy::Sound => words::SOUND_ONLY,
        Policy::Log => words::LOG_ONLY,
        Policy::Drop => words::DROPPED,
    })
}

fn next_policy(p: Policy) -> Policy {
    match p {
        Policy::Speak => Policy::Sound,
        Policy::Sound => Policy::Log,
        Policy::Log => Policy::Drop,
        Policy::Drop => Policy::Speak,
    }
}

fn policy_choice(c: Category) -> Choice {
    Choice::setting(
        category_title(c),
        move |cx| policy_words(cx.config.news.of(c)),
        move |cx| {
            let p = next_policy(cx.config.news.of(c));
            cx.config.news.set(c, p);
            cx.save_config();
            Ok(policy_words(p))
        },
    )
}

fn on_off(on: bool) -> Msg {
    text(if on { words::ON } else { words::OFF })
}

fn nvda_direct_choice() -> Choice {
    Choice {
        explanation: Some(text(words::NVDA_DIRECT_EXPLANATION)),
        ..Choice::setting(
            text(words::NVDA_DIRECT),
            |cx| on_off(cx.config.nvda_direct != 0),
            |cx| {
                cx.config.nvda_direct = i64::from(cx.config.nvda_direct == 0);
                cx.save_config();
                Ok(on_off(cx.config.nvda_direct != 0))
            },
        )
    }
}

/// A speech rate in whole characters a second.
fn characters_a_second(seconds_a_character: f64) -> Msg {
    let n = (1.0 / seconds_a_character).round() as i64;
    [nb2msg(n), text(words::CHARACTERS_A_SECOND)].concat()
}

fn speech_rate_choice() -> Choice {
    Choice {
        explanation: Some(text(words::MEASURE_IT)),
        ..Choice::setting(
            text(words::SPEECH_RATE),
            |cx| characters_a_second(cx.config.wait_delay_per_character),
            measure_speech_rate,
        )
    }
}

/// What a measure of the speech rate found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Measure {
    /// Seconds a character, to a ten-thousandth.
    Rate(f64),
    /// Faster than [`FASTEST`]: the key cannot have come at the end.
    TooEarly,
    /// Slower than [`SLOWEST`].
    TooLate,
}

/// The speech rate if `sentence` took `seconds` to say.
pub fn measure(seconds: f64, sentence: &str) -> Measure {
    let rate = seconds / sentence.chars().count() as f64;
    if rate < FASTEST {
        Measure::TooEarly
    } else if rate > SLOWEST {
        Measure::TooLate
    } else {
        Measure::Rate((rate * 1e4).round() / 1e4)
    }
}

/// Waits for a key press: `None` for Escape. A key held since `held` was
/// pressed repeats; its repeats do not count until it is released.
fn next_key(cx: &mut Client, mut held: Option<Key>) -> Result<Option<Key>, Exit> {
    loop {
        cx.clock.sleep(0.001);
        cx.input.input_point(InputPoint::Menu, cx.voice.is_idle());
        match cx.input.poll() {
            Some(Event::Quit) => return Err(Exit),
            Some(Event::KeyUp(k)) if Some(k.physical) == held => held = None,
            Some(Event::KeyDown(k)) if Some(k.physical) == held => {}
            Some(Event::KeyDown(k)) if k.key == Key::Escape => return Ok(None),
            Some(Event::KeyDown(k)) => return Ok(Some(k.physical)),
            Some(Event::KeyUp(_)) | None => {}
        }
        cx.voice.update();
    }
}

/// Speaks a sentence of known length, and takes the time from the start of
/// the speech to the player's key at its end. A plausible rate is saved as
/// `wait_delay_per_character`, and the estimate of the end of speech uses
/// it at once. Returns what to say of the result.
fn measure_speech_rate(cx: &mut Client) -> Result<Msg, Exit> {
    let nothing_changed = text(words::NOTHING_CHANGED);
    cx.voice.item(&text(words::MEASURE_HOW));
    let Some(start_key) = next_key(cx, None)? else {
        return Ok(nothing_changed);
    };
    let start = cx.clock.now();
    cx.voice.item(&text(words::MEASURE_SENTENCE));
    if next_key(cx, Some(start_key))?.is_none() {
        return Ok(nothing_changed);
    }
    let seconds = cx.clock.now() - start;
    let why = match measure(seconds, words::MEASURE_SENTENCE) {
        Measure::Rate(rate) => return Ok(measured(cx, seconds, rate)),
        Measure::TooEarly => words::TOO_EARLY,
        Measure::TooLate => words::TOO_LATE,
    };
    cx.log.info(format!(
        "speech rate not measured: {seconds:.3} s for the sentence"
    ));
    Ok([
        text(why),
        msg(mp::COMMA),
        nothing_changed,
        msg(mp::PERIOD),
        text(words::TRY_AGAIN),
    ]
    .concat())
}

/// Saves a measured rate, and says it.
fn measured(cx: &mut Client, seconds: f64, rate: f64) -> Msg {
    cx.log.info(format!(
        "speech rate measured: {seconds:.3} s for the sentence, {rate} s a character"
    ));
    cx.set_wait_delay_per_character(rate);
    [
        text(words::MEASURED),
        characters_a_second(rate),
        msg(mp::PERIOD),
        text(words::MEASURE_AGAIN),
    ]
    .concat()
}
