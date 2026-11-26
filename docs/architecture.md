# Architecture and reference analysis

## Reference identity

The supplied `Linux/TerrariaServer.exe` is a managed PE assembly named `TerrariaServer`, version **1.4.5.8**. Its SHA-256 is `4b87890ac53d40f61db5f928693a379acf4ccbd8ed3b47eb32fb096f145df034`. The PE version strings, server banner and first entry in `Linux/changelog.txt` agree. `MessageBuffer.GetData` and `NetMessage.SendData` both use release number **326** for the greeting. Independently, `WorldFile.SaveFileFormatHeader` writes **326** as the world version.

The distribution includes a Bash wrapper, an x86-64 MonoKickstart launcher, managed framework and FNA libraries, native SDL3, FNA3D and FAudio libraries, runtime configuration, an icon and a changelog. The original checkout contained a Cargo starter and the project brief. It supplied no decompiled C#, gameplay client, world, packet capture or dedicated server configuration.

Read the IL directly using Python analysis dependencies in a disposable virtual environment. These commands show the method bodies, field references, literal values, overload signatures and switch targets from this exact assembly. Redirect output locally when inspecting the large dispatch methods.

```sh
python3 -m venv /tmp/terraria-il
/tmp/terraria-il/bin/pip install dnfile==0.18.0 dncil==1.0.2
/tmp/terraria-il/bin/python tools/inspect_il.py Linux/TerrariaServer.exe \
    Terraria.MessageBuffer::GetData Terraria.NetMessage::SendData > /tmp/dispatch.il
/tmp/terraria-il/bin/python tools/inspect_il.py Linux/TerrariaServer.exe \
    Terraria.IO.WorldFile::SaveWorld_Version2 Terraria.IO.WorldFile::LoadFileFormatHeader
```

A type name alone lists its fields and methods. In the dispatch methods the first switch indexes `message ID - 1`. Tool offsets are decimal positions in the method body, including the method header. RVAs identify the body within this assembly. Keep binary distributions and IL listings outside commits.

## Rust ownership

| Component | Responsibility |
| --- | --- |
| terraria-binary | Bounds-checked .NET primitives shared by file and wire readers. |
| terraria-world | Validated immutable world image, column index, world-data projection, row-major tile sections, container and tile entity records, and lossless save. |
| terraria-protocol | Version-specific typed packets, fallible field codecs and immutable Bytes frames. It has no sockets or global game state. |
| terraria-network | Tokio listener, bounded admission, socket I/O, connection phases, terrain streaming and the active player roster. |
| terraria-server | TOML configuration, structured logging, startup and shutdown. |
| terraria-testkit | Headless TCP client and semantic comparison of two live servers. |

The listener alone owns the free slots. Each admitted socket gets one task and one staging character. A JoinSet returns slots to the listener on completion. A full listener sends the vanilla full-server kick with a bounded write deadline. Reads and writes run in order with TCP backpressure. No unbounded application queue is needed during the handshake. A 90-second absolute join deadline also covers partial reads and blocked writes. Shutdown stops admission, closes sockets and drains the roster owner. The immutable world is shared by all peers. At most four blocking workers build and compress sections. Each transfer writes one section followed by its batched chest contents. The active roster owns the set of sections scheduled for each connection.

The active roster has one owner and a bounded 256-event input channel. Its per-peer output queues have 64 entries. A slow consumer is disconnected. Each active connection reads frames and drains its outgoing queue independently, retaining partial TCP reads when a peer broadcasts. The leave event is queued before the listener releases the slot.

## Reference entry points

