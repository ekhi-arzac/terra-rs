use super::{Error, SECTION_COUNT, Tile, World};
use bytes::{BufMut, BytesMut};
use std::{collections::BTreeMap, io::Read, path::Path};
use terraria_binary::{Reader, tiles};

#[derive(Clone, Copy, Debug)]
pub struct ChestStack {
    pub stack: i16,
    pub prefix: u8,
    pub item: i16,
}

impl World {
    /// Build a new validated image of the original world version and preserve opaque sections verbatim.
    pub fn with_edits(
        &self,
        blocks: &BTreeMap<[i16; 2], Tile>,
        slots: &BTreeMap<(i16, u8), ChestStack>,
    ) -> Result<World, Error> {
        let mut out = BytesMut::from(&self.data[..self.offsets[1]]);
        let mut offsets = self.offsets;
        for (x, column) in self.columns.windows(2).enumerate() {
            let raw = &self.data[column[0]..column[1]];
            let x = i16::try_from(x).map_err(|_| Error::Invalid("save column"))?;
            let mut edits = blocks.range([x, 0]..=[x, i16::MAX]).peekable();
            if edits.peek().is_none() {
                out.extend_from_slice(raw);
                continue;
            }
            let mut r = Reader(raw);
            let mut row = 0;
            let important = &self.data[self.important.clone()];
            while !r.0.is_empty() {
                let before = r.0;
                let (tile, count) = tiles::read_run(&mut r, important, self.type_count)?;
                let end = row + count;
                if edits.peek().is_some_and(|(pos, _)| (pos[1] as u32) < end) {
                    while let Some((&[_, y], changed)) =
                        edits.next_if(|(pos, _)| (pos[1] as u32) < end)
                    {
                        let y = y as u32;
                        if y > row {
                            tiles::write_run(
                                &mut out,
                                tile,
                                (y - row) as u16,
                                important,
                                self.type_count,
                            )?;
                        }
                        tiles::write_run(&mut out, *changed, 1, important, self.type_count)?;
                        row = y + 1;
                    }
                    if row < end {
                        tiles::write_run(
                            &mut out,
                            tile,
                            (end - row) as u16,
                            important,
                            self.type_count,
                        )?;
                    }
                } else {
                    out.extend_from_slice(&before[..before.len() - r.0.len()]);
                }
                row = end;
            }
        }
        if blocks.keys().any(|&[x, y]| {
            x < 0 || y < 0 || x as u32 >= self.meta.width || y as u32 >= self.meta.height
        }) {
            return Err(Error::Invalid("save tile coordinate"));
        }
        offsets[2] = out.len();
        let mut r = Reader(&self.data[self.offsets[2]..self.offsets[3]]);
        out.put_i16_le(r.i16()?);
        let mut written = 0;
        for (index, chest) in self.chests.iter().enumerate() {
            let header = r.0;
            r.bytes(8)?;
            r.string()?;
            r.i32()?;
            out.extend_from_slice(&header[..header.len() - r.0.len()]);
            for slot in 0..chest.capacity {
                let before = r.0;
                if r.i16()? != 0 {
                    r.bytes(5)?;
                }
                let changed = u8::try_from(slot)
                    .ok()
                    .and_then(|slot| slots.get(&(index as i16, slot)));
                if let Some(&ChestStack {
                    stack,
                    prefix,
                    item,
                }) = changed
                {
                    if stack < 0 || !(0..6196).contains(&item) || (stack == 0) != (item == 0) {
                        return Err(Error::Invalid("save chest item"));
                    }
                    out.put_i16_le(stack);
                    if stack > 0 {
                        out.put_i32_le(item.into());
                        out.put_u8(prefix);
                    }
                    written += 1;
                } else {
                    out.extend_from_slice(&before[..before.len() - r.0.len()]);
                }
            }
        }
        if written != slots.len() {
            return Err(Error::Invalid("save chest slot"));
        }
        let tail_start = out.len();
        for (i, offset) in offsets.iter_mut().enumerate().take(SECTION_COUNT).skip(3) {
            *offset = tail_start + (self.offsets[i] - self.offsets[3]);
        }
        out.extend_from_slice(&self.data[self.offsets[3]..]);
        for (field, &offset) in out[26..26 + 4 * SECTION_COUNT]
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(&offsets)
        {
            field.copy_from_slice(
                &i32::try_from(offset)
                    .map_err(|_| Error::Invalid("save file size"))?
                    .to_le_bytes(),
            );
        }
        out[12..16].copy_from_slice(&self.meta.revision.wrapping_add(1).to_le_bytes());
        World::parse(out.to_vec())
    }

    /// Keep the loaded image as a backup before publishing the edited image.
    pub fn publish_edits(&self, edited: &World, path: &Path) -> Result<(), Error> {
        let mut current = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;
        current.try_lock().map_err(std::io::Error::from)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let fd = current.metadata()?;
            let at_path = std::fs::metadata(path)?;
            if (fd.dev(), fd.ino()) != (at_path.dev(), at_path.ino()) {
                return Err(Error::Invalid("world file replaced during save"));
            }
        }
        if current.metadata()?.len() != self.data.len() as u64 {
            return Err(Error::Invalid("world file changed since startup"));
        }
        let mut buf = [0; 16384];
        for chunk in self.data.chunks(buf.len()) {
            current.read_exact(&mut buf[..chunk.len()])?;
            if &buf[..chunk.len()] != chunk {
                return Err(Error::Invalid("world file changed since startup"));
            }
        }
        let mut backup = path.as_os_str().to_owned();
        backup.push(".terra-rs.bak");
        self.save(Path::new(&backup))?;
        edited.save(path)
    }
}
