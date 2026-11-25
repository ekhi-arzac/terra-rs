use crate::{
    Error,
    terrain::{Checkpoint, LoadedWorld},
};
use std::{num::NonZeroU32, path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    sync::mpsc,
    time::{Instant, MissedTickBehavior, interval_at},
};

pub enum ConsoleCommand {
    Save,
    Playing,
    Exit,
}

pub struct Administration {
    pub autosave_seconds: Option<NonZeroU32>,
    pub commands: mpsc::Receiver<ConsoleCommand>,
}

impl Default for Administration {
    fn default() -> Self {
        Self {
            autosave_seconds: NonZeroU32::new(600),
            commands: mpsc::channel(1).1,
        }
    }
}

pub(super) async fn save_worker(
    world: Arc<LoadedWorld>,
    path: PathBuf,
    seconds: Option<NonZeroU32>,
    mut requests: mpsc::Receiver<()>,
) -> Result<(), Error> {
    let mut timer = seconds.map(|n| {
        let period = Duration::from_secs(u64::from(n.get()));
        let mut tick = interval_at(Instant::now() + period, period);
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        tick
    });
    enum Trigger {
        Manual,
        Timer,
        Shutdown,
    }
    let mut checkpoint = Checkpoint::default();
    loop {
        let trigger = tokio::select! {
            biased;
            request = requests.recv() => if request.is_some() { Trigger::Manual } else { Trigger::Shutdown },
            _ = async { if let Some(tick) = &mut timer { tick.tick().await; } }, if timer.is_some() => Trigger::Timer,
        };
        if matches!(trigger, Trigger::Timer) && world.revision() == checkpoint.revision {
            continue;
        }
        let source = Arc::clone(&world);
        let file = path.clone();
        let (next, result) = tokio::task::spawn_blocking(move || {
            let result = source.save_changes(&file, &mut checkpoint);
            (checkpoint, result)
        })
        .await?;
        checkpoint = next;
        if matches!(trigger, Trigger::Shutdown) {
            return result.map(|_| ());
        }
        match result {
            Ok(false) if matches!(trigger, Trigger::Manual) => {
                tracing::info!("world is already saved")
            }
            Err(error) => tracing::error!(%error, world=%path.display(), "world save failed"),
            _ => {}
        }
    }
}
