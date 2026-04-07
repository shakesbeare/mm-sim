use bevy::prelude::*;
use crossterm::{
    ExecutableCommand,
    cursor::{MoveTo, RestorePosition, SavePosition},
};
use rgb::RGB8;
use textplots::{Chart, ColorPlot as _, Shape};
use tracing::*;

use crate::{
    MEAN_MMR, MatchStats,
    fs::FileHandles,
    lobby::{InProgress, Lobby, WaitingForPlayers},
    player::{Any, InLobby, InQueue, LoggedOut, NeverToReturn, Player},
};

use extra_collections::RingBuf;

pub const GRAPH_POINTS: usize = 20_000;
pub const SMOOTHING: usize = 2000;

#[derive(Resource)]
pub struct MMRStats {
    pub mean: RingBuf<f64>,
    pub min: RingBuf<f64>,
    pub max: RingBuf<f64>,
    pub median: RingBuf<f64>,
    pub mean_rating_range: RingBuf<f64>,
}

impl MMRStats {
    pub fn new() -> Self {
        Self {
            mean: RingBuf::new(GRAPH_POINTS),
            min: RingBuf::new(GRAPH_POINTS),
            max: RingBuf::new(GRAPH_POINTS),
            median: RingBuf::new(SMOOTHING),
            mean_rating_range: RingBuf::new(SMOOTHING),
        }
    }

    pub fn median(&self) -> f64 {
        self.median.iter().sum::<f64>() / self.median.len() as f64
    }

    pub fn mean_rating_range(&self) -> f64 {
        self.mean_rating_range.iter().sum::<f64>() / self.mean_rating_range.len() as f64
    }
}

impl Default for MMRStats {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Resource)]
pub struct Ticks(pub RingBuf<f64>);

#[derive(Resource, Default)]
pub struct TicksSinceStart(pub usize);

#[derive(Resource)]
pub struct MeanWaitTime(pub RingBuf<f64>);

#[derive(Resource)]
pub struct LowWaitTime(pub RingBuf<usize>);

#[derive(Resource)]
pub struct HighWaitTime(pub RingBuf<usize>);

#[derive(Resource)]
pub struct MedianWaitTime(pub RingBuf<usize>);

#[derive(Component)]
pub struct LogTimer {
    pub timer: Timer,
}

impl Default for LogTimer {
    fn default() -> Self {
        Self {
            timer: Timer::new(
                std::time::Duration::from_secs_f32(1. / 12.),
                TimerMode::Repeating,
            ),
        }
    }
}

pub fn wait_time_stats(
    players_in_queue: Query<&Player<InQueue>>,
    lobbies_waiting: Query<&Lobby<WaitingForPlayers>>,
    mut mean_wait_time: ResMut<MeanWaitTime>,
    mut low_wait_time: ResMut<LowWaitTime>,
    mut high_wait_time: ResMut<HighWaitTime>,
    mut median_wait_time: ResMut<MedianWaitTime>,
) {
    let players_waiting: Vec<&Player<Any>> = lobbies_waiting
        .iter()
        .flat_map(|l| l.players().iter())
        .filter_map(|p| {
            if p.is_none() {
                None
            } else {
                Some(p.as_ref().unwrap())
            }
        })
        .map(|p| p.as_any())
        .collect();

    let players_in_queue: Vec<&Player<Any>> = players_waiting
        .clone()
        .into_iter()
        .chain(players_in_queue.iter().map(|p| p.as_any()))
        .collect();

    let mut mean_wait = players_in_queue
        .iter()
        .map(|p| p.queue_stats().wait_time().unwrap())
        .sum::<usize>() as f64
        / players_in_queue.iter().len() as f64;

    if mean_wait.is_nan() {
        mean_wait = 0.0;
    }

    mean_wait_time.0.push(mean_wait);

    low_wait_time.0.push(
        players_in_queue
            .iter()
            .map(|p| p.queue_stats().wait_time().unwrap())
            .min()
            .unwrap_or(0),
    );

    high_wait_time.0.push(
        players_in_queue
            .iter()
            .map(|p| p.queue_stats().wait_time().unwrap())
            .max()
            .unwrap_or(0),
    );

    median_wait_time.0.push(
        players_in_queue
            .iter()
            .map(|p| p.queue_stats().wait_time().unwrap())
            .nth(players_in_queue.iter().len() / 2)
            .unwrap_or(0),
    );
}

