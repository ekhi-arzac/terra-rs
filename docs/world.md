# World file release 326

`terraria-world` reads the format written by the supplied 1.4.5.8 server. Configure `world` in the Rust server TOML to validate and index a file before accepting connections. World entry reads the extended header, builds packet 7 and streams indexed terrain and chest contents. The standalone utility can inspect a world, look up a tile or save an exact copy.

```sh
cargo run -p terraria-world -- inspect existing.wld
cargo run -p terraria-world -- npcs existing.wld
cargo run -p terraria-world -- tile existing.wld 200 100
cargo run -p terraria-world -- copy existing.wld output.wld
```

## Create a world from the terminal

From the repository root on Linux x86-64, start the bundled 1.4.5.8 dedicated server.

```sh
chmod +x Linux/TerrariaServer.bin.x86_64
./Linux/TerrariaServer.bin.x86_64
```

Enter `n` in the world menu and follow the prompts. Select the generated world to start the official server, then enter `exit` in its console to save and stop it. On Linux the default world directory is `~/.local/share/Terraria/Worlds`. List the actual filenames and their absolute paths with:

```sh
ls -1 "$HOME"/.local/share/Terraria/Worlds/*.wld
```

Set `world` in the Rust server TOML to the full path of the desired file, including its exact spelling and capitalization.

## Directory and metadata

All numbers use little-endian encoding. The signed i32 version is followed by seven ASCII bytes `relogic`, a u8 file type (2 for world), a u32 revision and a u64 flag mask (bit 0 marks a favorite). An i16 count precedes eleven i32 absolute offsets. An i16 tile-type count and a low-bit-first packed frame-importance table end at the first offset. The reference initializes 754 tile types.

The offsets point to the world header, tiles, chests, signs, NPCs, tile entities, weighted pressure plates, town manager, bestiary, creative powers and footer. They must increase within the file. The header begins with the world name and seed as .NET strings, generator version as u64, 16 GUID bytes in .NET order, i32 world ID, four i32 pixel bounds, then height and width as i32. The loader also reads game mode, spawn, surface and rock layers, time and day flag. The footer contains a true byte, the world name and world ID. The loader verifies both identities.

## Tile records

Columns run left to right. Records within a column run top to bottom. Zero to three continuation bytes follow the first header byte. Bit 0 of each of the first three headers marks the next header as present.

| Header | Bits | Meaning |
| --- | --- | --- |
| First | 1, 2, 3–4, 5, 6–7 | Active block, wall present, liquid kind, two-byte block ID, repeat encoding |
| Second | 1–3, 4–6 | First three wires, half-block/slope encoding |
| Third | 1–2, 3–4, 5, 6, 7 | Actuator/inactive, block/wall paint, fourth wire, high wall byte, shimmer |
| Fourth | 1–4 | Invisible block/wall and fullbright block/wall |

The optional fields follow this order. Read the block ID, its two signed i16 frame coordinates if marked important, block paint, low wall byte, wall paint, liquid amount and high wall byte. A shimmer bit overrides the liquid kind. An RLE mode of 0 has no count, 1 has a u8 count, and 2 or 3 has an i16 count. The count describes **additional** copies. Negative counts and runs crossing the bottom of a column are rejected. The returned tile fields represent their on-disk values. Vanilla can apply post-load framing and terrain corrections.

The loader validates the complete tile section once and stores a byte offset per column. A coordinate query scans runs only in that column. The source image remains compressed in memory.

## Containers and lossless saving

The chest section starts with an i16 count. Each record has i32 x/y, name string, i32 capacity and one i16 stack per slot. A nonzero stack adds an i32 item type and u8 prefix. A negative stack decodes as a single item in the version-326 reader. Empty slots occupy two bytes. Sign records follow their i16 count with string text and i32 x/y. The parser exposes serialized containers, before tile-based validation or deduplication.

The immutable image retains the original bytes of every section. The join path reads persisted progression, seed flags, biome styles, ore tiers, weather, team spawns and dungeon coordinates. The tile index pass also counts interior active tiles for the Skyblock density flag and checks the release’s dungeon block and wall types. Full tile entity records use the same eleven type layouts on disk and in section packets. As in the reference loader, the network ID is assigned from file order and the last record at a duplicated position wins. The NPC section contains the list of shimmered NPC types, followed by two boolean-terminated lists. Town records include type, name, pixel position, housing, optional variation and despawn flag. Other persisted NPC records contain type and pixel position. File order determines the initial slot. Bestiary and pressure plate data remain opaque. Saving writes the original bytes to a unique temporary file in the destination directory, syncs it, reads it back for comparison, atomically publishes it on supported filesystems and syncs the parent on Unix. Loading checks the interpreted sections and the directory. Gameplay saves rewrite edited tile runs and chest slots and retain the three preceding world revisions. The newest backup ends in `.terra-rs.bak`. The two older slots end in `.even` and `.odd` and alternate by world revision.

The Rust loader sets local ceilings of 512 MiB/file, 32767 tiles per axis and 64 million tiles/world. Those are resource bounds. The binary evidence for the format is in `IO.FileMetadata.Write`, `IO.WorldFile.SaveWorld_Version2`, `SaveWorldHeader`, `SaveWorldFlags`, `LoadWorldTiles`, `LoadChests`, `LoadSigns` and `LoadFooter`.


The clock checkpoint preserves fractional phase time as an f64, the day flag, moon phase, blood moon and eclipse state, both dial cooldowns and both fast-forward flags. Packet 7 truncates the fractional time to an integer. Supported files also carry the saved creative time-freeze toggle and time-speed slider in section 9. The reader applies these values to the simulation and preserves the section bytes when saving.

Sign text changes keep their saved record slots. Opening a valid sign tile without a text record appends an empty record. A checkpoint writes changes to the sign section and adjusts following section offsets while retaining the following data bytes.

Liquid cell edits travel through the tile journal. Saving a dry cell omits the liquid field. Saving a filled cell preserves the type and amount alongside its other tile fields.
