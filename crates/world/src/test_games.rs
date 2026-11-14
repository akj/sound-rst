//! Whole games for tests, and what is done to them on the way: allies who
//! share control, a seat in cheat mode then defeated, mods, sixteen
//! computers. Only the `truth` feature has them: they change the world
//! directly.
//!
//! A game's texts are the original's files, which the caller reads: this
//! crate touches no file.

use crate::command::{Cheat, Command};
use crate::entity::{Place, PlayerId};
use crate::player::{Controller, ControllerKind};
use crate::setup::GameSetup;
use crate::world::{World, multiplayer_default_triggers};

/// A game to play, and what to do to it on the way.
pub struct Case {
    pub name: &'static str,
    /// The original's rules files, in layer order.
    pub rules: Vec<&'static str>,
    /// The original's computer player scripts, in layer order.
    pub ai: Vec<&'static str>,
    /// The original's map file.
    pub map: &'static str,
    pub seed: i64,
    pub controllers: Vec<Controller>,
    /// The turns to play, at most: a game can end sooner.
    pub turns: usize,
    /// Every player shares control of its units with its allies.
    pub shared_control: bool,
    /// Changes made to the game at the start of a turn, before it is
    /// played: the world and the turn, from 0.
    pub script: fn(&mut World, usize),
}

impl Case {
    /// The game's set-up, its files read by `read`, which takes a path
    /// relative to the original game's folder.
    pub fn setup(&self, read: impl Fn(&str) -> String) -> GameSetup {
        let join = |files: &[&str]| files.iter().map(|f| read(f)).collect::<Vec<_>>().join("\n");
        GameSetup {
            rules: join(&self.rules),
            ai: join(&self.ai),
            map: read(self.map),
            seed: self.seed,
            controllers: self.controllers.clone(),
            default_triggers: multiplayer_default_triggers(),
            allow_cheatmode: true,
            random_starts: true,
            equivalents: true,
        }
    }

    /// The game's world before its first turn.
    pub fn world(&self, read: impl Fn(&str) -> String) -> World {
        let (mut w, _) = self
            .setup(read)
            .build_world()
            .unwrap_or_else(|e| panic!("{}: {e}", self.name));
        if self.shared_control {
            let players: Vec<PlayerId> = w.all_players().map(|(p, _)| p).collect();
            for p in players {
                let allied = w.player(p).allied.clone();
                w.player_mut(p).allied_control = allied;
            }
        }
        w
    }
}

/// Makes every player that is not a person receive events, as a local
/// human does, so that every seat is told a turn's events as a host that
/// runs the game will tell them. Only whether events are recorded changes:
/// recording reads the world and changes nothing else, and a computer
/// stays a computer, as its player's kind says.
pub fn tell_every_seat(w: &mut World) {
    let players: Vec<PlayerId> = w.all_players().map(|(p, _)| p).collect();
    for p in players {
        let pl = w.player_mut(p);
        if !matches!(pl.controller.kind, ControllerKind::Human { .. }) {
            let login = pl.controller.login();
            pl.controller.kind = ControllerKind::Human { login, local: true };
        }
    }
}

/// A computer player of `ai_type` in `alliance`.
pub fn computer(ai_type: &str, alliance: &str) -> Controller {
    let mut c = Controller::computer(ai_type);
    c.alliance = Some(alliance.into());
    c
}

pub fn with_faction(mut c: Controller, faction: &str) -> Controller {
    c.faction = faction.into();
    c
}

fn nothing(_: &mut World, _: usize) {}

/// Builds a `kind` of `p` on an exit of the square of one of its units,
/// makes it block the exit, and puts a unit of `p` inside it.
pub fn shelter(w: &mut World, p: PlayerId, kind: &str) {
    let units = w.player(p).units.clone();
    let Some((square, exit)) = units.iter().find_map(|&u| {
        let q = w.entity(u)?.square()?;
        Some((q, *w.square(q).exits.first()?))
    }) else {
        panic!("no exit beside a unit of player {}", p.0);
    };
    let (x, y) = {
        let e = w.entity(exit).unwrap();
        (e.x, e.y)
    };
    let t = w.rules.type_id(kind).unwrap();
    let Ok(b) = w.create_creature(t, Some(p), Place::Square(square), x, y) else {
        panic!("no room for a {kind} on square {}", square.0);
    };
    w.block(b, exit);
    let inside = units.iter().copied().find(|&u| {
        w.entity(u)
            .is_some_and(|e| e.class.is_unit() && !e.is_inside())
    });
    w.load(b, inside.expect("a unit to put inside"));
}

/// Two alliances of computers on jl7, each player sharing control of its
/// units with its allies.
pub fn allies_sharing_control(turns: usize) -> Case {
    Case {
        name: "jl7, allies sharing control",
        rules: vec!["res/rules.txt"],
        ai: vec!["res/ai.txt"],
        map: "res/multi/jl7.txt",
        seed: 7,
        controllers: vec![
            computer("aggressive", "1"),
            computer("easy", "1"),
            computer("ai2", "1"),
            computer("aggressive", "2"),
            computer("easy", "2"),
            computer("aggressive", "2"),
        ],
        turns,
        shared_control: true,
        script: |w, turn| {
            if turn == 30 {
                // An ally's unit inside an ally's gate: controlled, not
                // perceived. An enemy's wall closes another exit.
                shelter(w, PlayerId(0), "gate");
                shelter(w, PlayerId(3), "wall");
            }
            if turn == 40 {
                // Groups are shared by every player: each seat recalls
                // only the units it controls.
                for (p, group) in [(0, "1"), (3, "1"), (1, "2")] {
                    let units = w.player(PlayerId(p)).units.clone();
                    let ids: Vec<String> = units.iter().take(3).map(|u| u.0.to_string()).collect();
                    w.queue_command(PlayerId(p), &format!("control {}", ids.join(" ")));
                    w.queue_command(PlayerId(p), &format!("order 0 0 join_group {group}"));
                }
            }
        },
    }
}