| System | Entry points and observed ownership |
| --- | --- |
| Connections | `Netplay.OnConnectionAccepted` and `FindNextOpenClientSlot` own slot allocation. `RemoteClient` owns a socket, state, termination flags, timeout and sent-section tracking. `NetMessage.CheckBytes` splits the receive buffer and dispatches complete frames. |
| Messages | `ID.MessageID` names the IDs. `MessageBuffer.GetData` (RVA 0x14d2e4) handles mode and phase checks, field decoding and effects. `NetMessage.SendData` (0x167c7c) writes the matching fields and dispatches to the intended recipients. |
| Player and inventory | Clients upload join-time appearance, vitals, loadout, buffs and equipment. The receiving server selects the player by connection. `PlayerItemSlotID..cctor` allocates the 990 serialized slot indices. `Item.SetDefaults` and the slot reference type link inventory entries to game definitions. A successful spawn transfers the staged character into the roster. Active changes use the socket-assigned slot. |
| World files and generation | `IO.WorldFile.LoadWorld` selects a versioned reader. `LoadWorld_Version2` uses a directory, frame-importance table and section loaders. `SaveWorld_Version2` writes the sections and backpatches their end offsets. `WorldGen` owns generation and terrain updates. `Tile` holds active type, wall, liquid, frames and packed flags. |
| World transmission | Request 6 transitions vanilla state 1 to 2 and triggers WorldData (7). `NetMessage.SendSection` and `RemoteClient.TileSections` track delivery. `Netplay.GetSectionX/Y` divides tile coordinates by 200 and 150 respectively. `CompressTileBlock` writes x/y as i32 and width/height as i16 inside a DeflateStream, then delegates tile and entity encoding to `CompressTileBlock_Inner`. |
| Tick and entities | `Main.DedServ` drives a 1000/60 ms accumulator. With no fully joined player it services networking. With joined players it calls the game update. In `DoUpdateInWorld_Inner` the observed relative order is players, NPC spawning, pressure plates, NPCs, projectiles, items, time, world generation/invasion and server/client updates. Other passes occur between these calls. `Player`, `NPC`, `Projectile` and `Item` state is reached through the world update routines. |
| Shutdown | `Main.DedServ` handles console input and server-loop termination. `WorldFile.InternalSaveWorld` and `DoRollingBackups` are the reference save/backup entry points. |

The roster owns the 60 Hz simulation timer. The first active player starts it and the last departure pauses it. Joining players receive the live clock with each initial world-data response. A transition during section transfer is replayed through the same revision check as terrain changes. The disk worker snapshots calendar and edit state under one read lock. Player, NPC and item simulation can use the same owner in reference tick order.

The roster serializes world-item allocation, pickup reservations, chest access and resource edits. Ground items occupy 400 slots and use owner updates for position and velocity. The server checks pickup candidates every 100 ms and requests a release from the previous owner before transferring a live reservation. Item physics, inventory capacity and specialized pickup ranges remain incomplete.

A sparse map holds the latest tile and chest slot changes. For an active section request the roster snapshots its tile and chest changes before enqueuing the transfer. The socket uses the same FIFO for that snapshot and subsequent patches, and compression stays on the terrain workers. Terrain patches go to clients whose queues include the section. A revision taken before the initial transfer lets joining sockets catch up with changes before packet 129. One blocking worker writes each requested checkpoint using the changes since its last successful publication. It copies edits and their revision under the read lock, releases the lock before doing file I/O, and preserves later edits for the next save. It compares the previous image against disk before rotating backups and atomically replacing the world. The listener drains the roster on shutdown before requesting the final checkpoint.

Supported town residents keep their saved NPC slot, position, name, variation and housing state. The terrain sender includes local residents by section and the full list at world entry. The roster answers name requests using the same resident slots.

Team membership and PvP status belong to the active connection. The roster initializes team from the stock spawn message, includes both values in join snapshots and decides party-chat recipients when it processes the command.

A death report updates the roster's life and PvP death state and releases the player's open chest. A later spawn replaces the prior position, team and spawn record. Health uploads, item reservation selection, door obstruction checks and joining snapshots read the same player state.

The roster tracks the last reported projectile key and position per client and local slot for the supported projectile messages. It verifies the key against the connection and clears the remaining entries on departure. A delayed removal with an older generation leaves the tracked newer instance intact.

A world-clock tick updates the save revision. Tile and container mutations also record a content revision, allowing saves and joins to skip the terrain and container journals when only time has advanced. A clock-only checkpoint copies the original sections and validates the new world image before publishing.

The roster owns sign opening and edits. The world journal tracks the sign slot and text revision. A section snapshots sign text under the same lock as its terrain, and initial catch-up sends edits made during the transfer. The disk worker includes sign records in the same validated checkpoint as the calendar, terrain and containers.

The roster resolves each switch press from one view of the terrain. The traversal follows connected cells by color, deduplicates loops and computes the final block states before committing them together. The visit limit bounds the work for one press. The roster groups resulting patches by section for peers already in the world.
