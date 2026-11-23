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
        data.extend([0xaa, i as u8]);
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
