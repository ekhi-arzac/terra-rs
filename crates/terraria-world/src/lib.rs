//! Lossless inspection of the sectioned world format written by releases 326 through 329.
mod cavern;
mod npcs;
pub use npcs::{Npc, TownNpc};
mod join;
pub use join::JoinState;
mod saving;
pub use saving::ChestStack;
mod sections;
use terraria_binary::tiles;
pub use terraria_binary::tiles::{Liquid, Tile};
use terraria_protocol::section::TileEntity;

use std::{
    fs::File,
    io::{self, Read, Seek, Write},
    ops::Range,
    path::Path,
};
use tempfile::NamedTempFile;
use terraria_binary::Reader;
use thiserror::Error;

pub const WORLD_VERSION: i32 = 329;
const MIN_WORLD_VERSION: i32 = 326;
const MAX_FILE: usize = 512 * 1024 * 1024;
const SECTION_COUNT: usize = 11;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("invalid world field: {0}")]
    Field(#[from] terraria_binary::Error),
    #[error("unsupported world version {0} (expected 326 through 329)")]
    Version(i32),
    #[error("invalid world: {0}")]
    Invalid(&'static str),
}

#[derive(Debug)]
pub struct Metadata {
    pub file_version: i32,
    pub name: String,
    pub seed: String,
    pub world_id: i32,
    pub guid_bytes: [u8; 16],
    pub generator_version: u64,
    pub revision: u32,
    pub favorite: bool,
    pub width: u32,
    pub height: u32,
    pub game_mode: i32,
    pub spawn: (i32, i32),
    pub surface: f64,
    pub rock_layer: f64,
    pub time: f64,
    pub daytime: bool,
}

#[derive(Debug, Clone)]
pub struct StoredItem {
    pub slot: u32,
    pub stack: i16,
    pub item_type: i32,
    pub prefix: u8,
}
#[derive(Debug, Clone)]
pub struct Chest {
    pub position: (i32, i32),
    pub name: String,
    pub capacity: u32,
    pub items: Vec<StoredItem>,
}
#[derive(Debug)]
pub struct Sign {
    pub position: (i32, i32),
    pub text: String,
}

pub struct World {
    data: Vec<u8>,
    meta: Metadata,
    flags: Range<usize>,
    important: Range<usize>,
    type_count: usize,
    /// Absolute file positions of the first tile in each column, plus the end.
    columns: Vec<usize>,
    offsets: [usize; SECTION_COUNT + 1],
    active_interior: u32,
    has_dungeon: bool,
    chests: Vec<Chest>,
    signs: Vec<Sign>,
    entities: Vec<TileEntity>,
    npcs: Vec<Npc>,
    shimmered: Vec<i32>,
}

impl World {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Error> {
        let file = File::open(path)?;
        if file.metadata()?.len() > MAX_FILE as u64 {
            return Err(Error::Invalid("file exceeds 512 MiB"));
        }
        let mut data = Vec::new();
        file.take(MAX_FILE as u64 + 1).read_to_end(&mut data)?;
        Self::parse(data)
    }

    pub fn parse(data: Vec<u8>) -> Result<Self, Error> {
        if data.len() > MAX_FILE {
            return Err(Error::Invalid("file exceeds 512 MiB"));
        }
        let mut r = Reader(&data);
        let version = r.i32()?;
        if !(MIN_WORLD_VERSION..=WORLD_VERSION).contains(&version) {
            return Err(Error::Version(version));
        }
        if &r.take::<8>()? != b"relogic\x02" {
            return Err(Error::Invalid("file signature or type"));
        }
        let revision = u32::from_le_bytes(r.take()?);
        let favorite = u64::from_le_bytes(r.take()?) & 1 != 0;
        if r.i16()? != SECTION_COUNT as i16 {
            return Err(Error::Invalid("section count"));
        }
        let mut offsets = [0; SECTION_COUNT + 1];
        for offset in &mut offsets[..SECTION_COUNT] {
            *offset =
                usize::try_from(r.i32()?).map_err(|_| Error::Invalid("negative section offset"))?;
        }
        offsets[SECTION_COUNT] = data.len();
        if offsets.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(Error::Invalid("section directory"));
        }
        let type_count =
            usize::try_from(r.i16()?).map_err(|_| Error::Invalid("tile type count"))?;
        let start = data.len() - r.0.len();
        r.bytes(type_count.div_ceil(8))?;
        let important = start..data.len() - r.0.len();
        if important.end != offsets[0] {
            return Err(Error::Invalid("header pointer"));
        }

        let mut header = Reader(&data[offsets[0]..offsets[1]]);
        let (meta, flags_len) = metadata(&mut header, version, revision, favorite)?;
        let flags = offsets[1] - flags_len..offsets[1];
        let mut footer = Reader(&data[offsets[10]..]);
        if footer.byte()? == 0 || footer.string()? != meta.name || footer.i32()? != meta.world_id {
            return Err(Error::Invalid("footer identity"));
        }
        if !footer.0.is_empty() {
            return Err(Error::Invalid("data after footer"));
        }

        let mut tiles = Reader(&data[offsets[1]..offsets[2]]);
        let mut columns = Vec::with_capacity(meta.width as usize + 1);
        let mut active_interior = 0;
        let mut has_dungeon = false;
        for x in 0..meta.width {
            columns.push(offsets[2] - tiles.0.len());
            let mut rows_left = meta.height;
            while rows_left > 0 {
                let (tile, run) =
                    tiles::read_run(&mut tiles, &data[important.clone()], type_count)?;
                let y = meta.height - rows_left;
                let end = (y + run).min(meta.height.saturating_sub(40));
                if (40..meta.width.saturating_sub(40)).contains(&x) && end > y.max(40) {
                    if tile.block.is_some() {
                        active_interior += end - y.max(40);
                    }
                    has_dungeon |= matches!(tile.block, Some(41 | 43 | 44 | 677..=679))
                        || matches!(tile.wall,7..=9|94..=99);
                }
                rows_left = rows_left
                    .checked_sub(run)
                    .ok_or(Error::Invalid("tile run crosses column boundary"))?;
            }
        }
        if !tiles.0.is_empty() {
            return Err(Error::Invalid("tile section length"));
        }
        columns.push(offsets[2]);
        let chests = read_chests(&data[offsets[2]..offsets[3]])?;
        let signs = read_signs(&data[offsets[3]..offsets[4]])?;
        let (shimmered, npcs) = npcs::read(&data[offsets[4]..offsets[5]])?;
        let entities = sections::entities(&data[offsets[5]..offsets[6]])?;
        Ok(Self {
            data,
            entities,
            npcs,
            shimmered,
            meta,
            flags,
            important,
            type_count,
            columns,
            offsets,
            active_interior,
            has_dungeon,
            chests,
            signs,
        })
    }

    pub fn npcs(&self) -> &[Npc] {
        &self.npcs
    }
    pub fn shimmered_npc_types(&self) -> &[i32] {
        &self.shimmered
    }
    pub fn metadata(&self) -> &Metadata {
        &self.meta
    }
    pub fn chests(&self) -> &[Chest] {
        &self.chests
    }
    pub fn signs(&self) -> &[Sign] {
        &self.signs
    }

    pub fn tile(&self, x: u32, y: u32) -> Result<Tile, Error> {
        if x >= self.meta.width || y >= self.meta.height {
            return Err(Error::Invalid("tile coordinates outside world"));
        }
        let column = x as usize;
        let mut r = Reader(&self.data[self.columns[column]..self.columns[column + 1]]);
        let mut remaining = y;
        loop {
            let (tile, len) =
                tiles::read_run(&mut r, &self.data[self.important.clone()], self.type_count)?;
            if remaining < len {
                return Ok(tile);
            }
            remaining -= len;
        }
    }

    /// Publish the exact validated input image, including every opaque field and section.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        let path = path.as_ref();
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut staging = NamedTempFile::new_in(parent)?;
        staging.write_all(&self.data)?;
        staging.as_file().sync_all()?;
        staging.rewind()?;
        let mut buf = [0; 16384];
        for chunk in self.data.chunks(buf.len()) {
            staging.read_exact(&mut buf[..chunk.len()])?;
            if &buf[..chunk.len()] != chunk {
                return Err(Error::Invalid("staged save verification failed"));
            }
        }
        staging.persist(path).map_err(|e| e.error)?;
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;
        Ok(())
    }
}

