use std::fs;
use tempfile::tempdir;
use terraria_world::{Error, Liquid, WORLD_VERSION, World};

// A small structural fixture for the interpreted sections. The opaque sections
// carry distinct markers to catch accidental rewriting. It is not a playable world.
fn fixture() -> Vec<u8> {
    let mut data = WORLD_VERSION.to_le_bytes().to_vec();
    data.extend(b"relogic\x02");
    data.extend(12u32.to_le_bytes());
    data.extend(1u64.to_le_bytes());
    data.extend(11i16.to_le_bytes());
    data.extend([0; 44]);
    data.extend(300i16.to_le_bytes());
    let mut important = [0; 38];
    important[256 / 8] = 1;
    data.extend(important);
    fn section(data: &mut [u8], index: usize) {
        let len = (data.len() as i32).to_le_bytes();
        let p = 26 + index * 4;
        data[p..p + 4].copy_from_slice(&len);
    }
    section(&mut data, 0);
    data.extend(b"\x03Oak\x0212");
    data.extend(42u64.to_le_bytes());
    data.extend([9; 16]);
    data.extend(75i32.to_le_bytes());
    data.extend([0; 16]);
    data.extend(3i32.to_le_bytes());
    data.extend(2i32.to_le_bytes());
    data.extend(1i32.to_le_bytes());
    data.extend([0; 9 + 16 + 1 + 68]);
    data.extend(1i32.to_le_bytes());
    data.extend(2i32.to_le_bytes());
    for value in [1.5f64, 2.0, 12.5] {
        data.extend(value.to_le_bytes());
    }
    data.extend([1, 0x6c]);
    section(&mut data, 1);
    // Column 0: block 256 with frame, paints, wall 262, shimmer and chained flags, repeated 3x.
    data.extend([
        0x6f, 0x17, 0xff, 0x1e, 0, 1, 0xfe, 0xff, 4, 0, 9, 6, 8, 127, 1, 2,
    ]);
    // Column 1: air repeated 3x using i16 RLE.
    data.extend([128, 2, 0]);
    section(&mut data, 2);
    data.extend(1i16.to_le_bytes());
    data.extend(0i32.to_le_bytes());
    data.extend(1i32.to_le_bytes());
    data.extend(b"\x01C");
    data.extend(2i32.to_le_bytes());
    data.extend(0i16.to_le_bytes());
    data.extend(42i16.to_le_bytes());
    data.extend(259i32.to_le_bytes());
    data.push(3);
    section(&mut data, 3);
    data.extend(1i16.to_le_bytes());
    data.extend(b"\x02Hi");
    data.extend(1i32.to_le_bytes());
    data.extend(2i32.to_le_bytes());
    for i in 4..10 {
        section(&mut data, i);
        if i == 4 {
            data.extend([0; 6]);
        } else if i == 5 {
            data.extend(0i32.to_le_bytes());
        } else {
            data.extend([0xaa, i as u8]);
        }
    }
    section(&mut data, 10);
    data.extend(b"\x01\x03Oak");
    data.extend(75i32.to_le_bytes());
    data
}

fn offset(data: &[u8], idx: usize) -> usize {
    let start = 26 + idx * 4;
    i32::from_le_bytes(data[start..start + 4].try_into().unwrap()) as usize
}

#[test]
fn indexes_tiles_containers_and_flags() {
    let world = World::parse(fixture()).unwrap();
    let m = world.metadata();
    assert_eq!(
        (m.name.as_str(), m.width, m.height, m.spawn),
        ("Oak", 2, 3, (1, 2))
    );
    assert!(m.favorite && m.daytime);
    assert_eq!(m.time, 12.5);
    for y in 0..3 {
        let t = world.tile(0, y).unwrap();
        assert_eq!(t.block, Some(256));
        assert_eq!(t.frame, Some((-2, 4)));
        assert_eq!(t.wall, 262);
        assert_eq!(
            (t.block_paint, t.wall_paint, t.liquid),
            (9, 8, Some((Liquid::Shimmer, 127)))
        );
        assert_eq!(t.flags, [0x6f, 0x17, 0xff, 0x1e]);
        assert_eq!(world.tile(1, y).unwrap().block, None);
    }
    assert!(world.tile(2, 0).is_err());
    assert!(world.tile(1, 3).is_err());
    let c = &world.chests()[0];
    let i = &c.items[0];
    assert_eq!(
        (
            c.name.as_str(),
            c.capacity,
            i.slot,
            i.stack,
            i.item_type,
            i.prefix
        ),
        ("C", 2, 1, 42, 259, 3)
    );
    assert_eq!(world.signs()[0].text, "Hi");
}

