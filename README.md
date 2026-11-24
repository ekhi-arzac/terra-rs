# Native Terraria server

This Rust workspace implements the direct-IP join protocol of Terraria **1.4.5.8**. Headless clients can join an existing release-326 world, receive terrain and chest contents, see other players move, exchange chat and reconnect. The server currently serves a saved world snapshot. NPC simulation, combat, tile edits and gameplay saves remain under development.

```sh
cargo run -p terraria-server -- --config server.example.toml
```

Set the optional `world` path to an existing release-326 `.wld` for world entry. Omitting it allows admission and character-upload probes. The world utility can inspect metadata and tiles or save a verified byte-identical copy. See the [world format and commands](docs/world.md).

The default listener is `127.0.0.1:7777` with eight slots. Set `bind` to the address and port clients should use. Press Ctrl-C to close the listener and its sessions. The executable runs independently of the supplied server distribution.

```sh
cargo test --workspace
cargo run -p terraria-testkit -- probe 127.0.0.1:7777 enter
cargo run -p terraria-testkit -- peers 127.0.0.1:7777
```

The test suite includes a loopback TCP test. For release evidence and reproduction commands, see [compatibility](docs/compatibility.md). The [protocol](docs/protocol.md), [architecture](docs/architecture.md) and [progress log](docs/progress.md) describe the implemented boundary.
