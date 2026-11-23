//! The direct TCP protocol of the supplied Terraria 1.4.5.8 server.
mod packet;
mod text;
mod wire;

pub use packet::{Appearance, Equipment, MAX_BUFFS, Packet, Vitals};
pub use text::NetworkText;
pub use wire::{Error, Frame, MAX_FRAME, frame_len};

pub const GAME_VERSION: &str = "1.4.5.8";
pub const GREETING: &str = "Terraria326";

mod world;
pub use world::WorldData;
