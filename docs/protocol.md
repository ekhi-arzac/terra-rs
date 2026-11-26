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
| 17 | Action byte, two i16 tile coordinates, i16 value, style byte. Actions 0, 1, 2, 3, 4 and 20 cover block break, block place, wall break, wall place, no-drop break and break-result confirmation. Supported edits publish the committed tile state using packet 20. A tree cut sends one patch per affected section and a separate wood drop. |
| 19 | Door action byte, two i16 tile coordinates and direction byte (zero for left, nonzero for right). Action 0 opens a hinged door and action 1 closes it. Rust sends the committed tiles to subscribed players in packet 20. |
| 79 | Object placement with two i16 anchor coordinates, i16 tile type, i16 style, u8 alternate, i8 random variation, and bool facing right. Hinged doors use alternate 0, 1 or 2 for the three anchor rows. Early crafting stations also accept packet 17 action 1. Successful placements and breaks publish the entire footprint in a single packet 20, including across section boundaries. |
| 20 | Active tile patch with two i16 origin coordinates, u8 width, u8 height and a reason byte. Columns contain uncompressed tile state with three flag bytes and optional paint, block, frame, wall and liquid fields. The server uses this to publish committed edits, correct rejected predictions and catch up joining clients. |
| 21 | i16 world slot, two f32 position components, two f32 velocity components, i16 stack, prefix byte, flags byte, i16 item ID. Flag 4 adds a shimmer boolean and f32 time. Flag 8 adds the enemy pickup delay byte. The low two bits select spawn ownership. Slot 400 requests a new stack. |
| 22 | i16 world slot, owner byte, 7-bit reservation time, delayed player byte, 7-bit delay time, two f32 position components. Player 255 represents an unowned stack. |
| 39, 151 | Release the reserved slot (i16 slot and boolean), or remove the picked-up stack (i16 slot). The connection must own the stack to change or remove it. A rejected change or removal restores the current stack and reservation on the requesting socket. |
| 31, 155, 32 | Request a chest by two i16 tile coordinates. The server replies with capacity and all slots. The current user can update a slot with its stack, prefix and type. |
| 34 | Action byte, two i16 tile coordinates, i16 style and i16 chest slot. For standard wooden chests, action 0 places using the bottom-left anchor and action 1 removes using the top-left tile. The server assigns the slot. A supported chest has 40 slots. |
| 33, 80 | Chest open/close uses i16 chest index, two i16 coordinates and a name marker with an optional string. Index −1 closes the chest. ID 80 announces the player's slot and current chest index to peers. The name marker counts UTF-16 code units up to 20. A marker of 255 clears the name. The open chest on the sending connection owns the rename. |
| 69 | Three i16 fields identify the chest index and its tile coordinates. Clients can use index −1 for lookup by coordinates. A server reply also carries the .NET name string. A stock client request may include a cached name. |

## Town NPCs on release 329

| ID | Direction and layout |
| --- | --- |
| 23 | Server update. The slot and generation occupy one byte each. Position and velocity each contain two f32 components, followed by a u16 target and two flag bytes. In the first byte bits 0 and 1 encode facing and vertical direction, bits 2–5 select four optional f32 AI values, bit 6 encodes sprite direction, and bit 7 signals full life. The selected AI values precede the i16 net type. In the second byte bits 0–4 signal player scaling, statue origin, difficulty override, new spawn, and shimmer. Scaling adds a u8 player count and difficulty adds an f32. For partial life, a width byte of 1, 2 or 4 precedes signed life of that width. Catchable types append the release owner's byte. |
| 54 | Server buff list. An i16 NPC slot precedes up to 20 pairs of u16 buff type and remaining time. A zero u16 type terminates the list. |
| 56 | The client requests an i16 NPC slot. The server replies with the slot, a .NET name string and the i32 variation. |
| 60 | An i16 slot, i16 home X and Y in tiles, and a status byte. Server status 1 is homeless and 2 is an assigned house. Status 0 is a housed resident without a room-manager assignment. Player-initiated moves and eviction are pending. |

Section delivery includes the supported local residents. The full initial resident and buff list precedes the spawn invitation. Housing follows player spawn and precedes the entry acknowledgement.


## Shared clock

Packet 7 carries the current integer phase time, a flag byte with day, blood moon and eclipse in bits 0–2, and the moon phase byte. Progression byte 2 bit 1 indicates fast-forward to dawn and byte 8 bit 7 indicates fast-forward to dusk (zero-based indices). The two dial cooldown bytes follow the progression flags. Each initial world-data response includes current time. Dial activation and phase transitions broadcast packet 7.

