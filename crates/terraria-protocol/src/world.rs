//! WorldData (7), including the fixed graphics-style fields required on the wire.
use bytes::{BufMut, BytesMut};
use crate::wire::{Error, Reader, put_string};

#[derive(Debug, Clone, PartialEq)]
pub struct WorldData {
    pub time: i32,
    pub time_flags: u8,
    pub moon_phase: u8,
    pub size: [i16; 2],
    pub spawn: [i16; 2],
    pub layers: [i16; 2],
    pub id: i32,
    pub name: String,
    pub game_mode: u8,
    pub guid: [u8; 16],
    pub generator_version: u64,
    pub moon_type: u8,
    pub backgrounds: [u8; 13],
    pub biome_backgrounds: [u8; 3],
    pub wind: f32,
    pub clouds: u8,
    pub tree_x: [i32; 3],
    pub tree_styles: [u8; 4],
    pub cave_x: [i32; 3],
    pub cave_styles: [u8; 4],
    pub tree_tops: [u8; 13],
    pub rain: f32,
    pub progression: [u8; 11],
    pub sundial: u8,
    pub moondial: u8,
    pub ores: [i16; 7],
    pub invasion: i8,
    pub lobby: u64,
    pub sandstorm: f32,
    pub extra_spawns: Vec<[i16; 2]>,
    pub dungeon: [i16; 2],
}

impl WorldData {
    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        let time = r.i32()?;
        let time_flags = r.byte()?;
        let moon_phase = r.byte()?;
        let size = r.i16s()?;
        let spawn = r.i16s()?;
        let layers = r.i16s()?;
        let id = r.i32()?;
        let name = r.string()?.to_owned();
        let game_mode = r.byte()?;
        let guid = r.take()?;
        let generator_version = u64::from_le_bytes(r.take()?);
        let moon_type = r.byte()?;
        let backgrounds = r.take()?;
        let biome_backgrounds = r.take()?;
        let wind = r.f32()?;
        let clouds = r.byte()?;
        let tree_x = r.i32s()?;
        let tree_styles = r.take()?;
        let cave_x = r.i32s()?;
        let cave_styles = r.take()?;
        let tree_tops = r.take()?;
        let rain = r.f32()?;
        let progression = r.take()?;
        let sundial = r.byte()?;
        let moondial = r.byte()?;
        let ores = r.i16s()?;
        let invasion = r.byte()? as i8;
        let lobby = u64::from_le_bytes(r.take()?);
        let sandstorm = r.f32()?;
        let count = r.byte()?;
        let extra_spawns = (0..count).map(|_|r.i16s()).collect::<Result<_,_>>()?;
        let dungeon = r.i16s()?;
        Ok(Self { time,time_flags,moon_phase,size,spawn,layers,id,name,game_mode,guid,generator_version,moon_type,backgrounds,biome_backgrounds,wind,clouds,tree_x,tree_styles,cave_x,cave_styles,tree_tops,rain,progression,sundial,moondial,ores,invasion,lobby,sandstorm,extra_spawns,dungeon })
    }
    pub(crate) fn write(&self, w: &mut BytesMut) -> Result<(), Error> {
        w.put_i32_le(self.time);
        w.extend_from_slice(&[self.time_flags,self.moon_phase]);
        for v in self.size.into_iter().chain(self.spawn).chain(self.layers) { w.put_i16_le(v); }
        w.put_i32_le(self.id);
        put_string(w,&self.name)?;
        w.put_u8(self.game_mode);
        w.extend_from_slice(&self.guid);
        w.put_u64_le(self.generator_version);
        w.put_u8(self.moon_type);
        w.extend_from_slice(&self.backgrounds);
        w.extend_from_slice(&self.biome_backgrounds);
        w.put_f32_le(self.wind);
        w.put_u8(self.clouds);
        for v in self.tree_x { w.put_i32_le(v); }
        w.extend_from_slice(&self.tree_styles);
        for v in self.cave_x { w.put_i32_le(v); }
        w.extend_from_slice(&self.cave_styles);
        w.extend_from_slice(&self.tree_tops);
        w.put_f32_le(self.rain);
        w.extend_from_slice(&self.progression);
        w.extend_from_slice(&[self.sundial,self.moondial]);
        for v in self.ores { w.put_i16_le(v); }
        w.put_i8(self.invasion);
        w.put_u64_le(self.lobby);
        w.put_f32_le(self.sandstorm);
        w.put_u8(self.extra_spawns.len().try_into().map_err(|_|Error::Invalid("extra spawn count"))?);
        for p in self.extra_spawns.iter().chain([&self.dungeon]) {
            for v in p { w.put_i16_le(*v); }
        }
        Ok(())
    }
}
