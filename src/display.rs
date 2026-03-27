use bevy::prelude::*;
use crossterm::{
    ExecutableCommand,
    cursor::{MoveTo, RestorePosition, SavePosition},
};
use rgb::RGB8;
use textplots::{Chart, ColorPlot as _, Shape};

use crate::{
    MEAN_MMR, MatchStats, fs::FileHandles, r#match::Match, player::Player, queue::Queue
};

use extra_collections::RingBuf;

pub const GRAPH_POINTS: usize = 20_000;
pub const SMOOTHING: usize = 2000;

#[derive(Resource)]
pub struct AvgMMR(pub RingBuf<f64>);

#[derive(Resource)]
pub struct MinMMR(pub RingBuf<f64>);

#[derive(Resource)]
pub struct MaxMMR(pub RingBuf<f64>);

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

#[derive(Resource)]
pub struct MeanRatingRange(pub RingBuf<f64>);

#[derive(Component)]
pub struct LogTimer {
    pub timer: Timer,
}

impl Default for LogTimer {
    fn default() -> Self {
        Self {
            timer: Timer::new(
                std::time::Duration::from_secs_f32(1. / 24.),
                TimerMode::Repeating,
            ),
        }
    }
}

pub fn queue_stats(
    matches_in_progress: Query<&Match>,
    queue: Res<Queue>,
    mut log_timer: Query<&LogTimer>,
    mut avg_mmr: ResMut<AvgMMR>,
    mut min_mmr_r: ResMut<MinMMR>,
    mut max_mmr_r: ResMut<MaxMMR>,
    mut mean_wait_time: ResMut<MeanWaitTime>,
    mut low_wait_time: ResMut<LowWaitTime>,
    mut high_wait_time: ResMut<HighWaitTime>,
    mut median_wait_time: ResMut<MedianWaitTime>,
    mut mean_rating_range: ResMut<MeanRatingRange>,
    match_stats: Res<MatchStats>,
    mut ticks: ResMut<Ticks>,
    mut ticks_since: ResMut<TicksSinceStart>,
    logged_out_players: Query<&Player>,
    mut file_handles: ResMut<FileHandles>,
) {
    let timer = log_timer.single_mut().unwrap();
    let mut mean_wait =
        queue.iter().map(|p| p.wait_time).sum::<usize>() as f64 / queue.len() as f64;
    if mean_wait.is_nan() {
        mean_wait = 0.0;
    }

    mean_wait_time.0.push(mean_wait);
    let mean_wait = mean_wait_time.0.iter().sum::<f64>() / GRAPH_POINTS as f64;

    low_wait_time.0.push(queue.iter().map(|p| p.wait_time).min().unwrap_or(0));
    let low_wait = low_wait_time.0.iter().sum::<usize>() / SMOOTHING;

    high_wait_time.0.push(queue.iter().map(|p| p.wait_time).max().unwrap_or(0));
    let high_wait = high_wait_time.0.iter().sum::<usize>() / SMOOTHING;

    median_wait_time.0.push(queue.iter().map(|p| p.wait_time).nth(queue.len() / 2).unwrap_or(0));
    let median_wait = median_wait_time.0.iter().sum::<usize>() / SMOOTHING;

    let mut mean_range = matches_in_progress.iter().map(|m| m.range()).sum::<f64>()
        / matches_in_progress.iter().count() as f64;
    if mean_range.is_nan() {
        mean_range = 0.0;
    }

    mean_rating_range.0.push(mean_range);
    let mean_range = mean_rating_range.0.iter().sum::<f64>() / SMOOTHING as f64;

    let player_count_in_match = matches_in_progress.iter().flat_map(|m| m.players()).count();
    let player_count = player_count_in_match + queue.len();
    let mmr_iter: Vec<f64> = matches_in_progress
        .iter()
        .flat_map(|m| m.players())
        .map(|p| p.rating())
        .chain(queue.iter().map(|qp| qp.player.rating()))
        .collect();

    let mean_mmr: f64 = mmr_iter.iter().sum::<f64>() / player_count as f64;

    let min_mmr = mmr_iter.iter().fold(f64::INFINITY, |a, &b| a.min(b));
    let max_mmr = mmr_iter.iter().fold(0.0_f64, |a, &b| a.max(b));

    avg_mmr.0.push(mean_mmr);
    min_mmr_r.0.push(min_mmr);
    max_mmr_r.0.push(max_mmr);
    ticks.0.push(ticks_since.0 as f64);

    let players_in_game: Vec<Player> = matches_in_progress
        .iter()
        .flat_map(|m| m.players())
        .copied()
        .collect();
    let all_players: Vec<Player> = queue
        .iter()
        .map(|p| p.player)
        .chain(players_in_game)
        .collect();

    let (highest_mmr_player_index, _) = all_players
        .iter()
        .enumerate()
        .map(|(k, p)| (k, p.rating()))
        .max_by_key(|(_, p)| *p as usize)
        .unwrap();

    let (lowest_mmr_player_index, _) = all_players
        .iter()
        .enumerate()
        .map(|(k, p)| (k, p.rating()))
        .min_by_key(|(_, p)| *p as usize)
        .unwrap();

    let highest_mmr_player = all_players.get(highest_mmr_player_index).unwrap();
    let lowest_mmr_player = all_players.get(lowest_mmr_player_index).unwrap();
    let avg_mmr_points: Vec<(f32, f32)> = ticks.0.iter().cloned().zip(avg_mmr.0.clone()).map(|(l, r)| (l as f32, r as f32)).collect();
    let min_mmr_points: Vec<(f32, f32)> =
        ticks.0.iter().cloned().zip(min_mmr_r.0.clone()).map(|(l, r)| (l as f32, r as f32)).collect();
    let max_mmr_points: Vec<(f32, f32)> =
        ticks.0.iter().cloned().zip(max_mmr_r.0.clone()).map(|(l, r)| (l as f32, r as f32)).collect();

    let right_bound = f64::max(*ticks.0.iter().last().unwrap(), GRAPH_POINTS as f64) as f32;
    let left_bound = f64::max(*ticks.0.iter().next().unwrap(), 0.0) as f32;

    let mean_mmr: f32 = (mmr_iter.iter().sum::<f64>() / player_count as f64) as f32;
    let median_mmr = mmr_iter.get(player_count / 2).unwrap();
    
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
        
        println!("Players in queue: {:07} — Players in match: {:07} — Total Players in Pool {:07} — Logged Out Players: {:07}",
            queue.len(),
            player_count_in_match,
            player_count,
            logged_out_count,
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
            mean_mmr, median_mmr, mean_range
        );

        let chart_y_max = f64::max(MEAN_MMR * 2.0, max_mmr + 500.0) as f32;

        Chart::new_with_y_range(300, 100, left_bound, right_bound, 0., chart_y_max)
            .linecolorplot(
                &Shape::Points(avg_mmr_points.as_slice()),
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

