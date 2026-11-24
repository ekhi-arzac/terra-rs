use super::{Error, World};
use std::collections::BTreeMap;
use terraria_binary::{
    Reader,
    tiles::{Tile, read_run},
};
use terraria_protocol::section::{
    SECTION_HEIGHT, SECTION_WIDTH, SectionObject, TileEntity, TileRun, TileSection, section_tile,
};

pub(crate) fn entities(bytes: &[u8]) -> Result<Vec<TileEntity>, Error> {
    let mut r = Reader(bytes);
    let count =
        usize::try_from(r.i32()?).map_err(|_| Error::Invalid("negative tile entity count"))?;
    if count > r.0.len() / 9 {
        return Err(Error::Invalid("tile entity count exceeds section"));
    }
    let mut by_position = BTreeMap::new();
    for id in 0..count {
        let mut entity = TileEntity::read(&mut r)?;
        entity.id = id as i32;
        by_position.insert([entity.position[1], entity.position[0]], entity);
    }
    if !r.0.is_empty() {
        return Err(Error::Invalid("tile entity section length"));
    }
    Ok(by_position.into_values().collect())
}

impl World {
    /// Convert the saved column runs to a row-major transfer rectangle in one pass per column.
    pub fn section(&self, index: [u16; 2]) -> Result<TileSection, Error> {
        let x = u32::from(index[0]) * u32::from(SECTION_WIDTH);
        let y = u32::from(index[1]) * u32::from(SECTION_HEIGHT);
        if x >= self.meta.width || y >= self.meta.height {
            return Err(Error::Invalid("section outside world"));
        }
        let width = (self.meta.width - x).min(SECTION_WIDTH.into());
        let height = (self.meta.height - y).min(SECTION_HEIGHT.into());
        let mut cells = vec![Tile::default(); (width * height) as usize];
        for dx in 0..width {
            let col = (x + dx) as usize;
            let mut r = Reader(&self.data[self.columns[col]..self.columns[col + 1]]);
            let mut row = 0;
            while row < y + height {
                let (tile, len) =
                    read_run(&mut r, &self.data[self.important.clone()], self.type_count)?;
                let tile = section_tile(tile);
                for yy in row.max(y)..(row + len).min(y + height) {
                    cells[((yy - y) * width + dx) as usize] = tile;
                }
                row += len;
            }
        }
        let mut runs: Vec<TileRun> = Vec::new();
        for tile in cells {
            if let Some(last) = runs.last_mut()
                && last.tile == tile
            {
                last.count += 1;
            } else {
                runs.push(TileRun { tile, count: 1 });
            }
        }
        let contains = |(px, py): (i32, i32)| {
            (x as i32..(x + width) as i32).contains(&px)
                && (y as i32..(y + height) as i32).contains(&py)
        };
        let object = |index: usize, (px, py): (i32, i32), text: &str| SectionObject {
            index: index as i16,
            position: [px as i16, py as i16],
            text: text.to_owned(),
        };
        let mut chests: Vec<_> = self
            .chests
            .iter()
            .enumerate()
            .filter(|(_, c)| contains(c.position))
            .map(|(i, c)| object(i, c.position, &c.name))
            .collect();
        let mut signs: Vec<_> = self
            .signs
            .iter()
            .enumerate()
            .filter(|(_, s)| contains(s.position))
            .map(|(i, s)| object(i, s.position, &s.text))
            .collect();
        let order = |o: &SectionObject| (o.position[1], o.position[0]);
        chests.sort_by_key(order);
        signs.sort_by_key(order);
        let entities = self
            .entities
            .iter()
            .filter(|e| contains((e.position[0].into(), e.position[1].into())))
            .cloned()
            .collect();
        Ok(TileSection {
            origin: [x as i32, y as i32],
            size: [width as u16, height as u16],
            runs,
            chests,
            signs,
            entities,
        })
    }
}
