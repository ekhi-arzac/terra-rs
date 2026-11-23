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

No matching unmodified graphical client was supplied or found in the configured Steam library. Milestone 2 requires that independent test. World entry, section streaming, in-world broadcasts, combat, progression and modified-world saves are pending. Server-side character mode, social transports and duplicate-name checks remain outside the implemented join behavior. The Rust server has been built and exercised on Linux x86-64. The workspace, including tests and benchmarks, passed `cargo check` for Windows x86-64 GNU and macOS ARM64. Linking and runtime tests on those operating systems are pending.

## World-file verification

The oracle-generated small classic world had seed 12345, dimensions 4200 × 1200, spawn (2095, 232), 176 chests and no signs. Rust walked all 5,040,000 tile positions via complete column runs and saved a 2,995,402-byte copy. Both files had SHA-256 `498d48e66d1908686b136c628a56b60c3902be44efe979445a97b41bcdca6aa7`. The official server loaded the Rust copy and accepted a headless handshake. Newly generated worlds can differ in identifiers and timestamps.

The [173-byte WorldData fixture](fixtures/world-info-326.bin) came from that reloaded world. Its frame starts with `ad 00 07`. The decoded prefix gives time 13500, width 4200, height 1200, spawn (2095, 232), world ID 1904902962 and name Oracle. Its SHA-256 is `a31031c608a27622e70d37d62952711aaeb6e9f4b00838900f9c8c8df64b4c71`. Reproduce a capture by sending a character upload and request 6 to the official server. The file is a test input for future packet-7 work.

```sh
cargo run -p terraria-testkit -- capture-world 127.0.0.1:18777 world-info.bin
```

## Initial benchmarks

```sh
cargo bench --bench packets
TERRARIA_BENCH_WORLD=/path/to/world.wld cargo bench --bench world
```

One release-profile run on the world above measured 72.9 ns per equipment encode/frame/decode, 12.8 ms to read and index the file, 1.15 µs per indexed tile lookup, and 1.75 ms for a verified copy under /tmp. These single-machine timings depend on filesystem, cache and hardware. Simulation benchmarks await an executable game tick.
