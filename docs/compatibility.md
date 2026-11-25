# Compatibility evidence

## Tested on Linux x86-64

The supplied Terraria 1.4.5.8 Linux server generated a fresh small classic world. A Rust headless client sent the same direct-TCP scenarios to that server and to the Rust listener. The pairs were idle at the start of each scenario. Each comparison used decoded message order, slot, flag and localized kick reason, with no normalization.

| Server configuration | Scenario | Matching responses |
| --- | --- | --- |
| Open | Join | 3 (slot 0, false) |
| Classic world | Journey character | 3, 2 (key Net.PlayerIsCreativeAndWorldIsNotCreative) |
| Open | Wrong version | 2 (key LegacyMultiplayer.4) |
| Open | Character upload followed by empty name | 3, 2 (key Net.EmptyName) |
| Password | Wrong password | 37, 2 (key LegacyMultiplayer.1) |
| Password | Correct password | 37, 3 |
| Password | Character upload followed by empty name | 37, 3, 2 (key Net.EmptyName) |

The upload probe sends a named character, UUID, life, mana, buffs, loadout and an equipment slot before requesting a deterministic rejection with a second appearance. The observed response establishes progress through this sequence. It does not establish equality of hidden character state. Byte fixtures in the codec tests cover the wire layouts. The slot assignment, challenge and localization keys above also have live server evidence.

## Reproduce a comparison

Launch the supplied official server with a disposable world, direct IP bound to loopback and UPnP disabled. Its wrapper and native launcher need execute permission on Linux. A minimal official config for an isolated run follows. Give the world an explicit writable path.

```ini
world=/tmp/terraria-oracle/Oracle.wld
worldname=Oracle
autocreate=1
seed=12345
port=18777
maxplayers=8
upnp=0
```

```sh
mkdir -p /tmp/terraria-oracle
bash Linux/TerrariaServer -config /absolute/path/oracle.conf -ip 127.0.0.1 -noupnp -savedirectory /tmp/terraria-oracle
```

Start the Rust listener in a second terminal. Use idle servers so the first free player index is reproducible.

```sh
cargo run -p terraria-server -- --config server.example.toml
cargo run -p terraria-testkit -- compare 127.0.0.1:18777 127.0.0.1:7777 join
cargo run -p terraria-testkit -- compare 127.0.0.1:18777 127.0.0.1:7777 wrong-version
cargo run -p terraria-testkit -- compare 127.0.0.1:18777 127.0.0.1:7777 upload
```

Use `journey-on-classic` to exercise the 1.4.5.8 character/world mode rejection. For protected runs, set matching nonempty passwords on both servers and set `TERRARIA_PROBE_PASSWORD` in the probe environment. The `wrong-password` scenario sends an intentional mismatch. The tool prints received payloads and semantic responses, reports the first differing response index, and exits unsuccessfully on a mismatch, unexpected outcome or timeout.

## Current boundary

Verification uses headless TCP clients and the supplied 1.4.5.8 dedicated server. The user also confirmed that the installed stock 1.4.5.8 client stayed connected to tesst. After spawn the client sent creative powers module 6, a seven-byte biome report and an empty latency ping. NPC and event synchronization, full simulation and combat are pending. Server-side character mode, social transports and duplicate-name checks remain outside the implemented join behavior. The Rust server has been built and exercised on Linux x86-64. The workspace, including tests and benchmarks, passed `cargo check` for Windows x86-64 GNU and macOS ARM64. Linking and runtime tests on those operating systems are pending.

## World-file verification

The oracle-generated small classic world had seed 12345, dimensions 4200 × 1200, spawn (2095, 232), 176 chests and no signs. Rust walked all 5,040,000 tile positions via complete column runs and saved a 2,995,402-byte copy. Both files had SHA-256 `498d48e66d1908686b136c628a56b60c3902be44efe979445a97b41bcdca6aa7`. The official server loaded the Rust copy and accepted a headless handshake. Newly generated worlds can differ in identifiers and timestamps.

The [173-byte WorldData fixture](fixtures/world-info-326.bin) came from that reloaded world. Its frame starts with `ad 00 07`. The decoded prefix gives time 13500, width 4200, height 1200, spawn (2095, 232), world ID 1904902962 and name Oracle. Its SHA-256 is `a31031c608a27622e70d37d62952711aaeb6e9f4b00838900f9c8c8df64b4c71`. Reproduce a capture by sending a character upload and request 6 to the official server. The codec decodes and re-encodes this fixture byte for byte.

