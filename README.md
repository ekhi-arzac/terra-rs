# Native Terraria server

This Rust workspace implements the direct-IP join protocol of Terraria **1.4.5.8**. The listener negotiates a version and password, assigns a player slot and receives the character upload. A world request currently ends the session with a readable explanation. World entry and gameplay are under development.

```sh
cargo run -p terraria-server -- --config server.example.toml
```

Set the optional `world` path to load a release-326 `.wld` and apply its character difficulty checks. The world utility can inspect metadata and tiles or save a verified byte-identical copy. See the [world format and commands](docs/world.md).

The default listener is `127.0.0.1:7777` with eight slots. Set `bind` to the address and port clients should use. Press Ctrl-C to close the listener and its sessions. The executable runs independently of the supplied server distribution.

```sh
cargo test --workspace
cargo run -p terraria-testkit -- probe 127.0.0.1:7777 upload
```

The test suite includes a loopback TCP test. For release evidence and reproduction commands, see [compatibility](docs/compatibility.md). The [protocol](docs/protocol.md), [architecture](docs/architecture.md) and [progress log](docs/progress.md) describe the implemented boundary.
