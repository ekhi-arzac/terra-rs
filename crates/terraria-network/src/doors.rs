//! Shared terrain state for hinged wooden doors.
use crate::stations::Cells;
use crate::terrain::{LoadedWorld, is_ground_cover, is_solid_floor};
use terraria_world::{Liquid, Tile};

pub(super) const CLOSED: u16 = 10;
const OPEN: u16 = 11;

#[derive(Clone, Copy)]
pub(super) struct Door {
    pub hinge: [i16; 2],
    style: i16,
    /// None for a closed door, or the direction the open door extends from its hinge.
    swing: Option<bool>,
}

impl Door {
    pub fn placement(anchor: [i16; 2], style: i16, origin_row: u8) -> Option<Self> {
        if origin_row > 2 {
            return None;
        }
        Some(Self {
            hinge: [anchor[0], anchor[1].checked_sub(i16::from(origin_row))?],
            style,
            swing: None,
        })
    }

    pub fn locate(world: &LoadedWorld, hit: [i16; 2], tile: Tile) -> Option<Self> {
        let (fx, fy) = tile.frame?;
        if fx < 0
            || !(0..36 * 54).contains(&fy)
            || fx % 18 != 0
            || fy % 18 != 0
            || (tile.block == Some(OPEN) && fx / 72 > (i16::MAX - 54) / 72)
        {
            return None;
        }
        let (hinge_x, style_group, swing) = match tile.block? {
            CLOSED => (hit[0], fx / 54, None),
            OPEN => {
                let frame = fx % 72;
                let right = frame < 36;
                let hinge_x = match frame {
                    18 => hit[0].checked_sub(1)?,
                    36 => hit[0].checked_add(1)?,
                    _ => hit[0],
                };
                (hinge_x, fx / 72, Some(right))
            }
            _ => return None,
        };
        let door = Self {
            hinge: [hinge_x, hit[1].checked_sub(fy % 54 / 18)?],
            style: style_group * 36 + fy / 54,
            swing,
        };
        // Validate the whole object before editing any cell.
        for (pos, actual) in door.footprint(world)? {
            let (expected, (x, y)) = door.frame_at(pos);
            let (ax, ay) = actual.frame?;
            let same_column = if expected == CLOSED {
                ax >= 0 && ax % 18 == 0 && ax / 54 == x / 54
            } else {
                ax == x
            };
            if actual.block != Some(expected) || !same_column || ay != y {
                return None;
            }
        }
        Some(door)
    }

    pub fn item(self) -> Option<i16> {
        match self.style {
            0 => Some(25),   // Wood
            1 => Some(650),  // Ebonwood
            2 => Some(651),  // Rich Mahogany
            3 => Some(652),  // Pearlwood
            10 => Some(912), // Shadewood
            _ => None,
        }
    }

    fn frame_at(self, pos: [i16; 2]) -> (u16, (i16, i16)) {
        let row = pos[1] - self.hinge[1];
        let y = (self.style % 36) * 54 + row * 18;
        match self.swing {
            None => (CLOSED, ((self.style / 36) * 54, y)),
            Some(right) => {
                let left = self.hinge[0] - i16::from(!right);
                let x = (self.style / 36) * 72 + 36 * i16::from(!right) + (pos[0] - left) * 18;
                (OPEN, (x, y))
            }
        }
    }

    fn wing(self, world: &LoadedWorld, right: bool) -> Option<Cells> {
        let x = self.hinge[0].checked_add(if right { 1 } else { -1 })?;
        self.column(world, x)
    }

    fn column(self, world: &LoadedWorld, x: i16) -> Option<Cells> {
        (0..3)
            .map(|row| {
                let pos = [x, self.hinge[1].checked_add(row)?];
                Some((pos, world.tile(pos)?))
            })
            .collect()
    }

    pub fn footprint(self, world: &LoadedWorld) -> Option<Cells> {
        let mut cells = self.column(world, self.hinge[0])?;
        if let Some(right) = self.swing {
            cells.extend(self.wing(world, right)?);
        }
        Some(cells)
    }

    pub fn place(self, world: &LoadedWorld, before: &[([i16; 2], Tile)]) -> Option<Cells> {
        self.item()?;
        for dy in [-1, 3] {
            let anchor = [self.hinge[0], self.hinge[1].checked_add(dy)?];
            let tile = world.tile(anchor)?;
            if tile.block == Some(19) || !is_solid_floor(tile) {
                return None;
            }
        }
        if !before
            .iter()
            .all(|&(_, tile)| replaceable(tile) && !matches!(tile.liquid, Some((Liquid::Lava, _))))
        {
            return None;
        }
        Some(
            before
                .iter()
                .map(|&(pos, tile)| (pos, self.paint(pos, tile, tile)))
                .collect(),
        )
    }

    /// Return the affected tiles and whether their state changes. A rejected
    /// prediction sends the original footprint back to its requester.
    pub fn toggle(
        self,
        world: &LoadedWorld,
        open: bool,
        right: bool,
        blocked: bool,
    ) -> Option<(Cells, bool)> {
        let mut cells = self.footprint(world)?;
        if !open {
            if self.swing.is_none() || blocked || self.item().is_none() {
                return Some((cells, false));
            }
            let closed = Self {
                swing: None,
                ..self
            };
            for (pos, tile) in &mut cells {
                *tile = if pos[0] == self.hinge[0] {
                    closed.paint(*pos, *tile, *tile)
                } else {
                    LoadedWorld::with_block(*tile, None)
                };
            }
            return Some((cells, true));
        }
        if let Some(current) = self.swing {
            if current != right {
                cells.extend(self.wing(world, right)?);
            }
            return Some((cells, false));
        }
        let wing = self.wing(world, right)?;
        let allowed = self.item().is_some() && wing.iter().all(|&(_, tile)| replaceable(tile));
        cells.extend(wing);
        if allowed {
            let hinge = [cells[0].1, cells[1].1, cells[2].1];
            let opened = Self {
                swing: Some(right),
                ..self
            };
            for (pos, tile) in &mut cells {
                *tile = opened.paint(*pos, *tile, hinge[(pos[1] - self.hinge[1]) as usize]);
            }
        }
        Some((cells, allowed))
    }

    fn paint(self, pos: [i16; 2], tile: Tile, source: Tile) -> Tile {
        let (block, frame) = self.frame_at(pos);
        let mut result = LoadedWorld::with_block(tile, Some(block));
        result.frame = Some(frame);
        result.block_paint = source.block_paint;
        result.flags[3] |= source.flags[3] & 0x0a;
        result
    }

    pub fn removal(self, world: &LoadedWorld) -> Option<(Cells, i16)> {
        let item = self.item()?;
        let mut cells = self.footprint(world)?;
        for (_, tile) in &mut cells {
            *tile = LoadedWorld::with_block(*tile, None);
        }
        Some((cells, item))
    }

    pub fn supported_by(self, pos: [i16; 2]) -> bool {
        pos[0] == self.hinge[0] && matches!(i32::from(pos[1]) - i32::from(self.hinge[1]), -1 | 3)
    }

    pub fn overlaps_hinge(self, [px, py]: [f32; 2]) -> bool {
        let [x, y] = self.hinge.map(|n| f32::from(n) * 16.);
        // Stock player hitbox. Movement supplies its top-left pixel position.
        px < x + 16. && px + 20. > x && py < y + 48. && py + 42. > y
    }
}

fn replaceable(tile: Tile) -> bool {
    tile.block.is_none_or(is_ground_cover)
}
