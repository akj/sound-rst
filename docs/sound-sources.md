# Sound sources: CC0 replacements for the original's sounds

The original game's sounds have an unclear licence and several authors (see `refactor-design.md`, section 13 and appendix D), so they are to be replaced before anything ships. This document records what the game plays, which CC0 recordings could stand in, how each fact was checked, and what is still missing. The recordings themselves stay out of the repository: `scripts/fetch-sound-candidates.ps1` rebuilds the set from its sources and checks every file's SHA-256, and `scripts/audition-sounds.ps1` plays one role's candidates in turn.

Researched on 14 November 2026.

## In short

- **Footsteps.** Good CC0 recordings exist for grass, leaves, mud, shallow water and snow, and they are far brighter than the game's own. Grass and leaves each have a set of five separate single steps, which is the shape the game uses. Gravel is weak. Wood is dark everywhere, though still brighter than the original.
- **Terrain ambience.** Every terrain has two or three CC0 beds, all 44.1 kHz or higher and long enough to loop. The town is the weak one: no CC0 recording found is both medieval-sounding and free of speech.
- **Brightness.** The game's default footstep has no energy above 4 kHz at all, and the cues headphones use to place a sound in front or behind lie mostly above 4 to 5 kHz. In the listening test, that footstep was placed wrongly 7 times in 8. The grass and leaves sets have 56 to 92 percent of their energy above 4 kHz.
- **Andrew rated every candidate by ear** on 14 November 2026, blind and in random order. See "What the listening found" below. There is a clear choice for every role except wood, forest and the town.

## How to listen

Run these from the repository root. Nothing plays until you run the audition command.

1. `pwsh scripts/fetch-sound-candidates.ps1 -FreesoundClientId <id>` downloads the set into `vendor/sound-candidates`, about 1.4 GB. Freesound gives its original files only to a signed-in user. The client id is listed with your API credentials at https://freesound.org/apiv2/apply. The script prints an address where you approve access, then asks for the code Freesound shows. Files already present with the right hash are kept, so a rerun downloads nothing.
2. `pwsh scripts/audition-sounds.ps1` lists the roles.
3. `pwsh scripts/audition-sounds.ps1 footsteps-grass` plays every grass candidate, best first. Before each it prints a line saying which candidate it is, who made it, how it was recorded and how bright it is.
4. `pwsh scripts/audition-sounds.ps1 footsteps-grass 2` plays only the second one again.
5. `pwsh scripts/rate-sounds.ps1` plays every role's candidates in random order without saying which is which, and asks you to rate each from 1 to 5. Type r to hear one again, or q to stop. At the end of each role it tells you which version was which candidate, then asks for a comment. To rate only some roles, name them: `pwsh scripts/rate-sounds.ps1 footsteps-*` rates only the footsteps. Your answers are saved after each one in `vendor/sound-candidates/ratings/`, in a new file for each session.

What you hear:

- Every candidate is brought to the same loudness first, so none wins by being louder.
- Footsteps are mixed to mono, as the game would play them. A set of single steps plays as a walk, one step every half second, twice through. A walking recording plays its first 20 seconds.
- Ambience plays its first 20 seconds in stereo. Add `-Seconds 60` to hear more.
- Ctrl+C stops at any point.

Each candidate's folder under `vendor/sound-candidates/<role>/` holds its files and a `source.txt` with the page, author, licence and the facts below.

## What the game plays

Found by reading `res/ui/style.txt`, the client code, and every sound file under `res`.

### How a sound is found

`style.txt` is a set of `def <name>` blocks, for unit types, buildings, terrains and events. A block inherits from another with `is_a`: a footman is a walking unit, which is a unit, which is a thing. Each key holds one or more numbers, and the game picks one at random. A number is the name of a file: `1053` is `res/ui/effects/1053.ogg`. When `res/ui/tts.txt` has text for a number, the text wins and is spoken through the screen reader (`crates/client/src/sound_cache.rs`, lines 111 and 174).

### Counts, and how speech was told apart

There are 652 `.ogg` files under `res`.