#[test]
fn copy_replaces_atomically_and_preserves_opaque_data() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("existing.wld");
    fs::write(&path, b"previous destination").unwrap();
    let bytes = fixture();
    let world = World::parse(bytes.clone()).unwrap();
    world.save(&path).unwrap();
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(World::load(&path).unwrap().metadata().revision, 12);
    // Failed publication must also clean the same-directory staging file.
    let existing_dir = dir.path().join("directory");
    fs::create_dir(&existing_dir).unwrap();
    assert!(world.save(&existing_dir).is_err());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn corrupt_headers_sections_and_runs_are_rejected() {
    let original = fixture();
    for end in 0..original.len() {
        assert!(World::parse(original[..end].to_vec()).is_err());
    }
    let mutate = |at: usize, part: &[u8]| {
        let mut data = original.clone();
        data[at..at + part.len()].copy_from_slice(part);
        assert!(World::parse(data).is_err(), "offset {at}");
    };
    mutate(0, &[0]);
    mutate(11, &[1]);
    mutate(24, &[10]);
    mutate(26, &(-1i32).to_le_bytes());
    mutate(30, &(i32::MAX).to_le_bytes());
    let tiles = offset(&original, 1);
    mutate(tiles + 4, &[255, 255]); // no matching frame table entry
    mutate(tiles + 15, &[3]); // crosses the first column
    mutate(tiles + 17, &[255, 255]); // negative repeat count
    mutate(offset(&original, 2), &[255, 255]);
    mutate(offset(&original, 2) + 12, &(i32::MAX).to_le_bytes()); // capacity
    mutate(offset(&original, 3), &[1, 127]); // excessive sign count
    mutate(original.len() - 4, &[76]); // footer world ID
    assert!(matches!(
        World::parse(vec![1, 0, 0, 0]),
        Err(Error::Version(1))
    ));
}

#[test]
fn extended_disk_header_matches_recorded_world_and_exposes_row_major_tiles() {
    let w = World::parse(include_bytes!("../../../docs/fixtures/entry-326.wld").to_vec()).unwrap();
    let p = terraria_protocol::Packet::WorldData(Box::new(w.world_data().unwrap()));
    assert_eq!(
        p.encode().unwrap().as_ref(),
        include_bytes!("../../../docs/fixtures/entry-world-data-326.bin")
    );
    let s = w.section([10, 1]).unwrap();
    let cells: Vec<_> = s
        .runs
        .iter()
        .flat_map(|r| std::iter::repeat_n(r.tile, r.count as usize))
        .collect();
    for (dx, dy) in [(95, 82), (96, 83), (97, 82), (99, 100)] {
        assert_eq!(
            cells[dy * 200 + dx],
            terraria_protocol::section::section_tile(
                w.tile(2000 + dx as u32, 150 + dy as u32).unwrap()
            )
        );
    }
    assert_eq!(s.chests[0].position, [2095, 232]);
    assert_eq!(w.chests()[0].items[0].stack, 1);
    assert!(w.section([21, 0]).is_err());
    assert!(World::parse(fixture()).unwrap().world_data().is_err());
}

#[test]
fn progressed_header_populates_banner_counts_and_reloaded_tower_strength() {
    let mut bytes = include_bytes!("../../../docs/fixtures/entry-326.wld").to_vec();
    let start = offset(&bytes, 0);
    let end = offset(&bytes, 1);
    let count = 293i16.to_le_bytes();
    // Locate BannerSystem.Save's paired array lengths in this release-326 header.
    let kills = (start..end - 1176)
        .find(|&i| bytes[i..i + 2] == count && bytes[i + 1174..i + 1176] == count)
        .unwrap();
    bytes[kills + 2 + 4 * 7..kills + 2 + 4 * 8].copy_from_slice(&1234i32.to_le_bytes());
    bytes[kills + 1176 + 2 * 7..kills + 1176 + 2 * 8].copy_from_slice(&257u16.to_le_bytes());
    let events = kills + 1176 + 293 * 2;
    bytes[events + 4] = 1; // Moon Lord defeated.
    bytes[events + 14] = 1; // Solar tower active.
    let state = World::parse(bytes).unwrap().join_state().unwrap();
    assert_eq!(state.banners.kills[7], 1234);
    assert_eq!(state.banners.claimable[7], 257);
    assert_eq!(state.shields, [50, 0, 0, 0]);
}

