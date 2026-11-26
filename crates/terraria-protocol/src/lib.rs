//! The direct TCP protocol of Terraria 1.4.6.
mod death;
pub use death::{DeathReason, PlayerDeath, PlayerHurt};
mod banners;
pub use banners::{BANNER_TYPES, BannerState};
mod items;
mod liquid;
pub use items::{DroppedItem, ItemOwner, NEW_ITEM};
pub use liquid::LiquidUpdate;
mod movement;
pub use movement::Movement;
mod npc;
pub use npc::{NpcLife, NpcSync};
mod packet;
mod projectile;
pub use projectile::{ProjectileKey, ProjectileRemove, ProjectileSync};
mod server_info;
mod text;
mod tile_patch;
pub use server_info::{PlayerPlatform, ServerInfo};
pub use tile_patch::TilePatch;
mod wire;

pub use packet::{Appearance, Equipment, MAX_BUFFS, Packet, Spawn, Vitals};
pub use text::NetworkText;
pub use wire::{Error, Frame, MAX_FRAME, frame_len};

pub const GAME_VERSION: &str = "1.4.6";
pub const GREETING: &str = "Terraria329";

mod world;
pub use world::WorldData;
pub mod section;
