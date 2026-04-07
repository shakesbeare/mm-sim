use bevy::prelude::*;
use crossterm::{
    ExecutableCommand,
    cursor::{Hide, Show},
    terminal::{Clear, ClearType},
};
use mm_sim::{
    MatchStats, lobby::{
        Lobby, WaitingForPlayers, add_players_to_lobbies, end_matches, merge_lobbies, start_lobbies,
    }, player::{InQueue, LoggedOut, Player}, player_management::{
        NeedNewPlayer, PlayerAddTimer, PlayerCount, add_new_players, frustrated_players, give_up_queue, request_new_player, spawn_random_player_archetype, unfrustrated_players
    }, stats::*, time::SimTime
};

use extra_collections::RingBuf;

fn main() {
    setup_logging().unwrap();

    // show the cursor on ctrlc
    ctrlc::set_handler(move || {
        std::io::stdout().execute(Show).unwrap();
        std::process::exit(0);
    })
    .expect("Failed to create CTRL-C handler");

    // show the cursor on crash
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        std::io::stdout().execute(Show).unwrap();
        default_panic(panic_info)
    }));

    std::io::stdout().execute(Hide).unwrap();

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.build().disable::<bevy::log::LogPlugin>());

    app.insert_resource(MMRStats::default());
    app.insert_resource(Ticks(RingBuf::new(GRAPH_POINTS)));
    app.insert_resource(TicksSinceStart::default());
    app.insert_resource(MeanWaitTime(RingBuf::new(SMOOTHING)));
    app.insert_resource(LowWaitTime(RingBuf::new(SMOOTHING)));
    app.insert_resource(MedianWaitTime(RingBuf::new(SMOOTHING)));
    app.insert_resource(HighWaitTime(RingBuf::new(SMOOTHING)));
    app.insert_resource(MatchStats::default());
    app.insert_resource(mm_sim::fs::setup().unwrap());
    app.insert_resource(SimTime::default());
    app.insert_resource(PlayerCount::default());

    app.add_message::<NeedNewPlayer>();

    app.add_systems(Startup, startup);

    app.add_systems(PreUpdate, tick);
    app.add_systems(PreUpdate, request_new_player);

    app.add_systems(
        Update,
        (add_players_to_lobbies, merge_lobbies, start_lobbies).chain(),
    );
    app.add_systems(Update, spawn_random_player_archetype);
    app.add_systems(Update, end_matches);

    app.add_systems(Update, (mmr_stats, wait_time_stats, display_stats).chain());

    app.add_systems(PostUpdate, add_new_players);
    app.add_systems(PostUpdate, frustrated_players);
    app.add_systems(PostUpdate, unfrustrated_players);
    app.add_systems(PostUpdate, give_up_queue);

    app.run();
}

fn setup_logging() -> Result<()> {
    use tracing_subscriber::Layer as _;
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;

    #[cfg(debug_assertions)]
    let e_filter = tracing_subscriber::EnvFilter::new("info,mm_sim=trace");
    #[cfg(not(debug_assertions))]
    let e_filter = tracing_subscriber::EnvFilter::new("info");

    // let stderr_layer = tracing_subscriber::fmt::layer()
    //     .pretty()
    //     .with_writer(std::io::stderr)
    //     .with_filter(e_filter.clone());

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
        // .with(stderr_layer)
        .with(file_layer)
        .try_init()?;

    Ok(())
}

fn startup(mut commands: Commands, mut flood_writer: MessageWriter<NeedNewPlayer>) {
    std::io::stdout().execute(Clear(ClearType::All)).unwrap();
    commands.spawn(LogTimer::default());
    commands.spawn(PlayerAddTimer::default());
}

fn tick(
    mut timers: Query<&mut mm_sim::TickTimer>,
    mut floating_players: Query<&mut Player<InQueue>>,
    mut lobbies: Query<&mut Lobby<WaitingForPlayers>>,
    mut logged_out_players: Query<&mut Player<LoggedOut>>,
    mut sim_time: ResMut<SimTime>,
    log_timer: Query<&mut LogTimer>,
    time: Res<Time>,
) {
    for mut t in timers.iter_mut() {
        t.tick();
    }

    for mut t in log_timer {
        t.timer.tick(time.delta());
    }

    for mut p in floating_players.iter_mut() {
        p.tick();
    }

    for mut l in lobbies.iter_mut() {
        l.tick();
    }

    for mut p in logged_out_players.iter_mut() {
        p.tick();
    }

    sim_time.tick();
}
