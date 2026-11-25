//! Frame layouts for the early crafting stations handled by the roster.
use crate::terrain::{LoadedWorld, is_ground_cover, is_solid_floor};
use terraria_world::{Liquid, Tile};

#[derive(Clone, Copy)]
pub(super) enum Station {
    Anvil,
    Furnace,
    WorkBench,
}

pub(super) type Cells = Vec<([i16; 2], Tile)>;

// Item IDs by the horizontal work-bench frame style in 1.4.6.
const WORK_BENCH_ITEMS: [i16; 65] = [
    36, 635, 636, 637, 811, 812, 813, 814, 815, 916, 1145, 1398, 1401, 1404, 1461, 1511, 1795,
    1817, 2229, 2251, 2252, 2253, 2534, 673, 2631, 2632, 2633, 2826, 3156, 3158, 3157, 3909, 3910,
    3949, 3975, 4163, 4184, 4205, 4226, 4315, 4584, 5166, 5187, 5208, 5566, 5619, 5707, 5730, 5775,
    5796, 5817, 5838, 5856, 5877, 5896, 5917, 5951, 5973, 5994, 6017, 6040, 6063, 6086, 6108, 6130,
];

impl Station {
    pub fn for_block(block: u16) -> Option<Self> {
        match block {
            16 => Some(Self::Anvil),
            17 => Some(Self::Furnace),
            18 => Some(Self::WorkBench),
            _ => None,
        }
    }

    fn block(self) -> u16 {
        match self {
            Self::Anvil => 16,
            Self::Furnace => 17,
            Self::WorkBench => 18,
        }
    }

    fn size(self) -> [i16; 2] {
        match self {
            Self::Furnace => [3, 2],
            _ => [2, 1],
        }
    }

    fn frame(self, style: i16, column: i16, row: i16) -> (i16, i16) {
        ((self.size()[0] * style + column) * 18, row * 18)
    }

    pub fn drop_item(self, style: i16) -> Option<i16> {
        match (self, style) {
            (Self::Anvil, 0) => Some(35),
            (Self::Anvil, 1) => Some(716),
            (Self::Furnace, 0) => Some(33),
            (Self::WorkBench, index) => WORK_BENCH_ITEMS.get(usize::try_from(index).ok()?).copied(),
            _ => None,
        }
    }

    pub fn footprint(self, world: &LoadedWorld, anchor: [i16; 2]) -> Option<Cells> {
        let origin = match self {
            Self::Furnace => [anchor[0].checked_sub(1)?, anchor[1].checked_sub(1)?],
            _ => anchor,
        };
        self.cells(world, origin)
    }

    fn cells(self, world: &LoadedWorld, origin: [i16; 2]) -> Option<Cells> {
        let [width, height] = self.size();
        let mut result = Vec::with_capacity((width * height) as usize);
        for dx in 0..width {
            for dy in 0..height {
                let pos = [origin[0].checked_add(dx)?, origin[1].checked_add(dy)?];
                result.push((pos, world.tile(pos)?));
            }
        }
        Some(result)
    }

    pub fn placement(
        self,
        world: &LoadedWorld,
        style: i16,
        existing: &[([i16; 2], Tile)],
    ) -> Option<Cells> {
        self.drop_item(style)?;
        let origin = existing.first()?.0;
        let floor = origin[1].checked_add(self.size()[1])?;
        for dx in 0..self.size()[0] {
            if !is_solid_floor(world.tile([origin[0].checked_add(dx)?, floor])?) {
                return None;
            }
        }
        let mut placed = Vec::with_capacity(existing.len());
        for &(pos, before) in existing {
            if before.block.is_some_and(|block| !is_ground_cover(block))
                || matches!(before.liquid, Some((Liquid::Lava, _)))
            {
                return None;
            }
            let mut after = LoadedWorld::with_block(before, Some(self.block()));
            after.frame = Some(self.frame(style, pos[0] - origin[0], pos[1] - origin[1]));
            placed.push((pos, after));
        }
        Some(placed)
    }

    /// Resolve a station from any of its tiles. Require the full matching frame sequence.
    pub fn removal(world: &LoadedWorld, hit: [i16; 2], tile: Tile) -> Option<(Cells, i16)> {
        let station = Self::for_block(tile.block?)?;
        let (frame_x, frame_y) = tile.frame?;
        let [width, height] = station.size();
        if frame_x < 0
            || frame_x % 18 != 0
            || !(0..height * 18).contains(&frame_y)
            || frame_y % 18 != 0
        {
            return None;
        }
        let style = frame_x / (width * 18);
        let item = station.drop_item(style)?;
        let origin = [
            hit[0].checked_sub(frame_x / 18 % width)?,
            hit[1].checked_sub(frame_y / 18)?,
        ];
        let mut edits = station.cells(world, origin)?;
        for (pos, current) in &mut edits {
            if current.block != tile.block
                || current.frame
                    != Some(station.frame(style, pos[0] - origin[0], pos[1] - origin[1]))
            {
                return None;
            }
            *current = LoadedWorld::with_block(*current, None);
        }
        Some((edits, item))
    }

    pub fn on_support(world: &LoadedWorld, support: [i16; 2]) -> Option<(Cells, i16)> {
        let above = [support[0], support[1].checked_sub(1)?];
        let tile = world.tile(above)?;
        let station = Self::for_block(tile.block?)?;
        if tile.frame?.1 != (station.size()[1] - 1) * 18 {
            return None;
        }
        Self::removal(world, above, tile)
    }
}
