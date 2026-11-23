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
| terraria-world | Validated immutable world image, column index, container inspection and lossless save. |
| terraria-protocol | Version-specific typed packets, fallible field codecs and immutable Bytes frames. It has no sockets or global game state. |
| terraria-network | Tokio listener, bounded admission, socket I/O, connection phases and per-connection character staging. |
| terraria-server | TOML configuration, structured logging, startup and shutdown. |
| terraria-testkit | Headless TCP client and semantic comparison of two live servers. |

The listener alone owns the free slots. Each admitted socket gets one task and one staging character. A JoinSet returns slots to the listener on completion. A full listener sends the vanilla full-server kick with a bounded write deadline. Reads and writes run in order with TCP backpressure. No unbounded application queue is needed during the handshake. A 90-second absolute join deadline also covers partial reads and blocked writes. Shutdown stops admission and closes all staging sessions.

## Reference entry points

| System | Entry points and observed ownership |
| --- | --- |
| Connections | `Netplay.OnConnectionAccepted` and `FindNextOpenClientSlot` own slot allocation. `RemoteClient` owns a socket, state, termination flags, timeout and sent-section tracking. `NetMessage.CheckBytes` splits the receive buffer and dispatches complete frames. |
| Messages | `ID.MessageID` names the IDs. `MessageBuffer.GetData` (RVA 0x14d2e4) handles mode and phase checks, field decoding and effects. `NetMessage.SendData` (0x167c7c) writes the matching fields and dispatches to the intended recipients. |
| Player and inventory | Clients upload join-time appearance, vitals, loadout, buffs and equipment. The receiving server selects the player by connection. `PlayerItemSlotID..cctor` allocates the 990 serialized slot indices. `Item.SetDefaults` and the slot reference type link inventory entries to game definitions. Staged Rust data has no in-world effect yet. |
| World files and generation | `IO.WorldFile.LoadWorld` selects a versioned reader. `LoadWorld_Version2` uses a directory, frame-importance table and section loaders. `SaveWorld_Version2` writes the sections and backpatches their end offsets. `WorldGen` owns generation and terrain updates. `Tile` holds active type, wall, liquid, frames and packed flags. |
| World transmission | Request 6 transitions vanilla state 1 to 2 and triggers WorldData (7). `NetMessage.SendSection` and `RemoteClient.TileSections` track delivery. `Netplay.GetSectionX/Y` divides tile coordinates by 200 and 150 respectively. `CompressTileBlock` writes x/y as i32 and width/height as i16 inside a DeflateStream, then delegates tile and entity encoding to `CompressTileBlock_Inner`. |
| Tick and entities | `Main.DedServ` drives a 1000/60 ms accumulator. With no fully joined player it services networking. With joined players it calls the game update. In `DoUpdateInWorld_Inner` the observed relative order is players, NPC spawning, pressure plates, NPCs, projectiles, items, time, world generation/invasion and server/client updates. Other passes occur between these calls. `Player`, `NPC`, `Projectile` and `Item` state is reached through the world update routines. |
| Shutdown | `Main.DedServ` handles console input and server-loop termination. `WorldFile.InternalSaveWorld` and `DoRollingBackups` are the reference save/backup entry points. |

The planned simulation will own the world and active entities in one fixed-step loop. Network tasks will hand off validated commands via bounded channels once world entry is implemented. Tick order, RNG consumption, individual AI decisions, interactions and persistence of gameplay changes require further implementation and comparison.