Packet 51 carries a player byte and an action byte. Actions 3 and 6 request the enchanted sundial and moondial. The connection identifies the player. A ready dial starts fast-forward and sets its cooldown to 8. Another request during cooldown produces no update. The matching phase boundary ends fast-forward and reduces the cooldown to 7.


## Teams and PvP

Packet 30 carries a player byte and a PvP-enabled boolean. Packet 45 carries a player byte and a team in 0–5 (none, red, green, blue, yellow, pink). Both clients and servers use these layouts. An active connection supplies its own player identity. Peers receive the changed setting and a joining client receives both settings for each current player. PvP toggle announcements reach everyone. Team-change announcements reach the sender and members of the old and new nonzero teams.

The text module's `Party` command delivers literal text in the sender’s team color to current teammates, including the sender. A player without a team receives the localized `LegacyMultiplayer.10` response. On a team, an empty message produces no delivery.


## Player death and respawn

Packet 118 begins with a player byte and a death-reason mask. In increasing bit order the selected fields are an i16 source player, i16 NPC slot, i16 projectile slot, u8 other cause, i16 projectile type, i16 item type, u8 item prefix and a .NET custom reason string. The fixed tail contains i16 damage, a byte encoding direction plus one, and a flag byte with PvP in bit 0. The server identifies the victim by the reporting socket and relays the death to peers. It sends the announcement as server-authored localized text.

Late entry includes packet 135 (one player byte) before the life packet for a player with zero health. A subsequent player spawn uses packet 12 with the normal spawn layout and reaches other active clients. The client continues to report its vitals in packet 16.

Packet 117 uses the same player byte, reason mask and fields, damage and direction encoding as packet 118. Its flag byte uses bits 0–2 for critical, PvP and dodgeable. A signed immunity slot follows, with values −1 through 5 in this version. A client may report a hit to itself or to another active player when both have enabled PvP. Rust uses the sender’s slot for cross-player attribution and relays the report to the other sockets. Packet 16 supplies the injured client’s latest health.

## Projectile synchronization

Packets 27 and 29 start with a little-endian u32 key. Bits 0–7 are the spawning player, bits 8–17 are the local slot, and bits 18–31 are the generation. Rust relays only keys belonging to the sending socket. Packet 29 appends two f32 coordinates.

Packet 27 appends two f32 position components, two f32 velocity components, an i16 type and a flag byte. Bit 2 selects an extra flag byte. The optional fields are two f32 AI values (bits 0–1), u16 banner (bit 3), i16 damage (bit 4), f32 knockback (bit 5), i16 original damage (bit 6), and a third f32 AI value (extra byte bit 0). Rust relays type 1 and 14 updates and maintains enough state to clear these projectiles when their owner leaves. Projectiles remain session-local.

## Sign text

Packet 46 requests a sign with two i16 tile coordinates. A click on any tile in the sign footprint resolves to the top-left tile using its stored frame. Packet 47 carries the i16 sign slot, two i16 top-left coordinates, a .NET UTF-8 string, a player byte and a flag byte. With the requesting player in the packet and flag bit 0 clear the client opens the sign. Rust verifies the slot and coordinates against the current sign before accepting a text edit and uses the editing connection as the player. Subscribed peers receive text changes. Join catch-up and corrections set bit 0 to retain the current UI state.

## Liquid cell updates

Packet 48 carries two i16 tile coordinates, an amount byte, and a liquid type byte (0 water, 1 lava, 2 honey, 3 shimmer). The server accepts edits to streamed tiles. An amount of zero clears a cell and relays packet 48 to peers in the section. A changed nonzero amount emits the server-to-client liquid module. A joining client receives the committed cells through packet 10 or a catch-up patch.

Net module 0 in packet 82 begins with a u16 entry count. Each entry has a packed little-endian u32 coordinate with x in the upper half and y in the lower half, then the amount and type bytes. Rust bounds the entry count by the frame and accepts the module from the server direction.

## Hammering and wire edits

Packet 17 actions 7 and 23 toggle a half-block. The stock client uses action 23 with value zero to clear an existing slope as it toggles. Action 14 sets a slope in the range 0–4, where zero restores the full block. Rust supports these edits for common terrain and checks for protected tiles above the block. Hammer actions publish the resulting tile in packet 20.

Actions 5/6, 10/11, 12/13 and 16/17 place and remove red, blue, green and yellow wire. Actions 8/9 place and remove an actuator. Each successful removal produces one wire (item 530) or actuator (item 849). The flags persist independently of foreground blocks.

Packet 59 carries two i16 coordinates for a switch press. Rust resolves pulses from one-tile switches through adjacent ordinary wires of the same color. Each color can trigger a connected actuator once per press. Packet 17 action 19 toggles a single eligible actuator directly. The server sends the changed switch frame and actuator states in packet 20. A late join receives both from the section snapshots.