pub fn mmr_stats(
    players_in_queue: Query<&Player<InQueue>>,
    dead_players: Query<&Player<NeverToReturn>>,
    lobbies_waiting: Query<&Lobby<WaitingForPlayers>>,
    lobbies_in_progress: Query<&Lobby<InProgress>>,
    mut mmr_stats: ResMut<MMRStats>,
) {
    let players_waiting: Vec<&Player<Any>> = lobbies_waiting
        .iter()
        .flat_map(|l| l.players().iter())
        .filter_map(|p| {
            if p.is_none() {
                None
            } else {
                Some(p.as_ref().unwrap())
            }
        })
        .map(|p| p.as_any())
        .collect();
    let players_in_progress: Vec<&Player<Any>> = lobbies_in_progress
        .iter()
        .flat_map(|l| l.players())
        .map(|p| p.as_any())
        .collect();

    let all_players: Vec<&Player<Any>> = players_waiting
        .clone()
        .into_iter()
        .chain(players_in_progress)
        .chain(dead_players.iter().map(|p| p.as_any()))
        .chain(players_in_queue.iter().map(|p| p.as_any()))
        .collect();

    let player_count = all_players.len();

    let mut mean_range = lobbies_in_progress.iter().map(|m| m.range()).sum::<f64>()
        / lobbies_in_progress.iter().count() as f64;
    if mean_range.is_nan() {
        mean_range = 0.0;
    }

    mmr_stats.mean_rating_range.push(mean_range);

    let mmr_iter: Vec<f64> = all_players.iter().map(|p| p.rating()).collect();

    mmr_stats
        .mean
        .push(mmr_iter.iter().sum::<f64>() / player_count as f64);
    mmr_stats
        .min
        .push(mmr_iter.iter().fold(f64::INFINITY, |a, &b| a.min(b)));
    mmr_stats
        .max
        .push(mmr_iter.iter().fold(0.0_f64, |a, &b| a.max(b)));
    mmr_stats
        .median
        .push(*mmr_iter.get(player_count / 2).unwrap_or(&0.0));
}

