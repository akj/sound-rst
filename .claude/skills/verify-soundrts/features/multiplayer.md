# Multiplayer

A player hosts a server from the game or joins one, meets other players in the lobby, opens or joins a game room, chats, and plays a game that the server runs. A player whose connection is lost returns to the same seat. A person can join a game only to watch it.

## Sub-features

- `multi-host-join` (harness, real server on 127.0.0.1): one player hosts from the game's menus and hears the address to give. A second player joins by that address, accepts an invitation, gets the map from the server, and both chat with F7.
- `multi-game` (harness, real server on 127.0.0.1): two players play a game on a server and leave it. One game played on the client and through a server is heard the same.
- `multi-discovery` (harness, 127.0.0.1): a server answers the search for servers on the local network.
- `multi-lobby-and-room` (harness, fake server): the lobby and the room say arrivals, departures, room changes, and chat. In a game they say the speed, the pause, the next organizer, flares, and removals.
- `multi-return` (harness, real server on 127.0.0.1): a player whose connection is cut hears "connection lost, trying to return", then "back in the game", and plays on. A player whose seat was given up hears why.
- `multi-watch` (harness, real server on 127.0.0.1): a watcher hears the side of the player it follows and changes nothing. A watcher too many, or an unwelcome one, hears why.
- `multi-network` (ears): two computers on a real network, a router that forwards the port, the public server list, and what a screen reader says of all the above.

## How to get to it (user POV)

- In the main menu, press Enter on `multiplayer`. If the login is still `player`, the game asks for a new login first.
- To host, choose the server item. The game says the address to give to the other players and copies it to the clipboard.
- To join, choose a server in a list, or enter the IP address of the server.
- On a server, choose `start a game on` or the public variant, then a map and a speed, to open a room. Accept an invitation, join an open game, or choose `watch` to enter another player's room.
- In a room or a game, F7 writes to the players. In a game, Control F7 sends a flare, and F10 opens the game menu.

## Driving it with verify.ps1

Preconditions:

- `verify.ps1 doctor` ends with `doctor: ready`.
- The machine lets a test listen on 127.0.0.1. The tests take free ports and ask no router for anything.

- **Host, join, chat, and play.** Scripted players go from the main menu to a server on 127.0.0.1, meet in a room, chat, and play. Run `verify.ps1 test multiplayer net_play`. Four tests pass in about 35 seconds. `a_player_hosts_another_joins_by_address_and_they_chat` checks that the host was told "the address to give to the other players is" and that the joiner heard "andrew says héllo bob".
- **Walk the lobby, the room, and the game on a server.** A scripted player meets a fake server that sends the messages of other players at set times. Run `verify.ps1 test multiplayer multiplayer`. Every test of `crates/client/tests/multiplayer.rs` passes.
- **Lose the connection and return.** The test pulls a player's connection out through a proxy on 127.0.0.1 and plugs it back in. Run `verify.ps1 test multiplayer returning`. Three tests pass.
- **Watch a game.** One person watches a game from its room, and another joins to watch while it runs. Run `verify.ps1 test multiplayer watching`. Three tests pass.

### By ear

For Andrew, with NVDA running. Steps 1 to 5 need one computer. Steps 6 to 9 need a second player on another computer of the same network.

1. In a terminal, run `cargo run -p soundrts -- --root vendor/soundrts-upstream`. Wait for the main menu.
2. Press Down twice, then Enter. You hear "multiplayer", then the multiplayer menu.
3. Choose the server item. You hear "the address to give to the other players is" and an address, then "welcome", your login, and "on the server of" your login.
4. Create a game on any map, invite a computer, and start. You hear the game start.
5. Press F10. You hear "Menu", and the sounds and the news of the game go on behind it. Press Escape. You hear "continue game".
6. Ask the second player to choose "enter the IP address of the server" and to type the address from step 3. You hear their login and "has just logged in".
7. Leave the game, create a new one, and invite the second player. When they accept, you hear their login and "has just joined".
8. Press F7, type a line, and press Enter. The second player hears a short sound, your login, "says", and the line.
9. Ask the second player to pull their network cable for 5 seconds during a game. They hear "connection lost, trying to return", then "back in the game".

## Gotchas

- Every harness proof here stays on 127.0.0.1. `docs/open-findings.md` states the limit: nobody has played over a real network.
- `multiplayer.rs` uses a fake server for the lobby and the room, so those tests prove what the client says and sends, not what the server does with it. The server's own rules have tests in `crates/server/tests`, which this map does not drive.
- The `returning` and `watching` tests put the players in a room through harness calls, not through the menus. The menu path to a room is proved by `net_play` only.
- The `net_play`, `returning`, and `watching` tests run on real time, so they take seconds and can fail on a machine under heavy load. Rerun once before you report a failure, and report both runs.
- No test asks a router to forward a port, and no test reads the public server list.
