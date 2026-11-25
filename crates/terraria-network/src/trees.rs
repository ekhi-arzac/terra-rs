//! Resolve a cut from the saved trunk and branch frames, then determine its wood drop.
use crate::terrain::LoadedWorld;
use terraria_world::Tile;

pub(super) struct Harvest {
    pub tiles: Vec<([i16; 2], Tile)>,
    pub wood: i16,
}

fn side_attachment(tile: Tile) -> i16 {
    if tile.block == Some(323) {
        return 0; // Palm trunks occupy one column.
    }
    match tile.frame {
        Some((44, 132 | 154 | 176 | 198 | 220 | 242) | (66, 0 | 22 | 44)) => 1,
        Some((22, 132 | 154 | 176) | (66, 198 | 220 | 242) | (88, 66 | 88 | 110)) => -1,
        _ => 0,
    }
}

pub(super) fn is_tree(tile: Tile) -> bool {
    matches!(tile.block, Some(5 | 323 | 596 | 616 | 634))
}

pub(super) fn harvest(world: &LoadedWorld, [x, y]: [i16; 2], cut: Tile) -> Harvest {
    let species = cut.block;
    let attachment = side_attachment(cut);
    let center = x + attachment;
    let mut ground = None;
    for row in (y..world.data.size[1]).map(|row| world.tile([center, row])) {
        let Some(tile) = row else { break };
        if tile.block != species || side_attachment(tile) != 0 {
            ground = tile.block;
            break;
        }
    }
    let wood = match (species, ground) {
        (Some(323), _) => 2504,
        (Some(634), _) => 5215,
        (_, Some(23 | 661)) => 619,
        (_, Some(199 | 662)) => 911,
        (_, Some(60)) => 620,
        (_, Some(109 | 492)) => 621,
        (_, Some(147)) => 2503,
        (_, Some(70)) => 183,
        _ => 9,
    };
    let mut tiles = vec![([x, y], cut)];
    if attachment == 0 {
        for row in (0..=y).rev() {
            if row != y {
                let Some(tile) = world.tile([x, row]) else {
                    break;
                };
                if tile.block != species || side_attachment(tile) != 0 {
                    break;
                }
                tiles.push(([x, row], tile));
            }
            for dx in [-1, 1] {
                let pos = [x + dx, row];
                if let Some(tile) = world.tile(pos)
                    && tile.block == species
                    && side_attachment(tile) == -dx
                {
                    tiles.push((pos, tile));
                }
            }
        }
    }
    Harvest { tiles, wood }
}
