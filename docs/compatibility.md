# Compatibility evidence

## Tested on Linux x86-64

The supplied Terraria 1.4.5.8 Linux server generated a fresh small classic world. A Rust headless client sent the same direct-TCP scenarios to that server and to the Rust listener. The pairs were idle at the start of each scenario. Each comparison used decoded message order, slot, flag and localized kick reason, with no normalization.

| Server configuration | Scenario | Matching responses |
| --- | --- | --- |
| Open | Join | 3 (slot 0, false) |
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

For protected runs, set matching nonempty passwords on both servers and set `TERRARIA_PROBE_PASSWORD` in the probe environment. The `wrong-password` scenario sends an intentional mismatch. The tool prints received payloads and semantic responses, reports the first differing response index, and exits unsuccessfully on a mismatch, unexpected outcome or timeout.

## Current boundary

No matching unmodified graphical client was supplied or found in the configured Steam library. Milestone 2 requires that independent test. World entry, section streaming, in-world broadcasts, combat, progression and modified-world saves are pending. Server-side character mode, social transports, duplicate-name checks and journey-mode admission remain outside the implemented join behavior. The Rust server has been built and exercised on Linux x86-64. Windows and macOS builds and runtime behavior remain unverified.