/// A person on jl7 who turns cheat mode on with a unit in a gate, then
/// off, and has everything for a while, against two computers, and is
/// defeated while they play on.
pub fn cheat_mode_and_defeat(turns: usize) -> Case {
    fn script(w: &mut World, turn: usize) {
        let me = PlayerId(0);
        match turn {
            20 | 400 => w.queue(me, Command::Cheat(Cheat::ToggleMode)),
            30 => shelter(w, me, "gate"),
            200 => w.player_mut(me).cheat_has_all = true,
            300 => w.player_mut(me).cheat_has_all = false,
            700 => w.defeat(me, false),
            _ => {}
        }
        if (450..=700).contains(&turn) && turn.is_multiple_of(5) {
            // Woods are cut, in sight and in the fog: the terrain changes.
            for q in 0..w.squares.len() {
                let wood = w.squares[q]
                    .objects
                    .iter()
                    .copied()
                    .find(|&o| w.entity(o).is_some_and(|e| &*e.type_name == "wood"));
                if let Some(wood) = wood {
                    w.die(wood, None).unwrap();
                }
            }
        }
    }
    let mut me = Controller::human("me", true);
    me.alliance = Some("1".into());
    Case {
        name: "jl7, cheat mode and defeat",
        rules: vec!["res/rules.txt"],
        ai: vec!["res/ai.txt"],
        map: "res/multi/jl7.txt",
        seed: 1,
        controllers: vec![me, computer("aggressive", "2"), computer("easy", "3")],
        turns,
        shared_control: false,
        script,
    }
}

/// Orcs and humans with the teleportation mod's abilities on jl7, every
/// player alone.
pub fn mods(turns: usize) -> Case {
    Case {
        name: "jl7, orc and teleportation mods",
        rules: vec![
            "res/rules.txt",
            "mods/orc/rules.txt",
            "mods/teleportation/rules.txt",
        ],
        ai: vec!["res/ai.txt", "mods/orc/ai.txt"],
        map: "res/multi/jl7.txt",
        seed: 5,
        controllers: vec![
            with_faction(computer("aggressive", "1"), "orc_faction"),
            with_faction(computer("easy", "2"), "orc_faction"),
            with_faction(computer("aggressive", "3"), "human_faction"),
            computer("ai2", "4"),
        ],
        turns,
        shared_control: false,
        script: nothing,
    }
}

/// `players` computers on a multiplayer map, every one alone, seed 1: the
/// aggressive, `ai2` and easy types in turn.
pub fn every_one_alone(
    name: &'static str,
    map: &'static str,
    players: usize,
    turns: usize,
) -> Case {
    let types = ["aggressive", "ai2", "easy"];
    Case {
        name,
        rules: vec!["res/rules.txt"],
        ai: vec!["res/ai.txt"],
        map,
        seed: 1,
        controllers: (0..players)
            .map(|i| computer(types[i % 3], &(i + 1).to_string()))
            .collect(),
        turns,
        shared_control: false,
        script: nothing,
    }
}

/// Sixteen computers on js13, every one alone.
pub fn sixteen(turns: usize) -> Case {
    every_one_alone("js13, 16 computers", "res/multi/js13.txt", 16, turns)
}

/// A case being played, turn by turn, with what every seat is told.
pub struct Game {
    world: World,
    players: Vec<PlayerId>,
    script: fn(&mut World, usize),
    turns: usize,
    turn: usize,
}

impl Game {
    /// The case's game before its first turn. With `every_seat_hears`,
    /// every seat is told the events (see [`tell_every_seat`]).
    pub fn new(case: &Case, read: impl Fn(&str) -> String, every_seat_hears: bool) -> Game {
        let mut world = case.world(read);
        if every_seat_hears {
            tell_every_seat(&mut world);
        }
        Game {
            players: world.all_players().map(|(p, _)| p).collect(),
            world,
            script: case.script,
            turns: case.turns,
            turn: 0,
        }
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    /// Every player of the game, in id order.
    pub fn players(&self) -> &[PlayerId] {
        &self.players
    }

    /// What every seat is sent once, at the start.
    pub fn start(&self) -> crate::message::GameStart {
        crate::message::GameStart::of(&self.world)
    }

    /// What each player is told of the game as it is now, without events,
    /// in the order of [`Game::players`]: what a seat that joins now is
    /// sent.
    pub fn messages(&self) -> Vec<crate::message::TurnMessage> {
        let w = &self.world;
        self.players
            .iter()
            .map(|&p| w.turn_message(p, Vec::new()))
            .collect()
    }

    /// Plays a turn, and gives what each player is told of it, in the
    /// order of [`Game::players`]. `None` once the case's turns are played
    /// or nobody is left playing.
    pub fn step(&mut self) -> Option<Vec<crate::message::TurnMessage>> {
        if self.turn >= self.turns || self.world.true_playing_players().is_empty() {
            return None;
        }
        (self.script)(&mut self.world, self.turn);
        self.turn += 1;
        self.world.update();
        let w = &mut self.world;
        Some(
            self.players
                .iter()
                .map(|&p| {
                    let events = w
                        .drain_events(p)
                        .into_iter()
                        .filter(|e| !matches!(e, crate::events::Event::Voila(_)))
                        .collect();
                    w.turn_message(p, events)
                })
                .collect(),
        )
    }

    /// What `p` is sent when the game ends for it.
    pub fn game_end(&self, p: PlayerId) -> crate::message::GameEnd {
        self.world.game_end(p)
    }
}
