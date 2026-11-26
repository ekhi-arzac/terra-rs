use std::{env, error::Error, hint::black_box, time::Instant};
use tempfile::tempdir;
use terraria_world::{World, WorldClock};

fn main() -> Result<(), Box<dyn Error>> {
    let Some(path) = env::var_os("TERRARIA_BENCH_WORLD") else {
        println!("Set TERRARIA_BENCH_WORLD to an absolute path to an existing Terraria world.");
        return Ok(());
    };
    let start = Instant::now();
    let world = World::load(path)?;
    println!("Read, validate and index world: {:?}", start.elapsed());
    let meta = world.metadata();
    let start = Instant::now();
    for i in 0..10_000u32 {
        black_box(world.tile((i * 271) % meta.width, (i * 197) % meta.height)?);
    }
    println!(
        "Indexed tile lookup: {:.1} ns/tile",
        start.elapsed().as_nanos() as f64 / 10_000.0
    );
    let index = [
        (meta.spawn.0.max(0) / 200) as u16,
        (meta.spawn.1.max(0) / 150) as u16,
    ];
    let start = Instant::now();
    for _ in 0..100 {
        black_box(
            terraria_protocol::Packet::TileSection(Box::new(world.section(index)?)).encode()?,
        );
    }
    println!(
        "Read and compress spawn section: {:?}/section",
        start.elapsed() / 100
    );
    let dir = tempdir()?;
    let start = Instant::now();
    world.save(dir.path().join("bench.wld"))?;
    println!(
        "Stage, sync, verify and publish world: {:?}",
        start.elapsed()
    );
    let mut clock = WorldClock::from_world(meta.time, &world.world_data()?);
    let rate = world.saved_time_rate()?;
    let steps = 1_000_000;
    let start = Instant::now();
    for _ in 0..steps {
        black_box(black_box(&mut clock).tick(black_box(rate)));
    }
    println!(
        "Advance world clock: {:.1} ns/step",
        start.elapsed().as_nanos() as f64 / f64::from(steps),
    );
    let start = Instant::now();
    for _ in 0..10 {
        black_box(world.checkpoint(&Default::default(), &Default::default(), clock)?);
    }
    println!(
        "Build and validate clock-only checkpoint: {:?}/checkpoint",
        start.elapsed() / 10,
    );
    Ok(())
}
