use super::{Chest, Error, SECTION_COUNT, Tile, World};
use bytes::{BufMut, BytesMut};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Read},
    path::Path,
};
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
        self.with_chest_names(blocks, slots, &BTreeMap::new())
    }

    pub fn with_chest_names(
        &self,
        blocks: &BTreeMap<[i16; 2], Tile>,
        slots: &BTreeMap<(i16, u8), ChestStack>,
        names: &BTreeMap<i16, String>,
    ) -> Result<World, Error> {
        self.with_changes(blocks, slots, names, &BTreeMap::new(), None)
    }

    /// Container positions survive file compaction; network slot numbers are session-local.
    pub fn with_containers(
        &self,
        blocks: &BTreeMap<[i16; 2], Tile>,
        containers: &BTreeMap<(i32, i32), Option<Chest>>,
    ) -> Result<World, Error> {
        self.with_changes(blocks, &BTreeMap::new(), &BTreeMap::new(), containers, None)
    }

    pub fn checkpoint(
        &self,
        blocks: &BTreeMap<[i16; 2], Tile>,
        containers: &BTreeMap<(i32, i32), Option<Chest>>,
        clock: crate::WorldClock,
    ) -> Result<World, Error> {
        self.with_changes(
            blocks,
            &BTreeMap::new(),
            &BTreeMap::new(),
            containers,
            Some(clock),
        )
    }

    fn with_changes(
        &self,
        blocks: &BTreeMap<[i16; 2], Tile>,
        slots: &BTreeMap<(i16, u8), ChestStack>,
        names: &BTreeMap<i16, String>,
        containers: &BTreeMap<(i32, i32), Option<Chest>>,
        clock: Option<crate::WorldClock>,
    ) -> Result<World, Error> {
        if names.iter().any(|(&index, name)| {
            self.chests.get(index as usize).is_none() || name.encode_utf16().count() > 20
        }) {
            return Err(Error::Invalid("chest rename"));
        }
        let mut out = BytesMut::with_capacity(self.data.len());
        out.extend_from_slice(&self.data[..self.offsets[1]]);
        if let Some(clock) = clock {
            if !clock.time.is_finite() {
                return Err(Error::Invalid("non-finite world time"));
            }
            self.join_state()?.clock_offsets.write(&mut out, clock);
        }
        if blocks.is_empty() && slots.is_empty() && names.is_empty() && containers.is_empty() {
            out.extend_from_slice(&self.data[self.offsets[1]..]);
            return self.validate_revision(out);
        }
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
        r.i16()?;
        let count_offset = out.len();
        out.put_i16_le(0);
        let mut count = 0;
        let mut replacements: BTreeMap<_, _> = containers.iter().collect();
        let mut written = 0;
        for (index, chest) in self.chests.iter().enumerate() {
            let header = r.0;
            let position = r.bytes(8)?;
            r.string()?;
            let capacity = r.i32()?;
            if let Some(update) = replacements.remove(&chest.position) {
                r.bytes(chest.capacity as usize * 2 + chest.items.len() * 5)?;
                if let Some(new) = update {
                    write_container(&mut out, chest.position, new, self)?;
                    count += 1;
                }
                continue;
            }
            count += 1;
            if let Some(name) = names.get(&(index as i16)) {
                out.extend_from_slice(position);
                terraria_binary::put_string(&mut out, name);
                out.put_i32_le(capacity);
            } else {
                out.extend_from_slice(&header[..header.len() - r.0.len()]);
            }
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
        for (&pos, update) in replacements {
            if let Some(new) = update {
                write_container(&mut out, pos, new, self)?;
                count += 1;
            }
        }
        if count > 8000 {
            return Err(Error::Invalid("chest count"));
        }
        out[count_offset..count_offset + 2].copy_from_slice(&(count as i16).to_le_bytes());
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
        self.validate_revision(out)
    }

    fn validate_revision(&self, mut out: BytesMut) -> Result<World, Error> {
        out[12..16].copy_from_slice(&self.meta.revision.wrapping_add(1).to_le_bytes());
        World::parse(out.into())
    }

    fn matches_file(&self, file: &mut File) -> io::Result<bool> {
        if file.metadata()?.len() != self.data.len() as u64 {
            return Ok(false);
        }
        let mut buf = [0; 16384];
        for chunk in self.data.chunks(buf.len()) {
            file.read_exact(&mut buf[..chunk.len()])?;
            if &buf[..chunk.len()] != chunk {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Keep up to three preceding images before publishing a new world revision.
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
        if !self.matches_file(&mut current)? {
            return Err(Error::Invalid(
                "world file changed since the last load or save",
            ));
        }
        let mut backup = path.as_os_str().to_owned();
        backup.push(".terra-rs.bak");
        let backup = Path::new(&backup);
        match File::open(backup) {
            Ok(mut previous) => {
                if self.matches_file(&mut previous)? {
                    return edited.save(path);
                }
                drop(previous);
                // Alternating slots retain the two preceding backups. A retry after
                // publishing the backup recognizes the same bytes and skips rotation.
                let mut older = backup.as_os_str().to_owned();
                older.push(if self.meta.revision.is_multiple_of(2) {
                    ".even"
                } else {
                    ".odd"
                });
                fs::rename(backup, older)?;
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
        self.save(backup)?;
        edited.save(path)
    }
}

fn write_container(
    out: &mut BytesMut,
    pos: (i32, i32),
    chest: &Chest,
    world: &World,
) -> Result<(), Error> {
    if pos != chest.position
        || !(0..world.meta.width as i32).contains(&pos.0)
        || !(0..world.meta.height as i32).contains(&pos.1)
        || chest.capacity > 256
    {
        return Err(Error::Invalid("edited chest bounds"));
    }
    out.put_i32_le(pos.0);
    out.put_i32_le(pos.1);
    terraria_binary::put_string(out, &chest.name);
    out.put_i32_le(chest.capacity as i32);
    let mut items = chest.items.iter().peekable();
    for slot in 0..chest.capacity {
        if let Some(item) = items.next_if(|item| item.slot == slot) {
            if item.stack <= 0 || item.item_type <= 0 {
                return Err(Error::Invalid("edited chest item"));
            }
            out.put_i16_le(item.stack);
            out.put_i32_le(item.item_type);
            out.put_u8(item.prefix);
        } else {
            out.put_i16_le(0);
        }
    }
    if items.next().is_some() {
        return Err(Error::Invalid("edited chest slot order"));
    }
    Ok(())
}