- **128 are sound effects**, all in `res/ui/effects`. No language folder has a copy of any of them. 110 are used: 106 by `style.txt`, three by the code (the minute bell and the two menu clicks), and one by a map title. 18 are never played.
- **524 are recorded speech**: 44 in `res/ui/actor` and 480 in the `actor` folders of 11 languages. They were told apart as speech because every one exists only in `actor` folders, each language has its own recording of the same number (no language's file matches the English one byte for byte), and the lengths differ by language: number 6005 lasts 1.13 seconds in English, 0.70 in French and 0.19 in Russian.

One correction to what was expected: these are not pieces of the original's spoken messages. Every message number in `tts.txt` is text, and none of them has a sound file. The recorded speech is the units' own voices: "yes, sir" when given an order, war cries, groans when wounded, the cry of a death, the call when a building is finished, and the progress and health readouts. The port plays these today. If they are dropped without a replacement, those cues go silent, so each needs either a new sound or a spoken message.

All the low sample rates are in the speech. 88 Russian and Belarusian voice files are at 22 kHz, and the Italian ones are at 24 kHz, 22 kHz or 11 kHz. Every sound effect is at 44.1 or 48 kHz. So the original footstep is dark because of how it was recorded, not because of its sample rate.

### By role

Positional means the sound is placed at the object on the map. A one-shot plays once per event.

- **Footsteps, 16 sounds.** Two per ground, played alternately as a walking unit moves: a positional one-shot per step. The grounds are grass (1053, 1054, also the default), leaves (1069, 1070), gravel (1344, 1345), wood (1067, 1350), mud (1087, 1349), water (1088, 1348) and snow (1346, 1347). A square's ground comes from its terrain, or else from the nearest object that has one:
  - meadows, paths and buildings give grass
  - woods give leaves
  - gold mines and building sites give gravel
  - bridges give wood
  - marsh gives mud
  - a ford gives water
  - a mountain pass gives snow

  Measured, the default grass steps and both wood steps have no energy above 4 kHz. The wood steps have 99 percent of their energy below 0.5 kHz. The others range from 5 to 42 percent.
- **Mounts and machines on the move, 7 sounds.** Hooves for knights, wheels for catapults, wings for dragons and flying machines, and a boat on water. Positional one-shots.
- **Terrain ambience, 7 loops and 9 repeating sounds.** The selected square plays its terrain's loop at its centre, positional, and the neighbouring squares can too. The loops are river, lake (also used for sea), ocean, mountain (also used for the mountain pass), ford, marsh, and a big bridge, which reuses the river. Meadows and forests have no loop of their own. Their sound comes from the meadow and wood objects on the square, which repeat short positional sounds every few seconds, with silence mixed in: 5 meadow sounds and 4 wood sounds.
- **Weapons and hits, 14 sounds.** A launch sound for bows, siege and magic, and a hit sound for swords, arrows, siege, magic and peasant tools, plus a miss. Positional one-shots on the target.
- **Unit noises, about 30 sounds.** Positional:
  - idle sounds that repeat: zombies, skeletons, knights, catapults, ships, dragons, corpses
  - peasants at work: mining, chopping, building
  - deaths
  - an order refused, blocked, healed, entering and leaving a building

  The wounded groans, war cries and acknowledgements are recorded speech.
- **Building noises, 15 sounds.** Each kind of building repeats its own sound every 6 or 15 seconds, positional: farm, lumber mill, town hall, keep, castle, blacksmith, stables (which reuse the knight's two sounds), workshop, dragon lair, mage tower and three towers. A damaged building plays a positional fire loop.
- **Production and completion, 4 effects plus speech.** A tick on each step of production, a sound for storing gold and one for storing wood, and the gold mine running out. The progress readouts and "building complete" are speech.
- **Alerts, 8 sounds.** A horn when a unit is attacked and a bell when a building is, at most every 10 seconds. These play panned by map position when the place is not on screen. Also a sound for each spell: teleport, holy vision, conversion, summoning, raising the dead, and meteors.
- **Interface, 8 sounds, not positional.** Two are shared with other roles.
  - a beep for errors
  - the minute bell, a quiet tick that also comes before positional speech
  - two menu clicks of 4 and 7 milliseconds
  - a chat sound
  - a prefix for custom maps
  - two sounds for moving along a path or across a bridge
- **Music:** none.

## Where the candidates came from

- **Freesound**, through its API only, because its terms forbid scraping the website. Search used a key; downloading original files used an OAuth2 sign-in on your account. Every Freesound candidate's licence field reads exactly `http://creativecommons.org/publicdomain/zero/1.0/`, the address of the CC0 1.0 Universal dedication. That was read from the API for every single file, not from the web pages, which were not opened.
- **Wikimedia Commons**, through its API and its file server. Only files whose page carries a CC0 or public-domain template.
- **Not used:**
  - OpenGameArt's `robots.txt` names Claude's crawlers with "Disallow: /", so nothing was fetched from it.
  - Kenney's `robots.txt` blocks Claude and disallows its asset and download paths for every robot. Kenney's packs are CC0 and you may download them by hand. Their footsteps measured dark (at most 4 percent above 4 kHz), so they would not have been chosen anyway.
  - BigSoundBank (La Sonothèque) is CC0, but its `robots.txt` disallows its audio folder for all robots.
- **Left out on licence:**
  - The radio aporee recordings on the Internet Archive carry the Public Domain Mark, which is meant for works already out of copyright, not a dedication by the author. For one of them the uploader is shown only as a cut-off email address, so who applied the mark is unknown. They include some of the brightest walks found: grass, leaves, gravel, mud and snow. If you decide the mark is good enough, the items are `aporee_60645_69691` (grass), `aporee_30121_34649` (leaves), `aporee_26730_30835` (gravel), `aporee_15132_17655` (mud) and `aporee_52977_60541` (snow), all at `https://archive.org/details/<item>`.
  - The USC sound-effects libraries on the Internet Archive: marked CC0 by whoever digitised them, but they are old Hollywood studio effects.
  - A SpliceSound upload of leaves, which measured very bright, because the uploader is named after a commercial sample company and the API cannot show whether they hold the rights.
  - Uploads that were plainly from Pixabay or SoundBible.
- **A judgement call, kept:** Nox_Sound links two of their uploads to paid packs of their own. A recordist releasing previews of their own work under CC0 is still a CC0 dedication by the rights holder, so their files are kept.

## Footsteps, by ground

Ranked by the share of energy above 4 kHz, the measure of how well headphone rendering can place a sound in front or behind. Where that share might come from hiss, wind or insects rather than the step, the note says so. Every file is at 44.1 kHz or higher. All candidates are Freesound CC0 unless stated.

### Grass

1. **Footstep_Grass_1 to 5**, five single steps, by GiocoSound. https://freesound.org/people/GiocoSound/sounds/421131/ and the rest of pack https://freesound.org/people/GiocoSound/packs/23825/. WAV, 48 kHz, mono, 0.5 seconds each. 56 to 63 percent above 4 kHz. Exactly the shape the game wants.
2. **Footsteps_Grass_1**, a walk, by SilentStrikeZ. https://freesound.org/people/SilentStrikeZ/sounds/389625/. WAV, 48 kHz, mono, 19.4 seconds. 26 percent. Would need cutting into steps.
3. **Footsteps_Mountain_Boots_Grass_Mono**, by Nox_Sound. https://freesound.org/people/Nox_Sound/sounds/556042/. WAV, 48 kHz, mono, 69.3 seconds: 19 walking steps in boots, then running and jumps. 15 percent.

### Leaves (forest floor)

1. **Sneaker Shoes Forest Footstep 1_1 to 1_5**, five single steps, by Joao_Janz. https://freesound.org/people/Joao_Janz/sounds/504242/ and pack https://freesound.org/people/Joao_Janz/packs/28240/, which has about 15 more. WAV, 48 kHz, stereo, to be mixed to mono, 0.6 to 0.9 seconds each. 67 to 92 percent. De-noised by the author, so the top end should be the steps themselves.
2. **footsteps - leaves 01**, a walk, by Anthousai. https://freesound.org/people/Anthousai/sounds/398776/. WAV, 96 kHz, stereo, to be mixed down, 17.8 seconds. 69 percent. Recorded with the microphone near the feet.
3. **Footsteps forest**, by Juno24. https://freesound.org/people/Juno24/sounds/474113/. WAV, 48 kHz, mono, 23.4 seconds. 25 percent. Dry pine needles rather than broad leaves.

### Gravel (stone, road)

1. **Footsteps Dirt Gravel**, a walk, by PotatokingXII. https://freesound.org/people/PotatokingXII/sounds/352870/. WAV, 48 kHz, mono, 35 seconds. 11 percent. The brightest real gravel found, and still only moderate.
2. **Left and Right Gravel Footstep**, six single steps, by Ali_6868. https://freesound.org/people/Ali_6868/sounds/384873/ and pack https://freesound.org/people/Ali_6868/packs/21608/. WAV, 44.1 kHz, stereo, to be mixed down, 0.5 to 0.7 seconds each. 4 to 16 percent. The right shape, but most steps are dark.

### Wood (bridge)

Every wood step found measured between 0 and 6 percent, so wood is dark by nature as well as in the original, whose wood steps measure 0 percent.

1. **walking_on_boardwalk**, by kbeezy88. https://freesound.org/people/kbeezy88/sounds/494972/. WAV, 44.1 kHz, stereo, to be mixed down, 76.8 seconds. Footsteps on an outdoor wooden boardwalk.
2. **Footsteps over Wooden Bridge**, by OBXJohn. https://freesound.org/people/OBXJohn/sounds/253090/. WAV, 44.1 kHz, stereo, 10.8 seconds. A real wooden bridge crossed in boots, recorded on a pocket voice recorder and converted to WAV, so probably compressed once on the way.

The audition prints each file's measured brightness.

### Mud (marsh)

1. **Footsteps Mud - Walk and run**, by Vrymaa. https://freesound.org/people/Vrymaa/sounds/770097/. WAV, 48 kHz, mono, 29.8 seconds, walking first. 55 percent. The author says it is close, dry and without background noise.
2. **Squelching Footsteps**, by Adielees9. https://freesound.org/people/Adielees9/sounds/369579/. WAV, 96 kHz, stereo, to be mixed down, 11.5 seconds. 52 percent.
3. **Footsteps_Mountain_Boots_Mud_Mono**, by Nox_Sound. https://freesound.org/people/Nox_Sound/sounds/548384/. WAV, 48 kHz, mono, 14.6 seconds: five walking steps, then running. 41 percent.

### Shallow water (ford)

1. **Slow Water Footsteps**, by Adielees9. https://freesound.org/people/Adielees9/sounds/369580/. WAV, 96 kHz, stereo, to be mixed down, 12.8 seconds. 58 percent. Slow steps through a small still stream, the nearest thing to a ford.
2. **Footsteps Water 01**, by aglinder. https://freesound.org/people/aglinder/sounds/265582/. WAV, 48 kHz, stereo, 52.8 seconds, walking and running. 53 percent, but the author mentions wind and a cicada, which may account for some of it.
3. **Splashing Footsteps Shallow Water**, by ChristopherJngs. https://freesound.org/people/ChristopherJngs/sounds/861369/. WAV, 44.1 kHz, mono, 8.9 seconds. 42 percent. A puddle rather than a ford.

### Snow (mountain pass)

1. **Footsteps on snow (clean)**, by xkeril. https://freesound.org/people/xkeril/sounds/611277/. WAV, 48 kHz, mono, 45.4 seconds. 32 percent. The only bright snow recording found.
2. **Footstep in the snow 01 to 04**, four single steps, by cabled_mess. https://freesound.org/people/cabled_mess/sounds/384423/ and pack https://freesound.org/people/cabled_mess/packs/21581/. WAV, 96 kHz, stereo, unprocessed, 1 to 1.8 seconds each. 4 to 17 percent.

## Terrain ambience, by terrain

These are beds, not placed precisely, so brightness matters less. All are stereo and 44.1 kHz or higher. "Rumble" means a lot of energy below 100 Hz, usually wind on the microphone, which a high-pass filter would remove. All are Freesound CC0 unless stated.

### Meadow

1. **Summer afternoon Meadow, birds and crickets**, by DanGasior. https://freesound.org/people/DanGasior/sounds/414907/. WAV, 48 kHz, 121 seconds. 46 percent above 4 kHz. Clean, and plainly a meadow.
2. **Summer meadow - Houghton Lodge**, by richwise. https://freesound.org/people/richwise/sounds/815552/. WAV, 48 kHz, 114 seconds. Insects in long grass. Two pigeons fly over, and after a minute a breeze touches the microphone.
3. **Hungary Meadow Morning Birds**, by Alcappuccino. https://freesound.org/people/Alcappuccino/sounds/416205/. WAV, 48 kHz, 211 seconds. Darker, and recorded with a hand-held recorder.

### Forest

1. **Forest Ambience 1**, by Fester993. https://freesound.org/people/Fester993/sounds/564436/. WAV, 48 kHz, 181 seconds. A plain summer forest in Italy, the best-rated forest bed on Freesound.
2. **Quiet forest, birds, insects, leaf rustle**, by nfsgit. https://freesound.org/people/nfsgit/sounds/121965/. WAV, 48 kHz, 292 seconds. Deep in a forest north of Oslo. Has rumble.
3. **Birds In Forest (Scotland)**, by BurghRecords. https://freesound.org/people/BurghRecords/sounds/490846/. WAV, 44.1 kHz, 113 seconds. Full of birds, so it may sound too much like the meadow.

### Dense forest

1. **Clark's Tower Ambience 4**, by greysound. https://freesound.org/people/greysound/sounds/547922/. FLAC, 48 kHz, 460 seconds. Strong wind through the canopy, creaking trunks, distant birds. Cleaned by the author.
2. **Silent windy pine forest**, by bruno.auzet. https://freesound.org/people/bruno.auzet/sounds/670307/. WAV, 48 kHz, 334 seconds. Wind in tall pines, a trunk grinding now and then. Very dark; it may sound like the mountain.
3. **Forest at night**, by Dokuta_Gerovv. https://freesound.org/people/Dokuta_Gerovv/sounds/662095/. WAV, 48 kHz, 78 seconds. A summer night near a Greek village. Its insects may sound like the marsh, and dogs are possible.

### Mountain and mountain pass

1. **Wind singing in the mountain**, by felix.blume. https://freesound.org/people/felix.blume/sounds/165526/. WAV, 96 kHz, 230 seconds, 132 MB. Wind howling on a mountaintop in Mexico, crickets behind. The howl gives it an identity that wind in trees does not have.
2. **Mountain wind, heavy strong gusts, Swiss Alps**, by kyles. https://freesound.org/people/kyles/sounds/454092/. FLAC, 48 kHz, 171 seconds. The gusts may make a loop's seam audible.
3. **Larrun Mountains, medium wind**, by nicotep. https://freesound.org/people/nicotep/sounds/725630/. AIFF, 48 kHz, 151 seconds. The steadiest, and the least distinctive.

### Marsh

1. **Lily pads and frogs at night**, by Danjocross. https://freesound.org/people/Danjocross/sounds/503206/. WAV, 48 kHz, 112 seconds. Loud frogs and katydids at a lake at night. The author says it is unedited, with no car or plane noise.
2. **Marsh**, by MarleneAyni. https://freesound.org/people/MarleneAyni/sounds/580325/. WAV, 44.1 kHz, 154 seconds. Frogs and many birds by day, in Quebec. Has rumble.
3. **Swamp, frogs, birds, forest**, by jacekksiazek. https://freesound.org/people/jacekksiazek/sounds/393644/. WAV, 96 kHz, 216 seconds. A Polish swamp reserve. Dark.

### River

1. **Small river stream**, by klau78. https://freesound.org/people/klau78/sounds/546804/. WAV, 48 kHz, 341 seconds. Continuous flow, nothing else mentioned.
2. **Snowmelt flowing into a lake at Okanagan Mountain Provincial Park**, by Extemporalist, on Wikimedia Commons. https://commons.wikimedia.org/wiki/File:Snowmelt_flowing_into_lake_at_Okanagan_Mountain_Provincial_Park.flac. The page states: "This file is made available under the Creative Commons CC0 1.0 Universal Public Domain Dedication." That comes from the template `self|cc-zero`, read through the Commons API. FLAC, 48 kHz, 33 minutes.
3. **Bachlauf plätschert, Molln**, by DrTrumpet, on Wikimedia Commons. https://commons.wikimedia.org/wiki/File:2024-07-26_Molln_(Oberösterreich)_Bachlauf_plätschert_(Krumme_Steyerling_bei_Piesslingerstraße).wav. The page states the same CC0 dedication, from the same template. WAV, 48 kHz, mono, 27 seconds, recorded close on a studio microphone. Short for a loop.

### Lake

1. **Water waves lapping, lake, gentle, close**, by TRP. https://freesound.org/people/TRP/sounds/568819/. FLAC, 48 kHz, 133 seconds. Small waves on a rocky shore, recorded close. Clearly not the sea.
2. **Gentle waves on a lake**, by TheFlyFishingFilmmaker. https://freesound.org/people/TheFlyFishingFilmmaker/sounds/614299/. WAV, 44.1 kHz, 175 seconds. Has rumble.

### Sea and ocean

The game has both, sea and ocean, so two of these can be told apart by size of wave.

1. **Ocean coast, Outer Banks**, by YevgVerh. https://freesound.org/people/YevgVerh/sounds/827530/. WAV, 48 kHz, 181 seconds. Light ocean waves on a sand beach.
2. **Sea waves**, by molkko. https://freesound.org/people/molkko/sounds/425632/. WAV, 44.1 kHz, 123 seconds. Mid-sized Mediterranean waves, the brightest sea. Has a small DC offset to remove.
3. **Sea waves, Playas del Viento**, by huhuertas. https://freesound.org/people/huhuertas/sounds/786037/. WAV, 48 kHz, 208 seconds.

### Town

1. **Market**, by ninebilly. https://freesound.org/people/ninebilly/sounds/173013/. WAV, 48 kHz, 178 seconds. A market crowd; the place and language are not stated, so speech may be clear.
2. **La fontaine de la place Saint-Etienne**, by aldor, on Wikimedia Commons. https://commons.wikimedia.org/wiki/File:La_fontaine_de_la_place.ogg. The page states: "This work has been released into the public domain by its author, aldor." That comes from the templates `PD-pdsounds.org` and `PD-author`, read through the Commons API; the Commons licence field says "Public domain". Ogg, 44.1 kHz, 85 seconds. A town fountain with cathedral bells in Toulouse. It came to Commons from pdsounds.org as a 128 kbps MP3, so it has been compressed twice.
3. **Crowd**, by SamuelGremaud. https://freesound.org/people/SamuelGremaud/sounds/545756/. WAV, 48 kHz, 167 seconds. A market crowd in the south of France; expect French speech.
4. **Traditional blacksmith**, by bruno.auzet. https://freesound.org/people/bruno.auzet/sounds/640197/. WAV, 48 kHz, 312 seconds. A forge with fire and gentle hammering. Not a bed on its own, but the most medieval sound found, and it could sit under a crowd.

## The other roles

Shortlisted from the API's descriptions and facts, not downloaded into the audition set. Freesound items are CC0 by the same licence field. Ids are Freesound sound ids; the page is `https://freesound.org/s/<id>/`. Where a share above 4 kHz is given, the file was downloaded and measured.

- **Weapons and hits.**
  - Sword clash: 326803 by JohnBuhr, mono, 89 percent; the series has about 60 takes.
  - Arrow hit: 384914 by Ali_6868, 36 percent.
  - Siege impact: 569502 by Sheyvan, mono, 24 percent, with 14 variants.
  - Catapult launch: 479922 by craigsmith, 18 percent, needs trimming.
  - Bow release: 534940 by JoeDinesSound, not measured. The other bow found, 384915, is dark.
  - Magic cast and magic hit: 442774 by qubodup and 500911 by Bertsz both measured dull, at 5 percent.
- **Mounts and machines.**
  - Hooves: 175356 by Max_Headroom has strong low rumble and needs a high-pass. 564626 by D4XX is the alternative.
  - Wings: 389634 by _stubb, and 670509 by 2create for the dragon.
  - Boat: 588307 by Fenodyrie, a rowing boat with creaking rowlocks.
- **Unit noises.**
  - Zombie: 196720.
  - Skeleton rattle: 202102, 10 percent.
  - Ship creak: 397620.
  - Dragon growl: 146558. 315766 was made for a stage production, so check it before use.
  - Human death: 131710.
  - Building collapse: 675900.
- **Peasant work.**
  - Pickaxe: 233629 by abstraktgeneriert, 41 percent.
  - Wood chop: 537434 by wavecal22, 26 percent.
  - Hammering wood: 204946 by CapsLok.
- **Buildings.**
  - Anvil: 521972 by Kastenfrosch, 8 percent, and 386099 at 96 kHz.
  - Saw: 566037.
  - Farm animals: 401636 and 383144.
  - Stables: 675423 and 531429.
  - Workshop: 245462.
  - Dragon lair: 460459.
  - Mage tower hum: 861430 by qubodup.
  - Fire loop: the one measured, 558967, is dull; 729396 by HECKFRICKER is untested.
- **Production and completion.**
  - Completion chime: 541985 by Rob_Marion.
  - Wood stacked: 577455 and 478238.
  - Coins and ticks: Kenney's RPG Audio coins measured 94 percent and Kenney's interface tick 57 percent. Kenney is CC0, but must be downloaded by hand (see above).
- **Alerts.**
  - Alarm bell: 200318 by WebbFilmsUK.
  - Teleport: 220162 by GameAudio.
  - Holy chime: 351408 by newagesoup.
  - Summoning: 740776 by qubodup.
  - Meteor: 252092 layered with 522099.
  - War horn: no clean, dry horn was found. The best, 69206 by Countrychap999, was recorded in the field and is an MP3.
- **Interface.** Kenney's Interface Sounds pack has clicks, ticks, bongs and error sounds, all CC0 and all needing a download by hand. On Freesound: an error buzzer, 700641; a chat notification, 740420; a clock tick, 412751.

## What could not be found

- **Footsteps on wood that are bright.** None exist among the CC0 recordings searched, and the original's are darker still. If front and behind matter on bridges, wood may need a little added brightness, such as a short click of the board's surface layered on.
- **Real gravel that is bright.** The best is 11 percent.
- **A ford.** Only a still stream and puddles; no wading through moving water.
- **Single-step sets** for gravel that are bright, and for wood, mud, water and snow at all. The walks would need cutting into steps.
- **A medieval town square without speech.** Every real market is modern and full of talk; every "medieval" result was music or sound design.
- **A war horn, a bright fire loop, bright magic sounds, and a dragon** that is not a voice actor.

## What was verified, and how

- **Licences.** Every Freesound licence was read from the API for each file. The Commons licences were read from each page's templates and licence field through the Commons API. The rejected sources were checked on their own `robots.txt` and terms. The Freesound web pages themselves were not opened, because of Freesound's terms; you can open any of the links above to see the licence line.
- **Format, sample rate, channels and length.** For Freesound, these come from the API. For every downloaded file, they also come from `ffprobe` on the file itself.
- **Brightness.** Measured on each downloaded file with `scripts/measure-brightness.py`, which mixes to mono and counts the share of energy above 4 kHz, ignoring anything below 20 Hz.
- **Hashes.** Every audition file's SHA-256 is in `scripts/sound-candidates.psd1`. The fetch script downloaded the whole set again from those URLs and matched every hash.
- **Content, not verified.** What each recording contains comes from its author's description; nothing was played. Hiss, wind or insects can raise the brightness figure, and a description can leave out a passing car or a voice. Whether each bed loops cleanly is also unknown.
- **The other roles** come from a search pass whose facts are the API's; only the files with a percentage were downloaded and measured.

## What the listening found

Andrew rated all 45 candidates with `scripts/rate-sounds.ps1` on 14 November 2026: one pass, with headphones, each role's candidates in random order and unnamed until the role was done. Each rating is from 1, unusable, to 5, just right. Footsteps were played in mono and centred, so these ratings judge how each ground sounds, not how well a sound can be placed in front or behind.

Chosen, by role:

- **Grass:** Nox_Sound's boots on grass, 5. GiocoSound's single steps 3, SilentStrikeZ's walk 2 ("could be okay if chopped up").
- **Leaves:** Anthousai's walk, 5. Joao_Janz's single steps 4. Juno24's 2, ruled out for background noise.
- **Gravel:** Ali_6868's six single steps, 5. PotatokingXII's walk 4.
- **Wood:** OBXJohn's bridge 3, kbeezy88's boardwalk 2. "Lots of background noise in both." Still open.
- **Mud:** Nox_Sound's boots in mud, 5. Adielees9 and Vrymaa 3 each: "more realistic, but the sounds are too far apart. I want short sounds that make it quick to navigate with arrows."
- **Shallow water:** aglinder's walk, 5. ChristopherJngs 4, Adielees9 3.
- **Snow:** cabled_mess's four single steps, 5. xkeril's walk 4.
- **Meadow:** richwise and Alcappuccino, 4 each, to use both. DanGasior 3.
- **Forest:** all three 2, because they "all sound very similar to meadow". Fester993's was the best of them. Still open.
- **Dense forest:** greysound's Clark's Tower, 4. Dokuta_Gerovv's forest at night 3, "could be an okay forest perhaps". bruno.auzet's pine forest 2.
- **Mountain:** kyles's heavy gusts in the Swiss Alps, 5. nicotep 4, felix.blume 3.
- **Marsh:** Danjocross's frogs at night, 5. jacekksiazek 4, MarleneAyni 3.
- **River:** klau78's small river, 5, "the best river". Extemporalist's snowmelt, 5, heard as a stream. DrTrumpet's Molln stream, 4, heard as being on the river, as in a boat.
- **Lake:** TheFlyFishingFilmmaker's gentle waves, 5. TRP's 4.
- **Sea:** molkko's waves, 4. huhuertas and YevgVerh 3 each, "all great, but might be a bit loud". All were played at the same measured loudness, so these two sound bigger, not louder: they may suit the ocean, played quieter.
- **Town:** none. The market and the blacksmith 2, the crowd and the fountain with bells 1. Still open.

What it shows:

- **Brightness did not decide the preference.** In the four footstep roles where brightness clearly separated the candidates (grass, mud, water, snow), the brightest never won, and in three the darkest did. In the other three roles the candidates' brightness overlapped or differed by under 1 percent. The brightness measure stands for being placed in front or behind, which this test did not try. Before a footstep is final, the chosen ones should go through the listening room's front-or-behind step.
- **Footsteps must be short.** A step is heard on each move with the arrow keys, so each step is cut short, and a walk's own pace does not carry over.
- **One author for several grounds.** Nox_Sound's boots won on both grass and mud. The same series may cover the grounds still weak, with the same boots and microphone.

## Recommendations

1. **Test the chosen footsteps in front and behind** in the listening room, which today plays only the game's own footstep.
2. **Cut short single steps** from the chosen walks (Nox_Sound's grass and mud, Anthousai's leaves, aglinder's water) with `ffmpeg`. Then listen again, since a step on its own can sound different from a walk.
3. **Search again for wood, forest and the town.** Look first in Nox_Sound's boots series for wood. A forest must not sound like the meadow: fewer open-field birds and crickets, more leaves and trees. The town could be a quiet crowd bed with the forge and bells laid on top, rather than one recording.
4. **Decide on the Public Domain Mark.** If you accept it, the radio aporee walks are worth adding for grass, leaves, gravel and mud.
5. **Keep the licence record with the files.** When sounds are chosen, the facts here and the hashes in `scripts/sound-candidates.psd1` are the record to ship with them.
