sound-rst

A Rust port of SoundRTS, the real-time strategy audio game. You hear the
map through speech and positional sound, and you play with the keyboard.

The game speaks through your screen reader, NVDA or JAWS. When no screen
reader runs, the system voice speaks.


Start the game

Start soundrts.exe. If you used the installer, the game is in the Start
menu as sound-rst.

The first time, the game needs the data of the original game: its rules,
maps and sounds. They are not part of this download. A message box asks
whether to download them, about 10 megabytes, from the original game's
repository on GitHub. Choose Yes. The game says how far the download is,
then opens the main menu. The next starts need no download.

To download the data without the question, run this in a terminal:

    soundrts.exe --fetch-data

Keep the DLL files next to soundrts.exe. The game reaches your screen
reader through them. Keep soundrts.pdb there too. It is not a program: it
lets the log say where the game was if it stops after an error.


Where your files are

%APPDATA%\SoundRTS holds your settings (SoundRTS.ini), the log
(tmp\client.log) and the data of the original game (the data folder).
Removing the game leaves this folder. Delete it yourself to remove
everything.

If the game stops after an error, the log says what the game was doing.
Send it with your report.


Run a server

soundrts-server.exe runs a server for multiplayer games on a computer that
stays on. For its options, run this in a terminal:

    soundrts-server.exe --help

You do not need it to play. The game can also host a server from its
Multiplayer menu.


More

The manual, the keys and the source code:
https://github.com/akj/sound-rst

LICENSE.txt is the license of the game. The licenses folder holds the
licenses of Tolk and of the NVDA controller client.