fn metadata(
    r: &mut Reader<'_>,
    file_version: i32,
    revision: u32,
    favorite: bool,
) -> Result<(Metadata, usize), Error> {
    let name = r.string()?.to_owned();
    let seed = r.string()?.to_owned();
    let generator_version = u64::from_le_bytes(r.take()?);
    let guid_bytes = r.take()?;
    let world_id = r.i32()?;
    r.take::<16>()?; // Four i32 world bounds in pixels.
    let height = u32::try_from(r.i32()?).map_err(|_| Error::Invalid("negative height"))?;
    let width = u32::try_from(r.i32()?).map_err(|_| Error::Invalid("negative width"))?;
    // Local resource limits, above the vanilla large-world dimensions.
    if !(1..=32767).contains(&width)
        || !(1..=32767).contains(&height)
        || u64::from(width) * u64::from(height) > 64_000_000
    {
        return Err(Error::Invalid("world dimensions"));
    }
    let flags_len = r.0.len();
    let game_mode = r.i32()?;
    r.take::<9>()?; // Special-seed booleans through skyblock.
    r.take::<16>()?; // Creation and save timestamps.
    r.byte()?; // Moon type.
    r.take::<68>()?; // 3 tree boundaries, 4 styles, 3 cave boundaries, 4 styles, 3 backgrounds (i32).
    let spawn = (r.i32()?, r.i32()?);
    let surface = f64::from_le_bytes(r.take()?);
    let rock_layer = f64::from_le_bytes(r.take()?);
    let time = f64::from_le_bytes(r.take()?);
    let daytime = r.byte()? != 0;
    Ok((
        Metadata {
            file_version,
            name,
            seed,
            world_id,
            guid_bytes,
            generator_version,
            revision,
            favorite,
            width,
            height,
            game_mode,
            spawn,
            surface,
            rock_layer,
            time,
            daytime,
        },
        flags_len,
    ))
}

