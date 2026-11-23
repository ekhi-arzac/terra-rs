use std::{env, error::Error, ffi::OsString, io};
use terraria_world::World;

fn coord(value: &OsString) -> Result<u32, Box<dyn Error>> {
    Ok(value.to_str().ok_or("invalid coordinate")?.parse()?)
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    match args.as_slice() {
        [op, path] if op == "inspect" => {
            let w = World::load(path)?;
            let m = w.metadata();
            println!(
                "{} (ID {}, seed {}, revision {})",
                m.name, m.world_id, m.seed, m.revision
            );
            println!(
                "{} × {} tiles, spawn {:?}, {} chests, {} signs",
                m.width,
                m.height,
                m.spawn,
                w.chests().len(),
                w.signs().len()
            );
        }
        [op, path, x, y] if op == "tile" => {
            println!("{:?}", World::load(path)?.tile(coord(x)?, coord(y)?)?)
        }
        [op, src, dst] if op == "copy" => World::load(src)?.save(dst)?,
        _ => {
            return Err(io::Error::other(
                "usage: terraria-world inspect FILE | tile FILE X Y | copy SOURCE DEST",
            )
            .into());
        }
    }
    Ok(())
}