pub fn display_stats(
    lobbies_in_progress: Query<&Lobby<InProgress>>,
    players_in_queue: Query<&Player<InQueue>>,
    lobbies_waiting: Query<&Lobby<WaitingForPlayers>>,
    dead_players: Query<&Player<NeverToReturn>>,
    mut log_timer: Query<&LogTimer>,
    mmr_stats: Res<MMRStats>,
    mean_wait_time: Res<MeanWaitTime>,
    low_wait_time: Res<LowWaitTime>,
    high_wait_time: Res<HighWaitTime>,
    median_wait_time: Res<MedianWaitTime>,
    match_stats: Res<MatchStats>,
    mut ticks: ResMut<Ticks>,
    mut ticks_since: ResMut<TicksSinceStart>,
    logged_out_players: Query<&Player<LoggedOut>>,
    // mut file_handles: ResMut<FileHandles>,
) {
    let players_waiting: Vec<&Player<Any>> = lobbies_waiting
        .iter()
        .flat_map(|l| l.players().iter())
        .filter_map(|p| {
            if p.is_none() {
                None
            } else {
                Some(p.as_ref().unwrap())
            }
        })
        .map(|p| p.as_any())
        .collect();

    let players_in_game: Vec<&Player<Any>> = lobbies_in_progress
        .iter()
        .flat_map(|m| m.players())
        .map(|p| p.as_any())
        .collect();

    let all_players: Vec<&Player<Any>> = players_in_queue
        .iter()
        .map(|p| p.as_any())
        .chain(players_in_game)
        .chain(players_waiting.clone())
        .collect();

    let players_in_queue: Vec<&Player<Any>> = players_waiting
        .clone()
        .into_iter()
        .chain(players_in_queue.iter().map(|p| p.as_any()))
        .collect();

    let player_count_in_match = lobbies_in_progress.iter().flat_map(|m| m.players()).count();
    let player_count = player_count_in_match + players_in_queue.iter().len();
    let dead_count = dead_players.iter().len();

    let mean_wait = mean_wait_time.0.iter().sum::<f64>() / SMOOTHING as f64;
    let low_wait = low_wait_time.0.iter().sum::<usize>() / SMOOTHING;
    let high_wait = high_wait_time.0.iter().sum::<usize>() / SMOOTHING;
    let median_wait = median_wait_time.0.iter().sum::<usize>() / SMOOTHING;

    let timer = log_timer.single_mut().unwrap();

    ticks.0.push(ticks_since.0 as f64);

    if all_players.is_empty() {
        tracing::error!("all players: {}", all_players.len());
        tracing::error!("lobbies waiting: {}", lobbies_waiting.iter().len());
        tracing::error!("players waiting: {}", players_waiting.len());
    }

    let Some((highest_mmr_player_index, _)) = all_players
        .iter()
        .enumerate()
        .map(|(k, p)| (k, p.rating()))
        .max_by_key(|(_, p)| *p as usize)
    else {
        tracing::warn!("Aborting display, no players logged in");
        return;
    };

    let (lowest_mmr_player_index, _) = all_players
        .iter()
        .enumerate()
        .map(|(k, p)| (k, p.rating()))
        .min_by_key(|(_, p)| *p as usize)
        .unwrap();

    let highest_mmr_player = all_players.get(highest_mmr_player_index).unwrap();
    let lowest_mmr_player = all_players.get(lowest_mmr_player_index).unwrap();

    let mean_mmr_points: Vec<(f32, f32)> = ticks
        .0
        .iter()
        .cloned()
        .zip(mmr_stats.mean.iter())
        .map(|(l, r)| (l as f32, *r as f32))
        .collect();
    let min_mmr_points: Vec<(f32, f32)> = ticks
        .0
        .iter()
        .cloned()
        .zip(mmr_stats.min.iter())
        .map(|(l, r)| (l as f32, *r as f32))
        .collect();

    let max_mmr_points: Vec<(f32, f32)> = ticks
        .0
        .iter()
        .cloned()
        .zip(mmr_stats.max.iter())
        .map(|(l, r)| (l as f32, *r as f32))
        .collect();

    let right_bound = f64::max(*ticks.0.iter().last().unwrap(), GRAPH_POINTS as f64) as f32;
    let left_bound = f64::max(*ticks.0.iter().next().unwrap(), 0.0) as f32;

    let logged_out_count = logged_out_players.iter().count();

    if timer.timer.just_finished() {
        std::io::stdout().execute(SavePosition).unwrap();
        std::io::stdout().execute(MoveTo(0, 0)).unwrap();

        println!(
            "Average Queue Time {:07.2} — Median Queue Time: {:05} — Low Queue Time: {:05} — High Queue Time: {:07} — Gave Up: {:07} — Give Ups per game: {:07.5}",
            mean_wait,
            median_wait,
            low_wait,
            high_wait,
            match_stats.gave_up,
            match_stats.gave_up as f64 / match_stats.matches_played as f64,
        );

        println!(
            "Players in queue: {:07} — Players in match: {:07} — Total Players in Pool {:07} — Logged Out Players: {:07} — Dead Players: {:010}",
            players_in_queue.iter().len(),
            player_count_in_match,
            player_count,
            logged_out_count,
            dead_count
        );

        print!(
            "Highest MMR in Pool — MMR: {:04.0} — Matches Played: {:07} | ",
            highest_mmr_player.rating(),
            highest_mmr_player.matches_played(),
        );

        println!(
            "Lowest MMR in Pool — MMR: {:04.0} — Matches Played: {:07}",
            lowest_mmr_player.rating(),
            lowest_mmr_player.matches_played(),
        );

        println!(
            "Mean MMR: {:04.0} — Median MMR: {:04.0} — Mean MMR Range {:07.2}",
            mmr_stats.mean.last().copied().unwrap_or(0.0),
            mmr_stats.median(),
            mmr_stats.mean_rating_range()
        );

        let chart_y_max = f64::max(
            MEAN_MMR * 2.0,
            mmr_stats.max.last().copied().unwrap_or(0.0) + 500.0,
        ) as f32;

        Chart::new_with_y_range(300, 100, left_bound, right_bound, 0., chart_y_max)
            .linecolorplot(
                &Shape::Points(mean_mmr_points.as_slice()),
                RGB8 {
                    r: 0,
                    g: 0,
                    b: 255_u8,
                },
            )
            .linecolorplot(
                &Shape::Points(min_mmr_points.as_slice()),
                RGB8 {
                    r: 255_u8,
                    g: 0,
                    b: 0,
                },
            )
            .linecolorplot(
                &Shape::Points(max_mmr_points.as_slice()),
                RGB8 {
                    r: 0,
                    g: 255_u8,
                    b: 0,
                },
            )
            .display();

        std::io::stdout().execute(RestorePosition).unwrap();
    }

    // file_handles.queue_stats.write_record(&[
    //     format!("{}", mean_wait),
    //     format!("{}", mean_range),
    //     format!("{}", queue.len()),
    //     format!("{}", matches_in_progress.iter().flat_map(|m| m.players()).count()),
    //     format!("{}", player_count),
    //     format!("{}", logged_out_count),
    //     format!("{}", mean_mmr),
    //     format!("{}", median_mmr),
    //     format!("{}", highest_mmr_player.rating()),
    //     format!("{}", highest_mmr_player.matches_played()),
    //     format!("{}", lowest_mmr_player.rating()),
    //     format!("{}", lowest_mmr_player.matches_played())
    // ]).unwrap();

    ticks_since.0 += 1;
}
