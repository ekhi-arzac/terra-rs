# Direct TCP protocol, Terraria 1.4.6

## Frame and field encodings

`NetMessage.CheckBytes` (RVA 0x15f2ec) reads a little-endian u16 total length. Valid lengths span 3 through 65535. A u8 message ID follows, then exactly `length - 3` payload bytes. The prefix includes itself and the ID. A TCP read can contain part of a frame or several frames. The Rust transport bounds each payload by this prefix and reports EOF within a frame as an error.

Multibyte numbers are little-endian. A bool occupies one byte. The writer emits 0 or 1 and the reader accepts any nonzero byte as true. A string is a nonnegative .NET 7-bit encoded i32 UTF-8 **byte count** followed immediately by its bytes. Each prefix byte contributes seven bits, least significant group first, and bit 7 marks continuation. Rust rejects overflow, truncation, invalid UTF-8 and trailing fields. A Color here is R, G, B as three bytes.

`NetworkText.Serialize` writes a mode byte and a string. Mode 0 is a literal. Modes 1 (formattable) and 2 (localization key) append a u8 substitution count followed by that many nested NetworkText values. Rust limits nesting to 32 levels. `Lang.InitializeLegacyLocalization` maps legacy multiplayer strings to keys of the form `LegacyMultiplayer.N`.

## Implemented layouts

Fields in each row appear in wire order. Directions describe the direct join sequence. Several character packets also travel server-to-client once peers are in play.