#[test]
fn official_town_records_preserve_housing_and_variations() {
    let w = World::parse(include_bytes!("../../../docs/fixtures/entry-326.wld").to_vec()).unwrap();
    assert!(w.shimmered_npc_types().is_empty());
    assert_eq!(w.npcs().len(), 2);
    assert_eq!(w.npcs()[0].kind, 37);
    let guide = &w.npcs()[1];
    assert_eq!(guide.kind, 22);
    let town = guide.town.as_ref().unwrap();
    assert_eq!(town.name, "Tanner");
    assert_eq!(town.home, [2095, 232]);
    assert!(town.homeless);
    assert_eq!(town.variation, Some(0));
}

#[test]
fn edited_runs_and_chest_slots_survive_reopening_and_keep_the_original_backup() {
    use std::collections::BTreeMap;
    use terraria_world::ChestStack;
    let bytes = include_bytes!("../../../docs/fixtures/entry-326.wld");
    let original = World::parse(bytes.to_vec()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.wld");
    original.save(&path).unwrap();
    let mut blocks = BTreeMap::new();
    for y in [0, 1, 400, 401, 1199] {
        let mut tile = original.tile(2099, y).unwrap();
        tile.block = Some(7);
        tile.frame = None;
        blocks.insert([2099, y as i16], tile);
    }
    let slots = BTreeMap::from([
        (
            (0, 0),
            ChestStack {
                item: 0,
                prefix: 0,
                stack: 0,
            },
        ),
        (
            (0, 1),
            ChestStack {
                item: 12,
                prefix: 0,
                stack: 42,
            },
        ),
    ]);
    let edited = original.with_edits(&blocks, &slots).unwrap();
    original.publish_edits(&edited, &path).unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("session.wld.terra-rs.bak")).unwrap(),
        bytes
    );
    let disk = World::load(&path).unwrap();
    for y in 0..1200 {
        let expected = blocks
            .get(&[2099, y as i16])
            .copied()
            .unwrap_or_else(|| original.tile(2099, y).unwrap());
        assert_eq!(
            terraria_protocol::section::section_tile(disk.tile(2099, y).unwrap()),
            terraria_protocol::section::section_tile(expected)
        );
    }
    assert_eq!(disk.metadata().revision, original.metadata().revision + 1);
    assert_eq!(disk.chests()[0].items.len(), 1);
    let stored = &disk.chests()[0].items[0];
    assert_eq!((stored.slot, stored.stack, stored.item_type), (1, 42, 12));
    let modified = std::fs::read(&path).unwrap();
    assert!(original.publish_edits(&edited, &path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), modified);
    assert_eq!(
        std::fs::read(dir.path().join("session.wld.terra-rs.bak")).unwrap(),
        bytes
    );
}

#[test]
fn release_329_projects_the_optional_no_spiders_world_flag() {
    let mut bytes = include_bytes!("../../../docs/fixtures/entry-326.wld").to_vec();
    let previous = World::parse(bytes.clone()).unwrap();
    // In this fixture the manifest has a two-byte string length prefix.
    let manifest = bytes
        .windows(b"{\"GenPassResults\"".len())
        .position(|w| w == b"{\"GenPassResults\"")
        .unwrap()
        - 2;
    bytes.insert(manifest, 1);
    bytes[..4].copy_from_slice(&329i32.to_le_bytes());
    for index in 1..11 {
        let value = (offset(&bytes, index) as i32 + 1).to_le_bytes();
        bytes[26 + index * 4..30 + index * 4].copy_from_slice(&value);
    }
    let current = World::parse(bytes).unwrap();
    assert_eq!(current.metadata().file_version, 329);
    let mut expected = previous.world_data().unwrap();
    expected.progression[10] |= 0x20;
    assert_eq!(current.world_data().unwrap(), expected);
    assert_eq!(
        current.section([10, 1]).unwrap(),
        previous.section([10, 1]).unwrap()
    );
}

#[test]
fn rolling_backups_retain_the_three_preceding_revisions() {
    let mut base =
        World::parse(include_bytes!("../../../docs/fixtures/entry-326.wld").to_vec()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rolling.wld");
    base.save(&path).unwrap();
    for _ in 0..6 {
        let next = base
            .with_edits(&Default::default(), &Default::default())
            .unwrap();
        base.publish_edits(&next, &path).unwrap();
        base = next;
    }
    let revision = base.metadata().revision;
    assert_eq!(
        World::load(dir.path().join("rolling.wld.terra-rs.bak"))
            .unwrap()
            .metadata()
            .revision,
        revision - 1
    );
    let mut older = ["even", "odd"].map(|suffix| {
        World::load(
            dir.path()
                .join(format!("rolling.wld.terra-rs.bak.{suffix}")),
        )
        .unwrap()
        .metadata()
        .revision
    });
    older.sort();
    assert_eq!(older, [revision - 3, revision - 2]);
}