fn read_chests(bytes: &[u8]) -> Result<Vec<Chest>, Error> {
    let mut r = Reader(bytes);
    let count = r.i16()?;
    if !(0..=8000).contains(&count) {
        return Err(Error::Invalid("chest count"));
    }
    let mut chests = Vec::new();
    for _ in 0..count {
        let position = (r.i32()?, r.i32()?);
        let name = r.string()?.to_owned();
        let capacity =
            u32::try_from(r.i32()?).map_err(|_| Error::Invalid("negative chest capacity"))?;
        // Bound work before entering a client-independent loop; even empty slots use two bytes.
        if capacity as usize > r.0.len() / 2 {
            return Err(Error::Invalid("chest capacity exceeds section"));
        }
        let mut items = Vec::new();
        for slot in 0..capacity {
            let stack = r.i16()?;
            if stack != 0 {
                items.push(StoredItem {
                    slot,
                    stack: stack.max(1),
                    item_type: r.i32()?,
                    prefix: r.byte()?,
                });
            }
        }
        chests.push(Chest {
            position,
            name,
            capacity,
            items,
        });
    }
    if !r.0.is_empty() {
        return Err(Error::Invalid("chest section length"));
    }
    Ok(chests)
}

fn read_signs(bytes: &[u8]) -> Result<Vec<Sign>, Error> {
    let mut r = Reader(bytes);
    let count = r.i16()?;
    if !(0..=32000).contains(&count) {
        return Err(Error::Invalid("sign count"));
    }
    let mut signs = Vec::new();
    for _ in 0..count {
        let text = r.string()?.to_owned();
        signs.push(Sign {
            text,
            position: (r.i32()?, r.i32()?),
        });
    }
    if !r.0.is_empty() {
        return Err(Error::Invalid("sign section length"));
    }
    Ok(signs)
}