| ID | Direction | Payload fields |
| ---: | --- | --- |
| 163 ServerInfo | S → C | Advertised port, world name, host, width, evil biome, difficulty, slot capacity, active players, note and optional mapped address. A world-backed server sends this on TCP connect. |
| 164 PlayerPlatformInfo | C → S | Player slot, platform byte, ID string, display-name string when platform is 5 and host boolean. Direct-IP sessions consume this during character upload. |
| 1 Hello | C → S | string greeting. This build sends `Terraria329`. |
| 2 Kick | S → C | NetworkText reason. |
| 3 PlayerInfo / assign slot | S → C | u8 player, bool special flag. The dedicated writer emits false. The client stores it as server special flag bit 2. |
| 4 SyncPlayer | C → S | u8 player, u8 skin, u8 voice, f32 voice pitch offset, u8 hair, string name, u8 hair dye, u16 hidden accessory mask, u8 hidden misc mask, seven RGB colors (hair, skin, eyes, shirt, undershirt, pants, shoes), u8 difficulty/accessory flags, u8 torch/cart flags, u8 permanent-upgrade flags. |
| 5 SyncEquipment | C → S | u8 player, i16 equipment slot, i16 stack, u8 prefix, i16 item type, u8 flags (bit 0 favorited, bit 1 set from the sender's extra sync argument). |
| 6 RequestWorldData | C → S | Empty. |
| 7 WorldData | S → C | World header and runtime fields in the order below. |
| 8 RequestTileData | C → S | i32 personal spawn X, i32 Y, u8 team. Use −1, −1 for the ordinary world spawn. |
| 9 StatusTextSize | S → C | i32 section count, NetworkText status, u8 flags. |
| 10 TileSection | S → C | One raw DEFLATE stream containing the rectangle described below. |
| 12 PlayerSpawn | Both | u8 player, i16 spawn X and Y, i32 respawn timer, i16 PvE and PvP death counts, u8 team, u8 spawn context. |
| 13 PlayerControls | Both | u8 player, four u8 flag masks, u8 selected item, position vector. Mask 2 bit 2 adds velocity, mask 2 bit 7 adds a u16 mount, mask 3 bit 6 adds two return-potion vectors, mask 4 bit 5 adds a camera vector. Vectors are two f32s in X, Y order. |
| 14 PlayerActive | S → C | u8 player, bool active. |
| 32 SyncChestItem | S → C during transfer | i16 chest index, u8 slot, i16 stack, u8 prefix, i16 item ID. Empty slots are transmitted. |
| 49 InitialSpawn | S → C | Empty. Invites the client to send packet 12. |
| 82 NetModules, text module | Both | u16 module ID 1. Client sends command-name string and text string. Server sends u8 author, NetworkText and RGB. The supported chat command is `Say`. |
| 82 NetModules, banner full state | S → C | u16 module ID 11, u8 subtype 0, i16 count and i32 kills per entry, i16 count and u16 claimable banners per entry. Release 326 sends 293 entries for each array. |
| 101 Lunar tower shields | S → C | Four u16 strengths in solar, vortex, nebula and stardust order. |
| 136 Cavern monster variants | S → C | Six u16 NPC types, three per species group. The world ID seeds their selection. |
| 129 FinishedConnectingToServer | S → C | Empty. |
| 155 SyncChestSize | S → C | i16 chest index, i16 capacity. |
| 159 RequestSection | C → S | u16 section X, u16 section Y. |
| 16 PlayerLifeMana | C → S | u8 player, i16 current life, i16 maximum life. |
| 36 PlayerZone | Both | u8 player, five u8 zone masks and u8 nearby town NPC count. The roster assigns the sender slot and forwards the update. |
| 154 Ping | Both | Empty. The server echoes the packet to its sender. |
| 37 RequestPassword | S → C | Empty. |
| 38 SendPassword | C → S | string password. |
| 42 Unknown42 / mana | C → S | u8 player, i16 current mana, i16 maximum mana. |
| 50 PlayerBuffs | C → S | u8 player, zero or more nonzero u16 buff types followed by a zero u16 terminator. This build has 44 buff slots. |
| 68 Unknown68 / client UUID | C → S | string identifier. GetData reads and discards it. |
| 147 SyncLoadout | C → S | u8 player, u8 loadout index, u16 hidden accessory mask. |
| 161 HostToken | C → S | string token. Optional on join. A direct server without a configured host token gives no host privilege. |

For packet 4, difficulty flags at bits 0, 1, 2, 3 mean mediumcore, hardcore, extra accessory, journey. Torch/cart bits 0–4 track biome torch use, happy torch time, biome torch unlock, super cart unlock and cart enable. Upgrade bits 0–6 correspond to Aegis Crystal, Aegis Fruit, Arcane Crystal, Galaxy Pearl, Gummy Worm, Ambrosia and Artisan Bread.

## Join and rejection

1. At accept, the reference picks the lowest available slot below its limit. A full server immediately sends packet 2 with key `CLI.ServerIsFull`. When a world is loaded, the Rust listener starts an admitted connection with packet 163.
2. In vanilla state 0, ID 1 must match the exact greeting. A version mismatch gets key `LegacyMultiplayer.4`. An open server returns ID 3. A protected server enters state -1 and sends ID 37.
3. A matching ID 38 password yields ID 3 and state 1. A wrong password yields `LegacyMultiplayer.1`. Rust accepts ID 38 only while awaiting the password.
4. After ID 3 the client emits 4, 68, 16, 42, 50, 147, 164, repeated 5s for inventory and banks, optional 161, then 6. The connection determines player identity. The Rust session bounds equipment indices to 0–989 and applies the observed appearance and life limits. The name limit uses 20 UTF-16 code units after trimming. Game mode 3 accepts Journey characters. Other modes accept non-Journey characters.
5. A configured world answers 6 with 7. Request 8 returns another 7, then status 9 and the sections surrounding the world spawn. A valid personal or team spawn can extend this area. The server visits section X first and then Y within each column, clips at the world edges and tracks sections sent on that connection.
6. For each section the native server sends 10, then 155 and one 32 per slot for each chest. After the terrain, the banner full state, lunar shields and cavern monster types are sent. Packet 49 then invites the client to spawn. A named character sends 12, the roster registers the active player and the server sends 129. Request 159 can fetch additional sections during play. Join phases enforce packet direction and ordering. During play, the server discards unimplemented release-329 message IDs and net modules without applying their contents. This includes the creative powers module (6) sent by the stock client on entry. The server logs discarded IDs, checks the layouts of implemented messages and rejects invalid framing.

Character and packet limits come from `GetData`, `SendData`, `Player..cctor`, `PlayerItemSlotID..cctor`, `ItemID..cctor`, `PlayerVariantID..cctor` and `WriteAccessoryVisibility`. The signed wire slot in packet 5 is distinct from the server-assigned u8 player.

## WorldData field order

1. i32 time, u8 day/blood moon/eclipse flags, u8 moon phase, six i16s for dimensions, spawn and layer heights, i32 world ID, string name, u8 game mode, 16 GUID bytes, u64 generator version, u8 moon type.
2. Thirteen background bytes and three biome background bytes, f32 wind and u8 cloud count. Trees and caves each use three i32 boundaries and four style bytes. Thirteen tree-top bytes follow.
3. f32 rain intensity, eleven progression and event flag bytes, two u8 dial cooldowns, seven i16 ore types, i8 invasion, u64 lobby ID and f32 target sandstorm severity.
4. u8 extra spawn count, that many pairs of i16 coordinates and one i16 pair for the dungeon.

The mappings between saved flags and individual packet bits are in `terraria-world/src/join.rs`. The direct-IP server uses the loaded time and weather. A running vanilla simulation subsequently advances these fields.

## Tile sections

The inflated stream starts with i32 X and Y in **tiles**, then i16 width and height. Normal rectangles contain 200 × 150 tiles in row order. Runs can cross rows. The tile records use the release's 754-entry frame-importance map and the chained headers, paints, wall and liquid fields documented for the world file. The high two bits of the first header select the width of the repeat count. Different valid run boundaries and DEFLATE encoders can represent identical terrain.

Three lists follow the last tile. An i16 count precedes each list. A chest or sign entry contains an i16 index, two i16 coordinates and a string. A full tile entity contains a u8 kind, i32 entity ID, two i16 coordinates and type-specific data. Types 0, 2, 9 and 10 carry two extra bytes, types 1, 4, 6 and 8 carry a five-byte item, and type 7 ends after the coordinates. A display doll carries two equipment masks, pose and an extra mask, followed by the selected five-byte items. A hat rack uses one mask for four slots. The decoder checks the tile total, list lengths, inflated size and end of the compressed stream.

## Items, chests and resource blocks

| ID | Wire fields and supported behavior |
| --- | --- |
| 17 | Action byte, two i16 tile coordinates, i16 value, style byte. Actions 0, 1, 4 and 20 cover ordinary break, place, no-drop break and break-result confirmation for supported blocks. A normal tree cut sends the original operation once to observing clients. The cutter has already applied the cut locally. The server records the detached tiles and publishes the wood drop to both sockets. |
| 20 | Active tile patch with two i16 origin coordinates, u8 width, u8 height and a reason byte. Columns contain uncompressed tile state with three flag bytes and optional paint, block, frame, wall and liquid fields. The server uses this for corrections and join catch-up. |
| 21 | i16 world slot, two f32 position components, two f32 velocity components, i16 stack, prefix byte, flags byte, i16 item ID. Flag 4 adds a shimmer boolean and f32 time. Flag 8 adds the enemy pickup delay byte. The low two bits select spawn ownership. Slot 400 requests a new stack. |
| 22 | i16 world slot, owner byte, 7-bit reservation time, delayed player byte, 7-bit delay time, two f32 position components. Player 255 represents an unowned stack. |
| 39, 151 | Release the reserved slot (i16 slot and boolean), or remove the picked-up stack (i16 slot). The connection must own the stack to change or remove it. |
| 31, 155, 32 | Request a chest by two i16 tile coordinates. The server replies with capacity and all slots. The current user can update a slot with its stack, prefix and type. |
| 33, 80 | Chest open/close uses i16 chest index, two i16 coordinates and a name marker with an optional string. Index −1 closes the chest. ID 80 announces the player's slot and current chest index to peers. Chest renaming remains pending. |