```sh
cargo run -p terraria-testkit -- capture-world 127.0.0.1:18777 world-info.bin
```

## Initial benchmarks

```sh
cargo bench --bench packets
TERRARIA_BENCH_WORLD=/path/to/world.wld cargo bench --bench world
```

One release-profile run on the world above measured 72.9 ns per equipment encode/frame/decode, 12.8 ms to read and index the file, 1.15 µs per indexed tile lookup, and 1.75 ms for a verified copy under /tmp. These single-machine timings depend on filesystem, cache and hardware. Simulation benchmarks await an executable game tick.

## Headless world entry and multiplayer

Start the official server on a local port, load an existing world and issue its console `save` while idle. Point the native listener at that same saved image. Use separate save directories when running multiple official instances.

```sh
cargo run -p terraria-testkit -- compare 127.0.0.1:18777 127.0.0.1:7777 enter
cargo run -p terraria-testkit -- compare 127.0.0.1:18777 127.0.0.1:7777 enter-personal
cargo run -p terraria-testkit -- peers 127.0.0.1:18777 127.0.0.1:7777
```

| World and request | Ordered responses compared | Terrain and inventory verified through spawn |
| --- | ---: | --- |
| Small classic, seed 12345, world spawn | 393 | 15 sections, 450,000 tiles, 9 chests, 360 slots |
| Small expert, seed 67890, world and personal spawn at (410, 465) | 2,344 | 39 sections, 1,170,000 tiles, 56 chests, 2,240 slots |

The comparison covers admission, both world-info packets, status, decoded tiles and section objects, chest sizes and contents, banner kills and claims, lunar shields, all six cavern monster types, spawn invitation and entry acknowledgement. The reference also sent NPC updates and buffs, game modules, event state, housing, quest and host information. The harness reports their IDs and counts. These messages remain outside the equality check.

Before the official server saved its settled terrain, the initial 15 sections differed at 241 tile positions in the classic world and 42 in the expert world. The native server serves saved liquid levels. The testkit compares them exactly. Taking the settled save gave an exact terrain match in the two runs above. The reference advances time while a player is active, so subsequent runs can also report a genuine difference in packet 7.

The two-player scenario ran on both implementations. Each observed slot activation, the same movement update, chat text with the sender’s assigned slot and RGB on both sockets, a follow-up tile section, departure and slot reuse. The reference additionally synchronizes NPCs, event modules and more player state during entry.

The offline regression world `fixtures/entry-326.wld` retains the classic world's real extended header and auxiliary sections, with a small constructed terrain and chest. The supplied server loaded it and emitted `fixtures/entry-world-data-326.bin`. The section fixture `fixtures/tile-section-326.bin` was captured separately from the classic world at tile (1600, 150). It includes chest 26.

On the current classic snapshot, release benchmarks measured 11.6 ms to load and index the world, 379 µs to read and compress a spawn section, and 1.91 ms to stage, verify and publish an exact copy.

## Resource transfer and gameplay saves

Run this mutating scenario on disposable copies of `docs/fixtures/entry-326.wld`. The two coordinates identify the fixture chest, with an empty test cell four tiles right and one up.

```sh
cargo run -p terraria-testkit -- resources 127.0.0.1:18777 2095 232
```

Two headless clients exercised the same sequence on the Rust server and the supplied 1.4.5.8 server. Both assigned a world slot, enforced the pickup reservation, replicated a reduced stack and removed it on pickup. The clients opened a chest in turn, observed a stack of 17 stone, placed and mined stone, and rejoined to check the tile, dropped resource and chest contents. Exact item trajectories, pickup timing and tile framing are outside this comparison. The official server loaded a Rust-edited fixture after orderly shutdown and admitted a headless player.

Resource support covers dirt, grass, stone, placed wood blocks, clay, mud, ice and both sets of early metal ores. The Rust server accepts client inventory updates. Multi-tile furniture, quick-stack to nearby chests, tool-power checks and server-side item physics remain open. Ground stacks are session state. The CLI saves supported block and chest changes on Ctrl-C or Unix SIGTERM and keeps the previous image in `.terra-rs.bak`.

## Terraria 1.4.6 PC direct-IP

