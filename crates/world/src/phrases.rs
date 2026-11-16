//! What the code says and plays, by name: the phrases, the sounds the code
//! plays by itself, and the pauses.
//!
//! [`Phrase`] is the one list of the phrases the code says. Each has an id,
//! its name in snake case, and its English words, here beside it. A
//! translation gives other words for an id. To add a phrase, add one line.
//! Reword the English freely. When the meaning changes, change the name, so
//! that the translations of the old meaning do not follow.
//!
//! A comment `///` above a phrase is its note for translators
//! ([`Phrase::note`]). Most phrases are a fragment of a sentence, and the
//! note is what tells two fragments with the same English apart.

use std::collections::HashMap;
use std::sync::OnceLock;

/// Declares [`Phrase`] from lines `Name = "English words",` in two groups:
/// the phrases of the original, and the port's own.
macro_rules! phrases {
    (
        original { $( $(#[doc = $original_note:literal])* $original:ident = $original_english:literal, )* }
        own { $( $(#[doc = $own_note:literal])* $own:ident = $own_english:literal, )* }
    ) => {
        /// A phrase the code says. A message that is sent or saved names
        /// it by its id, so the order of the list is free to change.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum Phrase {
            $( $(#[doc = $original_note])* $original, )*
            $( $(#[doc = $own_note])* $own, )*
        }

        impl Phrase {
            /// The phrases that the original has. Its translations hold
            /// their words, under the number that
            /// `crates/client/src/original_names.txt` gives the id.
            pub const ORIGINAL: &'static [Phrase] = &[ $( Phrase::$original, )* ];

            /// The phrases that the port added. No translation has them yet.
            pub const OWN: &'static [Phrase] = &[ $( Phrase::$own, )* ];

            /// The English words.
            pub fn english(self) -> &'static str {
                match self {
                    $( Phrase::$original => $original_english, )*
                    $( Phrase::$own => $own_english, )*
                }
            }

            /// The note for translators. Empty for a phrase without one.
            pub fn note(self) -> &'static str {
                match self {
                    $( Phrase::$original => concat!($( $original_note, "\n", )*).trim(), )*
                    $( Phrase::$own => concat!($( $own_note, "\n", )*).trim(), )*
                }
            }

            /// The name as the code writes it: `AwaitingYourOrders`.
            fn name(self) -> &'static str {
                match self {
                    $( Phrase::$original => stringify!($original), )*
                    $( Phrase::$own => stringify!($own), )*
                }
            }
        }
    };
}

impl Phrase {
    /// Every phrase: the original's, then the port's own.
    pub fn all() -> impl Iterator<Item = Phrase> {
        Phrase::ORIGINAL.iter().chain(Phrase::OWN).copied()
    }

    /// The id, which a translation is keyed by: the name in snake case,
    /// `awaiting_your_orders`.
    pub fn id(self) -> &'static str {
        static IDS: OnceLock<Vec<String>> = OnceLock::new();
        let ids = IDS.get_or_init(|| Phrase::all().map(|p| snake_case(p.name())).collect());
        // The phrases are declared in the order of `all`.
        ids.get(self as usize).map_or("", String::as_str)
    }

    /// The phrase with this id.
    pub fn from_id(id: &str) -> Option<Phrase> {
        static PHRASES: OnceLock<HashMap<&'static str, Phrase>> = OnceLock::new();
        let phrases = PHRASES.get_or_init(|| Phrase::all().map(|p| (p.id(), p)).collect());
        phrases.get(id).copied()
    }

    /// The letter of a column of the map, from 0 for "a". None past "z".
    pub fn letter(column: usize) -> Option<Phrase> {
        use Phrase::*;
        const LETTERS: [Phrase; 26] = [
            LetterA, LetterB, LetterC, LetterD, LetterE, LetterF, LetterG, LetterH, LetterI,
            LetterJ, LetterK, LetterL, LetterM, LetterN, LetterO, LetterP, LetterQ, LetterR,
            LetterS, LetterT, LetterU, LetterV, LetterW, LetterX, LetterY, LetterZ,
        ];
        LETTERS.get(column).copied()
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Phrase {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.id())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Phrase {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = <String as serde::Deserialize>::deserialize(deserializer)?;
        Phrase::from_id(&id).ok_or_else(|| serde::de::Error::custom("no phrase has this id"))
    }
}

/// `AwaitingYourOrders` as `awaiting_your_orders`.
fn snake_case(name: &str) -> String {
    let mut id = String::with_capacity(name.len() + 4);
    for c in name.chars() {
        if c.is_ascii_uppercase() && !id.is_empty() {
            id.push('_');
        }
        id.push(c.to_ascii_lowercase());
    }
    id
}

/// A sound the code plays by itself, not because a style names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Cue {
    /// An order that is impossible, a key that does nothing.
    Beep,
    /// Where a square or an object is, by its place between the ears.
    PositionalBeep,
    /// A menu moved to another item.
    Select,
    /// A menu item was chosen.
    Confirm,
    /// An ally's flare.
    Flare,
    /// Before the title of a map that is not one of the game's.
    CustomMap,
    /// The first of the two sounds that stand for the clock's bell.
    BellStrike,
    /// Before a line of chat: a sound rarely heard in a game.
    Chat,
}

impl Cue {
    pub const ALL: [Cue; 8] = [
        Cue::Beep,
        Cue::PositionalBeep,
        Cue::Select,
        Cue::Confirm,
        Cue::Flare,
        Cue::CustomMap,
        Cue::BellStrike,
        Cue::Chat,
    ];

    /// The name of the sound: its path under `sounds/`. It says what the
    /// sound is, so two cues can have the sound of something else.
    pub fn sound(self) -> &'static str {
        match self {
            Cue::Beep => "ui/beep",
            Cue::PositionalBeep => "ui/positional_beep",
            Cue::Select => "ui/select",
            Cue::Confirm => "ui/confirm",
            Cue::Flare => "magic/holy_vision",
            Cue::CustomMap => "event/healed",
            Cue::BellStrike => "combat/sword_hit_1",
            Cue::Chat => "magic/conversion",
        }
    }
}

/// A silence between two parts of a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Pause {
    /// As a comma.
    Short,
    /// As the end of a sentence.
    Long,
}

phrases! {
    original {
        // In the order of the alphabet. The words are those of the original's
        // `res/ui/tts.txt`, and a test compares them.
        AcceptInvitationFrom = "accept the invitation from",
        Against = "against",
        AggressiveComputer = "aggressive computer",
        Alert = "alert",
        /// After the name of a player.
        Ally = "allied",
        And = "and",
        AreCompatible = "use a compatible version. Please select a server.",
        /// Before a place: "alert at a 2".
        At = "at",
        /// Before a distance: "at 12 meters".
        AtDistance = "at",
        AtTheCenter = "at the center",
        AwaitingYourOrders = "awaiting your orders",
        /// The last item of a menu.
        Back = "go back to the previous menu",
        BadLogin = "bad login",
        /// What a worker is busy with.
        Building = "building",
        /// The buildings of the score.
        Buildings = "building",
        /// A menu item.
        Cancel = "cancel",
        /// A menu item that goes back to the previous menu.
        CancelBack = "cancel",
        CancelBackExplanation = "go back to previous menu",
        CancelGame = "cancel game",
        CancelThisGame = "cancel this game",
        Changed = "has just changed.",
        CheatMode = "cheat mode",
        ChooseServerInList = "choose a server in a list",
        Cloaker = "cloaker",
        ConnectionInterrupted = "the connection was interrupted",
        ConnectionRejected = "error: connection rejected by the server. This is not a SoundRTS server",
        Consumed = "consumed",
        Contains = "contains",
        Continue = "continue",
        ContinueGame = "continue game",
        CurrentLoginKept = "the current login was kept",
        Defeat = "Defeat",
        Detector = "detector",
        DisplayOff = "the display is now off",
        DisplayOn = "the display is now on",
        Documentation = "documentation",
        Dot = "dot",
        Downloading = "Downloading...",
        East = "east",
        /// After the name of a player, and after what was sighted.
        Enemy = "enemy",
        EnterCommand = "Enter a command.",
        EnterMessage = "enter your message",
        EnterNewLogin = "enter your new login name",
        EnterServerIp = "enter the I P address of the server",
        ErrorDuringConnection = "error while connecting to the server",
        Exhausted = "exhausted",
        FirstPersonMode = "first person mode",
        Game = "game",
        Gathered = "gathered",
        HasBeenDefeated = "has been defeated",
        HasJustJoined = "has just joined",
        HasJustLoggedIn = "has just logged in",
        HasJustLoggedOut = "has just logged out",
        HasJustQuitGame = "has just quit the game",
        HistoryExplanation = "press F5 to listen again",
        HitPointsOn = "hit points on",
        InTheFog = "in the fog of war",
        Internet = "Internet",
        Invisible = "invisible",
        Invite = "invite",
        InvitesYou = "invites you",
        IsNowOff = "is now off",
        IsNowOn = "is now on",
        Join = "join",
        LeaveThisGame = "leave this game",
        LetterA = "a",
        LetterB = "b",
        LetterC = "c",
        LetterD = "d",
        LetterE = "e",
        LetterF = "f",
        LetterG = "g",
        LetterH = "h",
        LetterI = "i",
        LetterJ = "j",
        LetterK = "k",
        LetterL = "l",
        LetterM = "m",
        LetterN = "n",
        LetterO = "o",
        LetterP = "p",
        LetterQ = "q",
        LetterR = "r",
        LetterS = "s",
        LetterT = "t",
        LetterU = "u",
        LetterV = "v",
        LetterW = "w",
        LetterX = "x",
        LetterY = "y",
        LetterZ = "z",
        /// How well a computer plays, as in "quiet computer, level 3".
        Level = "level",
        Loading = "Loading",
        Local = "local",
        Lost = "lost",
        /// After the title of a menu.
        MakeASelection = "please make a selection",
        /// When Enter is pressed and no item is selected.
        MakeASelectionReminder = "please make a selection",
        ManaPointsOn = "mana points on",
        MapMode = "map mode",
        Menu = "Menu",
        MenuOf = "the menu of",
        Meters = "meters",
        Minutes = "minutes",
        MissionComplete = "mission complete",
        ModNotFound = "Couldn't find mod:",
        ModifyLogin = "Modify your login",
        Mods = "Mods",
        Move = "move",
        /// The title of the multiplayer menu.
        Multiplayer = "multiplayer",
        MultiplayerExplanation = "play on the Internet",
        /// The item of the main menu.
        MultiplayerMenuItem = "multiplayer,",
        Neutralized = "neutralized",
        NewLogin = "new login",
        NewObjective = "new objective",
        NoUnitControlled = "you are not controlling any unit",
        North = "north",
        Northeast = "northeast",
        Northwest = "northwest",
        NotEnoughPlayers = "Not enough players to start. The minimum is",
        Nothing = "nothing!",
        Objective = "objective:",
        ObjectiveComplete = "objective complete",
        ObserveAnotherPlayerExplanation = "Press Control Shift F4 to observe another player.",
        ObserveRecordedGame = "observe a recorded game",
        Ok = "ok!",
        /// As in "2 on gold".
        On = "on",
        OnTheServerOf = "on the server of",
        /// The count 1 before a feminine noun: "une ration".
        OneF = "1",
        OpenUserFolder = "open user folder",
        Options = "options,",
        OptionsExplanation = "modify your parameters",
        OptionsMenu = "options menu",
        OrLoginRejected = "or your login has been rejected",
        PercentVolume = "percent of the volume",
        Plateau = "plateau",
        PlayersOn = "players on",
        PressEnterToConfirm = "press enter to confirm",
        PrivateServer = "private server",
        PrivateServerExplanation = "The server won't be added to the list.",
        Processing = "processing",
        /// After a feminine noun: "units produced".
        ProducedF = "produced",
        /// After a masculine noun: "buildings produced".
        ProducedM = "produced",
        PublicServer = "public server",
        PublicServerExplanation = "a guest can create games",
        QuietComputer = "quiet computer",
        /// A menu item.
        Quit = "Quit",
        QuitExplanation = "quit this program",
        /// The item of the main menu.
        QuitMenuItem = "quit,",
        Restart = "restart",
        RestoreASavedGame = "restore a saved game",
        SaveTheGame = "save the game",
        Says = "says",
        Score = "score",
        Seconds = "seconds",
        SelectAndConfirmExplanation = "press Tab to select. press Enter to confirm.",
        SelectTargetAndConfirm = "press tab to select the target and then press enter to confirm",
        Server = "server,",
        ServerCouldNotStart = "error: the server couldn't start",
        ServerExplanation = "launch a server on your computer to host multiplayer games",
        ServerHostedBy = "server hosted by",
        ServerUnreachable = "error: the server is unreachable. The server is closed, or behind a firewall, or behind a router.",
        ServersOn = "servers on",
        SetSpeedToFast = "set the game speed to fast",
        SetSpeedToNormal = "set the game speed to normal",
        SetSpeedToSlow = "set the game speed to slow",
        SimpleServer = "simple server",
        SimpleServerExplanation = "only you can create games",
        SinglePlayer = "single player,",
        SinglePlayerExplanation = "play on your computer",
        Soundpacks = "Soundpacks",
        South = "south",
        Southeast = "southeast",
        Southwest = "southwest",
        Speed = "speed",
        Start = "start",
        StartAGameOn = "start a game on",
        StartAPublicGameOn = "Start a public game on",
        TheGameWillStartInFiveSeconds = "the game will start in 5 seconds",
        TheGameWillStartWhenOrganizerIsReady = "the game will start when the organizer is ready",
        TheListIsEmpty = "the list is empty",
        /// Between a key and what it does, "Tab to select", and between two amounts.
        To = "to",
        ToAlliance = "to alliance",
        ToThe = "to the",
        ToTheEast = "to the east",
        ToTheWest = "to the west",
        TooManyGames = "error : there are too many games",
        UnexpectedReply = "error: unexpected data was received from the server. This is not a SoundRTS server",
        Units = "units",
        Unknown = "unknown",
        UpdateAvailable = "a newer version of this program is available on the Internet",
        UseLettersAndNumbersOnly = "use letters and numbers only",
        VersionError = "version error",
        Victory = "Victory!",
        Welcome = "welcome",
        West = "west",
        WhatKindOfServer = "what kind of server?",
        YouAre = "you are",
        YouAreNowInObserverMode = "You are now in observer mode.",
        YouControl = "You control",
        YouHaveBeenDefeated = "You have been defeated.",
        Zoom = "zoom",
    }
    own {
        /// The help of the cheat console: each letter is a command.
        ConsoleHelp = "p: pause/unpause, s: set speed, r: get 1000 resources, t: get all techs, m: infinite mana, a: add units, v: instant victory",

        // A computer player, as players choose it and hear it.
        RandomComputer = "random computer",
        /// After the name of a computer player.
        WithUnlimitedResourcesAndNoRequirements = "with unlimited resources and no requirements",
        /// After the name of a computer player.
        WithUnlimitedResources = "with unlimited resources",
        /// After the name of a computer player.
        WithNoRequirements = "with no requirements",

        // A game room, the list of servers, and what a status key answers.
        IsReady = "is ready",
        IsNotReady = "is not ready",
        IAmReady = "I am ready",
        IAmNotReady = "I am not ready",
        Remove = "remove",
        /// As in "set quiet computer, level 3, to level 2".
        Set = "set",
        /// As in "set quiet computer, level 3, to level 2".
        SetTo = "to",
        /// As in "150 percent income": the share of what a seat's workers
        /// gather that it receives.
        Income = "income",
        /// Every computer of a campaign's missions, as in "set the computers,
        /// level 3, to level 2".
        TheComputers = "the computers",
        IsNowTheOrganizer = "is now the organizer",
        YouHaveBeenRemoved = "you have been removed from the game",
        ShareAddress = "the address to give to the other players is",
        AddressCopied = "it is copied to the clipboard",
        Recent = "recent",
        Favorite = "favorite",
        AddToFavorites = "add this server to your favorites",
        RemoveFromFavorites = "remove this server from your favorites",
        ToYourAllies = "to your allies",
        AnnounceArrivals = "say when players arrive on the server or leave it",
        TurnedOn = "on",
        TurnedOff = "off",
        ServerFull = "the server is full",
        NoServerFound = "no server found",
        /// After the server's name in the welcome: the connection is encrypted.
        Encrypted = "encrypted",
        // Why an encrypted connection was refused.
        CertificateForAnotherName = "not connected: the server's certificate is for another name",
        CertificateOutOfDate = "not connected: the server's certificate is out of date",
        CertificateNotTrusted = "not connected: the server's certificate is not trusted",
        MapRefused = "the map of this game is too large once unpacked",
        GameCannotStart = "this game cannot start here",
        NoPortForwarding = "the router did not open the port: only the players of your local network can join",
        Idle = "idle",
        BadlyHurt = "badly hurt",
        Stronger = "stronger",
        EvenlyMatched = "evenly matched",
        Weaker = "weaker",
        ObjectiveDone = "done",
        ObjectiveNotDone = "not done",
        /// As in "2 of 3".
        OutOf = "of",
        Damage = "damage",
        /// As in "damage 6 every 1.5 seconds".
        Every = "every",
        NoAttack = "no attack",
        Range = "range",
        Armor = "armor",
        HitPoints = "hit points",

        // Saved games.

        SaveGame = "save game",
        Saved = "saved",
        CannotSave = "cannot save",
        LoadAGame = "load a game",
        /// The game saved by itself when the player last quit.
        LastGame = "last game",
        Loaded = "loaded",
        CannotLoad = "cannot load",
        AnotherVersion = "made by another version of the game",
        FileDamaged = "the file is damaged",
        LoadingStopped = "loading stopped",
        SavedJustNow = "saved just now",
        /// As in "saved 3 hours ago".
        SavedAgo = "saved",
        MinutesAgo = "minutes ago",
        HoursAgo = "hours ago",
        DaysAgo = "days ago",
        DeleteThisSave = "delete this saved game?",
        Delete = "delete",
        Deleted = "deleted",

        // Listening to a recorded game: a save, or a record in the records folder.

        ListenToAGame = "listen to a game",
        /// The game kept by itself when the player's last game ended.
        LastFinishedGame = "last finished game",
        /// The record has no more turns.
        RecordingEnded = "end of the recording",
        /// A turn of the record cannot be replayed, or the file ends before its
        /// end: the listener stays where it stopped.
        RecordingDamaged = "the recording is damaged: it stops here",
        /// The result of a player still in the game when the recording ends.
        NotOver = "Not over",
        Paused = "paused",
        Slower = "slower",
        Faster = "faster",
        /// The fastest pace: the game is replayed silently, and only how far it
        /// has got is said.
        Fastest = "fastest, silent",
        JumpForward = "forward",
        JumpBack = "back",
        OneMinute = "1 minute",
        FiveMinutes = "5 minutes",
        KeepThisRecord = "keep this game's record",
        KeptInTheRecordsFolder = "kept in the records folder",
        CannotKeepIt = "cannot keep it",
        /// A key of the listener pressed in a game being played.
        OnlyWhileListening = "only while you listen to a recorded game",

        // The sound and speech settings of the options menu.

        SoundAndSpeech = "sound and speech",
        Volume = "volume",
        Percent = "percent",
        VolumeKeys = "Enter or Home raises it, End lowers it",
        NewsSpoken = "spoken",
        NewsSoundOnly = "its sound only",
        NewsLogOnly = "kept silently in the log",
        NewsDropped = "dropped",
        NvdaDirect = "NVDA tells the game when it has finished speaking",
        NvdaDirectExplanation = "this needs NVDA's current controller client and works from the next start: turn it off if messages stop, overlap, or wait too long",
        SpeechRate = "speech rate",
        CharactersASecond = "characters a second",
        MeasureIt = "press Enter to measure it",
        MeasureHow = "press a key to hear a sentence, then press a key the moment it ends. Escape cancels.",
        /// Said to measure the speech rate: no digits, which take longer.
        MeasureSentence = "This sentence measures how fast your screen reader speaks, so that each message of the game can follow the last one without a gap.",
        RateMeasured = "measured:",
        MeasureAgain = "to measure again, press Enter",
        TooEarly = "the key came too early to be the end of the sentence",
        TooLate = "the key came too late to be the end of the sentence",
        NothingChanged = "nothing changed",
        TryAgain = "to try again, press Enter",

        // Key help in a game.

        KeyHelpOn = "key help on: press a key to hear what it does, Escape to leave",
        KeyHelpOff = "key help off",
        NoCommand = "no command",

        // Copying a message of the log.

        Copied = "copied",

        // The categories of news, as the log's review keys name them. An alert is
        // the original's own word.

        ProductionNews = "production",
        EconomyNews = "economy",
        ScoutingNews = "scouting",
        ChatNews = "chat",
        SystemNews = "system",
        StoryNews = "story",

        // A game on a server: what happens to the players and the clock, and why
        // a game stops or does not start. The most important word comes first.

        LostTheConnection = "lost the connection",
        IsBack = "is back",
        DidNotComeBack = "did not come back, and is out of the game",
        LeftTheGame = "left the game",
        RemovedBy = "removed from the game by",
        PausedBy = "paused by",
        ResumedBy = "resumed by",
        GameSpeed = "speed",
        Slow = "slow",
        Normal = "normal",
        Fast = "fast",
        SetBy = "set by",
        GameStoppedByTheServer = "game stopped by the server:",
        ItFailed = "it failed",
        ATurnTookTooLong = "a turn took too long",
        GameNotStarted = "game not started:",
        ServerCannotBuildIt = "the server cannot build it:",
        TookTooLongToBuild = "it took too long to build",
        ServerBusy = "the server is busy, try again later",
        ServerSentWhatCannotBeRight = "game ended: the server sent what cannot be right",
        /// The game's rules are not the ones this client has, so its unit names
        /// would be wrong: the game is not played.
        RulesDiffer = "cannot play: your game data differs from this game's",
        OnlyTheOrganizerSetsTheSpeed = "only the organizer can change the speed",
        /// The game menu's item that stops the clock of a game on a server.
        Pause = "pause",
        /// The game menu's item that starts it again.
        Resume = "resume",

        // Returning to a game on a server after the connection is lost.

        ConnectionLost = "connection lost, trying to return",
        BackInTheGame = "back in the game",
        OrderNotTaken = "order not taken: not connected",
        /// A chat line or a flare given while the connection is lost.
        NotSentNotConnected = "not sent: not connected",
        CouldNotReturn = "could not return:",
        TheGameIsOver = "the game is over",
        TheSeatWasGivenUp = "your seat was given up",
        NoAnswerInTime = "the server could not be reached in time",

        // Watching a game: no units and no part in it, the side of one player at
        // a time.

        /// After someone's name: they joined the room or its game to watch.
        IsWatching = "is watching",
        StoppedWatching = "stopped watching",
        /// Before the names of those who watch.
        WatchingList = "watching:",
        /// Before the name of the player a watcher follows, and after a watcher's
        /// name in a chat line.
        Watching = "watching",
        /// The lobby's item that watches a game, before its host and map.
        Watch = "watch",
        WatchersWelcome = "watchers welcome",
        NoWatchers = "no watchers",
        /// The setting of a room being created: whether people may join it only
        /// to watch, and its two values.
        Watchers = "watchers",
        AreWelcome = "welcome",
        AreNotWelcome = "not welcome",
        GameTakesNoWatchers = "this game takes no watchers",
        WatchersFull = "no room for another watcher",
        RoomClosed = "the room closed: no player is left",
        WatchingNoOrders = "watching: you give no orders",

        // A flare that points the allies at a square.

        /// After an ally's name, before the square: "bob: flare at b 2".
        FlareAt = "flare at",
        /// To the player whose flare the server sent.
        FlareSent = "flare sent",
        NoAlly = "no ally in the game",
        /// Off a server: a flare has nobody to reach.
        NobodyToTell = "nobody to tell",
        /// The server refused a flare, a chat line or a command sent too soon
        /// after others.
        TooFast = "too fast: wait a moment",
        OnlyAWatcherFollows = "only a watcher follows another player",
    }
}
