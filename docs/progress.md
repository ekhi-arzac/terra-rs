# Progress

| Milestone | Status | Evidence |
| --- | --- | --- |
| 0 — Reference analysis | Initial scope complete | Identified the supplied 1.4.5.8 assembly, release 326 and relevant loaders and dispatch methods. |
| 1 — Protocol foundation | Implemented for the supported packet set | Bounded framing, directional module decoding, typed layouts, malformed-input tests and recorded fixtures. |
| 2 — Connectivity | Headless admission through world entry verified | Seven admission scenarios against the official server, plus world entry and two concurrent named players. Independent stock-client verification is deferred. |
| 3 — World loading and synchronization | Headless entry from existing worlds works | World header, compressed sections, chests and all slots, signs, tile entity records, banner state, lunar tower shields, cavern variants and follow-up section requests. Two live world-entry comparisons reached packet 129. NPC, quest, biome and other module state remain incomplete. |
| 4 — Basic multiplayer | Movement, chat and membership verified | One roster owner, per-socket identity, player snapshots, visible inventory and vitals. Two headless clients observed movement, chat, departure and reconnection on each server. Chest interaction and area-of-interest updates during movement remain open. |
| 5 — Gameplay | Early world state only | Cavern monster variants reproduce the seeded reference selection. Active NPCs, combat, liquids, item pickup, wiring and world updates still need simulation. |
| 6 — Persistence and administration | Read-only world snapshots, verified copies and configuration | The server accepts TOML configuration, passwords, slot limits and graceful shutdown. Modified-world saves, rolling backups and administration commands remain open. |
| 7 — Compatibility and performance | Ongoing | Rust headless differential scenarios and packet/world/section benchmarks. |

## Verification

Linux x86-64 passed all 25 workspace tests, formatting, Clippy with warnings denied and the optimized build. Windows x86-64 GNU and macOS ARM64 passed all-target type checks. The compact fixture world also loaded in the supplied server. The classic and expert entry runs compared 54 sections, 1,620,000 tiles and 2,600 chest slots in total. Additional reference-only packets are listed by the harness. Reproduction commands and the comparison scope are in the [compatibility record](compatibility.md).

## Next engineering tasks

1. Load persisted NPC records and generate their join snapshots, buffs and housing from release-326 NPC defaults. Add a comparison using a populated town.
2. Implement remaining entry modules and requests driven by movement. Keep the reported reference-only packet counts visible as coverage expands.
3. Extend the roster owner with the fixed-step world update. Begin mutable interactions with chest ownership, authoritative inventory changes and save/reload tests.