The installed Steam 1.4.6 build uses `Terraria329`. Its dedicated server sends ServerInfo (163) on connection, and the client uploads PlayerPlatformInfo (164) after slot assignment. Headless tests reached spawn on both servers, checked rejection of the old greeting, and repeated the two-player resource scenario using copies of a world saved by 1.4.6. The Rust listener also loads existing release-326 worlds. The release-329 world header carries its no-spiders setting in bit 5 of the final WorldData progression byte. Platform identities and other crossplay transports remain outside this direct-IP update.

## Tree harvesting

Ordinary and biome trunks, palms, ash and vanity trees support cuts. The server removes the detached trunk, roots and branches, publishes the resulting tile state to the cutter and observing players subscribed to the affected sections, and reserves the resulting wood for the player who made the cut. The current yield is one wood per removed tile. It omits random bonus drops, acorns, shaking and gem-tree harvests. The loopback regression cuts a jungle tree above its stump, checks the tile removals and wood on both sockets, replays the cut, reconnects a player and reloads the saved file. A separate probe harvested the same naturally generated forest tree on disposable copies of tesst. Both servers sent wood to the cutter and the observer. In that probe, Rust generated one stack of eight wood and recorded eight removed tiles. The official server generated eight stacks with variable quantities. The regression also covers grass trimming, redundant hits and one-tile corrections.

## Ordered terrain and contested pickups

A loopback test loads a section and immediately changes it, checks that the section snapshot arrives before its update, and confirms that a second player receives the current foreground block when subscribing. Both players place and break the same test wall and see the same full tile, including the foreground block. Two concurrent break requests create one wood-wall stack. A non-owner's removal attempt restores the stack on that socket, and the owner can then pick it up once. The terrain is checked again from disk after shutdown.

Two headless clients also verified the wood-wall placement, break and single item drop on both the Rust listener and the installed official 1.4.6.1 dedicated server. Supported crafted walls include stone, wood, brick, glass and several biome woods. Naturally generated wall removal and inventory capacity checks are pending.

## Early crafting stations

The server supports placement and full-object removal for work benches, furnaces, and iron and lead anvils. It validates the floor and footprint on common terrain and platforms, replaces ordinary grass cover, and returns the authoritative footprint for a rejected placement. Breaking a supported floor block removes the station, and the station produces one item even when two clients break it together.

The same two-player placement and break probe passed on the Rust server and the installed official 1.4.6.1 dedicated server. It covered work benches, style 64 of the work-bench table, the furnace anchor, legacy lead-anvil placement, and a bench losing its floor. An automated Rust regression also covers an occupied footprint, a station that spans two sections, and reconnect and save/reload. Crafting recipes and character inventory still run in the stock client. The server synchronizes the station tiles and drops.

## Hinged doors

Two headless players on disposable worlds exercised packet 79 placement at each of the three origin rows, right and left swings, closure from the outer column, a blocked swing, breaking an open door and removing a ceiling support. The Rust and official 1.4.6.1 servers both returned wood and rich-mahogany door items. The Rust regression also checks a swing across a section boundary, a player standing in the doorway, a contested support break, late entry and saved frames. Supported styles are wood, ebonwood, rich mahogany, pearlwood and shadewood. Trapdoors, tall gates and locked doors remain open work. Rust checks the last reported player hitboxes before closing and uses a fixed closed-door frame variation.

## Online saves and administration

Two loopback tests place a tile, wait for a manual or timed save while the client stays connected, join a second player and confirm that an edit made after the save reaches the first player. They exit through the console command channel and reload both the world and its previous backup. A separate persistence test performs six consecutive publications and verifies the three previous revision numbers on disk. These checks cover the Rust operator interface and world files.

## Saved town residents

Start both servers on separate copies of the same saved town, then run the following comparison as the first join on each server. The compact entry fixture also works.

```sh
cargo run -p terraria-testkit -- compare 127.0.0.1:1242 127.0.0.1:1240 town
```

The installed 1.4.6.1 server and Rust matched all 116 observations from an official save containing 29 residents. Each slot matched on its initial NPC state, buffs, name and variation, and housing. The probe groups duplicate section and spawn announcements by slot. A loopback regression checks the saved Guide and Old Man on two successive joins. An additional fixture with an invalid house assigned in open terrain exposed a difference. The official server marked the occupant homeless while Rust reported the saved room assignment. Live room validation, relocation, town AI, spawning and combat remain pending.

The two-player resource probe also renames an open chest on close, observes the name from another socket, looks it up by position and checks the next join’s section metadata. The sequence passed on Rust and the official 1.4.6.1 server. The Rust loopback test reloads the name and stack from the saved world.
