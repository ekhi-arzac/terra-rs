# Direct TCP protocol, Terraria 1.4.5.8

## Frame and field encodings

`NetMessage.CheckBytes` (RVA 0x16c828) reads a little-endian u16 total length. Valid lengths span 3 through 65535. A u8 message ID follows, then exactly `length - 3` payload bytes. The prefix includes itself and the ID. A TCP read can contain part of a frame or several frames. The Rust transport bounds each payload by this prefix and reports EOF within a frame as an error.

Multibyte numbers are little-endian. A bool occupies one byte. The writer emits 0 or 1 and the reader accepts any nonzero byte as true. A string is a nonnegative .NET 7-bit encoded i32 UTF-8 **byte count** followed immediately by its bytes. Each prefix byte contributes seven bits, least significant group first, and bit 7 marks continuation. Rust rejects overflow, truncation, invalid UTF-8 and trailing fields. A Color here is R, G, B as three bytes.

`NetworkText.Serialize` writes a mode byte and a string. Mode 0 is a literal. Modes 1 (formattable) and 2 (localization key) append a u8 substitution count followed by that many nested NetworkText values. Rust limits nesting to 32 levels. `Lang.InitializeLegacyLocalization` maps legacy multiplayer strings to keys of the form `LegacyMultiplayer.N`.

## Implemented layouts

Fields in each row appear in wire order. Directions describe the direct join sequence. Several character packets also travel server-to-client once peers are in play.

| ID | Direction | Payload fields |
| ---: | --- | --- |
| 1 Hello | C → S | string greeting. This build sends `Terraria326`. |
| 2 Kick | S → C | NetworkText reason. |
| 3 PlayerInfo / assign slot | S → C | u8 player, bool special flag. The dedicated writer emits false. The client stores it as server special flag bit 2. |
| 4 SyncPlayer | C → S | u8 player, u8 skin, u8 voice, f32 voice pitch offset, u8 hair, string name, u8 hair dye, u16 hidden accessory mask, u8 hidden misc mask, seven RGB colors (hair, skin, eyes, shirt, undershirt, pants, shoes), u8 difficulty/accessory flags, u8 torch/cart flags, u8 permanent-upgrade flags. |
| 5 SyncEquipment | C → S | u8 player, i16 equipment slot, i16 stack, u8 prefix, i16 item type, u8 flags (bit 0 favorited, bit 1 set from the sender's extra sync argument). |
| 6 RequestWorldData | C → S | Empty. |
| 16 PlayerLifeMana | C → S | u8 player, i16 current life, i16 maximum life. |
| 37 RequestPassword | S → C | Empty. |
| 38 SendPassword | C → S | string password. |
| 42 Unknown42 / mana | C → S | u8 player, i16 current mana, i16 maximum mana. |
| 50 PlayerBuffs | C → S | u8 player, zero or more nonzero u16 buff types followed by a zero u16 terminator. This build has 44 buff slots. |
| 68 Unknown68 / client UUID | C → S | string identifier. GetData reads and discards it. |
| 147 SyncLoadout | C → S | u8 player, u8 loadout index, u16 hidden accessory mask. |
| 161 HostToken | C → S | string token. Optional on join. A direct server without a configured host token gives no host privilege. |

For packet 4, difficulty flags at bits 0, 1, 2, 3 mean mediumcore, hardcore, extra accessory, journey. Torch/cart bits 0–4 track biome torch use, happy torch time, biome torch unlock, super cart unlock and cart enable. Upgrade bits 0–6 correspond to Aegis Crystal, Aegis Fruit, Arcane Crystal, Galaxy Pearl, Gummy Worm, Ambrosia and Artisan Bread.

## Join and rejection

1. At accept, the reference picks the lowest available slot below its limit. A full server immediately sends packet 2 with key `CLI.ServerIsFull`.
2. In vanilla state 0, ID 1 must match the exact greeting. A version mismatch gets key `LegacyMultiplayer.4`. An open server returns ID 3. A protected server enters state -1 and sends ID 37.
3. A matching ID 38 password yields ID 3 and state 1. A wrong password yields `LegacyMultiplayer.1`. Rust accepts ID 38 only while awaiting the password.
4. After ID 3 the client emits 4, 68, 16, 42, 50, 147, repeated 5s for inventory and banks, optional 161, then 6. The connection determines player identity. The Rust session bounds equipment indices to 0–989 and applies the observed appearance and life limits. The name limit uses 20 UTF-16 code units after trimming.
5. At ID 6 the current Rust implementation sends a literal disconnect explaining that world synchronization is unavailable. Vanilla would send ID 7. Unknown messages and illegal phase transitions terminate the session. Rust refuses malformed frames and enforces strict field consumption.

Character and packet limits come from `GetData`, `SendData`, `Player..cctor`, `PlayerItemSlotID..cctor`, `ItemID..cctor`, `PlayerVariantID..cctor` and `WriteAccessoryVisibility`. The signed wire slot in packet 5 is distinct from the server-assigned u8 player.
