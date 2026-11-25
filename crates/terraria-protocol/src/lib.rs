//! The direct TCP protocol of Terraria 1.4.6.
mod banners;
pub use banners::{BANNER_TYPES, BannerState};
mod items;
pub use items::{DroppedItem, ItemOwner, NEW_ITEM};
mod movement;
pub use movement::Movement;
mod packet;
mod server_info;
mod text;
pub use server_info::{PlayerPlatform, ServerInfo};
mod wire;

pub use packet::{Appearance, Equipment, MAX_BUFFS, Packet, Spawn, Vitals};
pub use text::NetworkText;
pub use wire::{Error, Frame, MAX_FRAME, frame_len};

pub const GAME_VERSION: &str = "1.4.6";
pub const GREETING: &str = "Terraria329";

mod world;
pub use world::WorldData;
pub mod section;
