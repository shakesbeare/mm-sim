use bevy::log::tracing;
use bevy::prelude::*;
use crossterm::{
    ExecutableCommand,
    cursor::Hide,
    terminal::{Clear, ClearType},
};
use mm_sim::{
    GaveUp, display::{
        AvgMMR, GRAPH_POINTS, HighWaitTime, LogTimer, LowWaitTime, MaxMMR, MeanRatingRange, MeanWaitTime, MedianWaitTime, MinMMR, SMOOTHING, Ticks, TicksSinceStart, queue_stats
    }, r#match::{end_matches, make_matches}, player::Player, player_management::{STARTING_PLAYER_COUNT, try_add_player}, queue::Queue
};

use extra_collections::RingBuf;

fn main() {
    setup_logging().unwrap();
    std::io::stdout().execute(Hide).unwrap();

    let mut app = App::new();
    app.add_plugins(DefaultPlugins);

    app.insert_resource(Queue::default());
    app.insert_resource(AvgMMR(RingBuf::new(GRAPH_POINTS)));
    app.insert_resource(MinMMR(RingBuf::new(GRAPH_POINTS)));
    app.insert_resource(MaxMMR(RingBuf::new(GRAPH_POINTS)));
    app.insert_resource(Ticks(RingBuf::new(GRAPH_POINTS)));
    app.insert_resource(TicksSinceStart::default());
    app.insert_resource(MeanWaitTime(RingBuf::new(GRAPH_POINTS)));
    app.insert_resource(LowWaitTime(RingBuf::new(SMOOTHING)));
    app.insert_resource(MedianWaitTime(RingBuf::new(SMOOTHING)));
    app.insert_resource(HighWaitTime(RingBuf::new(SMOOTHING)));
    app.insert_resource(MeanRatingRange(RingBuf::new(SMOOTHING)));
    app.insert_resource(GaveUp::default());
    app.insert_resource(mm_sim::fs::setup().unwrap());

    app.add_systems(Startup, startup);

    app.add_systems(PreUpdate, tick);

    app.add_systems(Update, (queue_stats, make_matches, end_matches).chain());

    app.add_systems(PostUpdate, try_add_player);

    app.run();
}

fn setup_logging() -> Result<()> {
    use tracing_subscriber::Layer as _;
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;

    #[cfg(debug_assertions)]
    let e_filter = tracing_subscriber::EnvFilter::new("info,mm_sim=debug");
    #[cfg(not(debug_assertions))]
    let e_filter = tracing_subscriber::EnvFilter::new("info");

    let queue_stats_appender = tracing_appender::rolling::RollingFileAppender::builder()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("mm_sim")
        .filename_suffix("log")
        .build("./logs")?;

    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(true)
        .with_writer(queue_stats_appender)
        .with_filter(e_filter);

    tracing_subscriber::Registry::default()
        .with(file_layer)
        .try_init()?;

    Ok(())
}

fn startup(mut commands: Commands, mut queue: ResMut<Queue>) {
    std::io::stdout().execute(Clear(ClearType::All)).unwrap();
    commands.spawn(LogTimer::default());
    tracing::trace!("Inserting {} players", STARTING_PLAYER_COUNT);
    for _ in 0..STARTING_PLAYER_COUNT {
        queue.insert(Player::new(None, None, None, None));
    }
}

fn tick(
    commands: Commands,
    mut timers: Query<&mut mm_sim::TickTimer>,
    mut queue: ResMut<Queue>,
    gave_up: ResMut<GaveUp>,
    log_timer: Query<&mut LogTimer>,
    time: Res<Time>,
) {
    for mut t in timers.iter_mut() {
        t.tick();
    }

    for mut t in log_timer {
        t.timer.tick(time.delta());
    }

    queue.tick(commands, gave_up);
}
