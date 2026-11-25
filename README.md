# Native Terraria server (WIP)

This Rust workspace implements the direct-IP join protocol of Terraria **1.4.6**. Headless clients can join an existing Terraria world, receive terrain and chest contents, see other players move, change teams and PvP settings, exchange public or party chat, and reconnect. Players can share dropped stacks, place, use, rename and recover empty wooden chests, and place and mine basic resource blocks, including dirt, stone, wood blocks and early ores. Players can also chop ordinary trees and palms for wood, place common walls, place and operate wooden doors, and use work benches, furnaces, and iron or lead anvils. Joining players also receive saved town residents, including the Guide, their names and housing. NPC simulation and combat remain under development.

```sh
cargo run -p terraria-server -- --config server.example.toml
```

Set the optional `world` path to an existing release 326–329 `.wld` for world entry. Omitting it allows admission and character-upload probes. The world utility can inspect metadata and tiles or save a verified byte-identical copy. See the [world format and commands](docs/world.md).

The default listener is `127.0.0.1:7777` with eight slots. Set `bind` to the address and port clients should use. The world clock advances while players are in the world. Enchanted sundials and moondials share their fast-forward state and cooldowns across players. The server saves the clock, tile and chest changes every 600 seconds. Set `autosave_seconds = 0` to disable the timer. Type `save` to write changes immediately, `playing` to list players, or `exit` to save and stop. Ctrl-C also saves and stops. The previous world is `.terra-rs.bak`, with up to two earlier revisions in `.bak.even` and `.bak.odd`. Ground items last for the running session. The executable runs independently of the supplied server distribution.

```sh
cargo test --workspace
cargo run -p terraria-testkit -- probe 127.0.0.1:7777 enter
cargo run -p terraria-testkit -- peers 127.0.0.1:7777
```

The test suite includes a loopback TCP test. For release evidence and reproduction commands, see [compatibility](docs/compatibility.md). The [protocol](docs/protocol.md), [architecture](docs/architecture.md) and [progress log](docs/progress.md) describe the implemented boundary.
