//! Raw-DEFLATE tile rectangles used by release 326. Tiles are traversed in row order.
use crate::wire::{Error, Reader};
use bytes::{BufMut, BytesMut};
use flate2::{Compression, Decompress, FlushDecompress, Status, write::DeflateEncoder};
use std::io::Write;
use terraria_binary::tiles::{Tile, read_run};

pub const FRAME_IMPORTANT: &[u8; 95] = include_bytes!("frame-important-326.bin");
pub const TILE_TYPES: usize = 754;
pub const SECTION_WIDTH: u16 = 200;
pub const SECTION_HEIGHT: u16 = 150;
const MAX_INFLATED: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileRun {
    pub tile: Tile,
    pub count: u16,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionObject {
    pub index: i16,
    pub position: [i16; 2],
    pub text: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileSection {
    pub origin: [i32; 2],
    pub size: [u16; 2],
    pub runs: Vec<TileRun>,
    pub chests: Vec<SectionObject>,
    pub signs: Vec<SectionObject>,
    pub entities: Vec<TileEntity>,
}

/// Remove representation bits (header continuation, field presence and RLE) while preserving tile state.
pub fn section_tile(mut tile: Tile) -> Tile {
    tile.flags = [
        0,
        tile.flags[1] & 0x7e,
        tile.flags[2] & 0x26,
        tile.flags[3] & 0x1e,
    ];
    if tile.block == Some(144)
        && let Some((x, _)) = tile.frame
    {
        tile.frame = Some((x, 0));
    }
    tile
}

fn area(size: [u16; 2]) -> Result<usize, Error> {
    if size[0] == 0 || size[0] > SECTION_WIDTH || size[1] == 0 || size[1] > SECTION_HEIGHT {
        return Err(Error::Invalid("section dimensions"));
    }
    Ok(usize::from(size[0]) * usize::from(size[1]))
}

fn inflate(data: &[u8]) -> Result<Vec<u8>, Error> {
    let mut stream = Decompress::new(false);
    let mut out = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        let input = stream.total_in() as usize;
        let produced = stream.total_out() as usize;
        let status = stream
            .decompress(&data[input..], &mut chunk, FlushDecompress::None)
            .map_err(|_| Error::Invalid("section DEFLATE stream"))?;
        let len = stream.total_out() as usize - produced;
        if produced + len > MAX_INFLATED {
            return Err(Error::Invalid("inflated section size"));
        }
        out.extend_from_slice(&chunk[..len]);
        if status == Status::StreamEnd {
            if stream.total_in() as usize != data.len() {
                return Err(Error::Invalid("trailing compressed data"));
            }
            return Ok(out);
        }
        if len == 0 && stream.total_in() as usize == input {
            return Err(Error::Invalid("incomplete section DEFLATE stream"));
        }
    }
}

impl TileSection {
    pub(crate) fn read(data: &[u8]) -> Result<Self, Error> {
        let raw = inflate(data)?;
        let mut r = Reader(&raw);
        let origin = r.i32s()?;
        let size = r.i16s::<2>()?.map(|v| v as u16);
        let mut left = area(size)?;
        let mut runs: Vec<TileRun> = Vec::new();
        while left > 0 {
            let (tile, count) = read_run(&mut r, FRAME_IMPORTANT, TILE_TYPES)?;
            left = left
                .checked_sub(count as usize)
                .ok_or(Error::Invalid("run outside section"))?;
            let tile = section_tile(tile);
            if let Some(last) = runs.last_mut()
                && last.tile == tile
            {
                last.count += count as u16;
            } else {
                runs.push(TileRun {
                    tile,
                    count: count as u16,
                });
            }
        }
        let chests = read_objects(&mut r, 8000)?;
        let signs = read_objects(&mut r, 32000)?;
        let count =
            usize::try_from(r.i16()?).map_err(|_| Error::Invalid("section tile entity count"))?;
        if count > area(size)? {
            return Err(Error::Invalid("section tile entity count"));
        }
        let entities = (0..count)
            .map(|_| TileEntity::read(&mut r))
            .collect::<Result<_, _>>()?;
        if !r.0.is_empty() {
            return Err(Error::Trailing(r.0.len()));
        }
        Ok(Self {
            origin,
            size,
            runs,
            chests,
            signs,
            entities,
        })
    }
    pub(crate) fn write(&self, out: &mut BytesMut) -> Result<(), Error> {
        let mut left = area(self.size)?;
        let mut raw = BytesMut::new();
        for v in self.origin {
            raw.put_i32_le(v)
        }
        for v in self.size {
            raw.put_u16_le(v)
        }
        for run in &self.runs {
            if run.count == 0 {
                return Err(Error::Invalid("empty section run"));
            }
            left = left
                .checked_sub(usize::from(run.count))
                .ok_or(Error::Invalid("run outside section"))?;
            write_run(&mut raw, run)?;
        }
        if left != 0 {
            return Err(Error::Invalid("section tile count"));
        }
        write_objects(&mut raw, &self.chests, 8000)?;
        write_objects(&mut raw, &self.signs, 32000)?;
        if self.entities.len() > area(self.size)? {
            return Err(Error::Invalid("section tile entity count"));
        }
        raw.put_i16_le(self.entities.len() as i16);
        for entity in &self.entities {
            raw.put_u8(entity.kind);
            raw.put_i32_le(entity.id);
            for v in entity.position {
                raw.put_i16_le(v)
            }
            raw.extend_from_slice(&entity.extra);
            if raw.len() > MAX_INFLATED {
                return Err(Error::Invalid("inflated section size"));
            }
        }
        let mut deflate = DeflateEncoder::new(Vec::new(), Compression::fast());
        deflate
            .write_all(&raw)
            .map_err(|_| Error::Invalid("section compression"))?;
        out.extend_from_slice(
            &deflate
                .finish()
                .map_err(|_| Error::Invalid("section compression"))?,
        );
        Ok(())
    }
}

fn read_objects(r: &mut Reader<'_>, limit: i16) -> Result<Vec<SectionObject>, Error> {
    let count = r.i16()?;
    if !(0..=limit).contains(&count) {
        return Err(Error::Invalid("section object count"));
    }
    (0..count)
        .map(|_| {
            let index = r.i16()?;
            if !(0..limit).contains(&index) {
                return Err(Error::Invalid("section object index"));
            }
            Ok(SectionObject {
                index,
                position: r.i16s()?,
                text: r.string()?.to_owned(),
            })
        })
        .collect()
}

fn write_objects(out: &mut BytesMut, objects: &[SectionObject], limit: i16) -> Result<(), Error> {
    if objects.len() > limit as usize {
        return Err(Error::Invalid("section object count"));
    }
    out.put_i16_le(objects.len() as i16);
    for obj in objects {
        if !(0..limit).contains(&obj.index) {
            return Err(Error::Invalid("section object index"));
        }
        if obj.text.len().saturating_add(out.len()) > MAX_INFLATED - 16 {
            return Err(Error::Invalid("section text size"));
        }
        out.put_i16_le(obj.index);
        for v in obj.position {
            out.put_i16_le(v)
        }
        crate::wire::put_string_unchecked(out, &obj.text);
    }
    Ok(())
}

fn write_run(out: &mut BytesMut, run: &TileRun) -> Result<(), Error> {
    Ok(terraria_binary::tiles::write_run(
        out,
        section_tile(run.tile),
        run.count,
        FRAME_IMPORTANT,
        TILE_TYPES,
    )?)
}

/// A release-326 tile entity with its validated type-specific payload. Disk and
/// full section transfers both use the non-light form, including the entity ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileEntity {
    kind: u8,
    pub id: i32,
    pub position: [i16; 2],
    extra: Vec<u8>,
}
impl TileEntity {
    pub fn kind(&self) -> u8 {
        self.kind
    }
    pub fn read(r: &mut terraria_binary::Reader<'_>) -> Result<Self, terraria_binary::Error> {
        let kind = r.byte()?;
        let id = r.i32()?;
        let position = r.i16s()?;
        let start = r.0;
        let len = match kind {
            0 | 2 | 9 | 10 => 2,
            1 | 4 | 6 | 8 => 5,
            3 => {
                let [equip, dyes, _pose, extra] = r.take::<4>()?;
                5 * (equip.count_ones() + dyes.count_ones() + (extra & 7).count_ones()) as usize
            }
            5 => 5 * (r.byte()? & 15).count_ones() as usize,
            7 => 0,
            _ => {
                return Err(terraria_binary::Error::Tile(
                    "unrecognized tile entity kind",
                ));
            }
        };
        r.bytes(len)?;
        Ok(Self {
            kind,
            id,
            position,
            extra: start[..start.len() - r.0.len()].to_vec(),
        })
    }
}
