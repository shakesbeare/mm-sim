use bevy::prelude::*;
use rand::Rng as _;
use rand_distr::Distribution as _;
use rand_distr::Normal;

use crate::TickTimer;
use crate::player::Player;
use crate::queue::Queue;
use crate::{MATCH_PLAYER_COUNT, TEAM_COUNT, TEAM_SIZE};

#[derive(Component, Debug, PartialEq)]
pub struct MatchInProgress {
    teams: [[Player; TEAM_SIZE]; TEAM_COUNT],
    /// Contains the index of the winning team if the game is over
    result: Option<usize>,
}

impl MatchInProgress {
    pub fn new(players: [Player; MATCH_PLAYER_COUNT]) -> Self {
        let mut teams = [[None; TEAM_SIZE]; TEAM_COUNT];
        for (k, player) in players.into_iter().enumerate() {
            let slot = k % TEAM_SIZE;
            let team = k % TEAM_COUNT;

            teams[team][slot] = Some(player);
        }

        let teams = teams.map(|t| t.map(|p| p.unwrap()));
        Self {
            teams,
            result: None,
        }
    }

    pub fn teams(&self) -> [[Player; TEAM_SIZE]; TEAM_COUNT] {
        self.teams
    }

    pub fn players(&self) -> &[Player] {
        self.teams.as_flattened()
    }

    pub fn enemies_of(&self, player: &Player) -> Vec<&Player> {
        let team = self
            .teams()
            .into_iter()
            .find(|t| t.contains(player))
            .unwrap();

        self.players()
            .iter()
            .filter(|p| !team.contains(p))
            .collect()
    }

    pub fn range(&self) -> f32 {
        let mut min = f32::MAX;
        let mut max = 0.0;

        for p in self.players().iter() {
            if p.mmr() < min {
                min = p.mmr();
            }

            if p.mmr() > max {
                max = p.mmr();
            }
        }

        max - min
    }

    /// Returns the index of the winning team
    pub fn finish_match(&mut self) {
        let teams = self.teams();
        let mut rng = rand::rng();
        // the team with the highest total SR (not mmr) will win the match, for now
        self.result = Some(
            teams
                .iter()
                .map(|t| {
                    t.iter()
                        .map(|p| {
                            let normal = Normal::new(0.0, p.consistency()).unwrap();
                            let offset = normal.sample(&mut rng) as f32;
                            p.sr() + offset
                        })
                        .sum::<f32>()
                })
                .enumerate()
                .max_by_key(|&(_, value)| value as usize)
                .map(|(i, _)| i)
                .unwrap(),
        );
    }

    pub fn get_result(&self) -> Option<usize> {
        self.result
    }

    /// Returns None if the game has not yet ended
    pub fn did_player_win(&self, player: &Player) -> Option<bool> {
        let result = self.result?;
        let winning_team = self.teams()[result];
        Some(winning_team.contains(player))
    }
}

pub fn make_matches(mut commands: Commands, mut queue: ResMut<Queue>) {
    let mut rng = rand::rng();
    let matches = queue.make_matches();
    for r#match in matches {
        let m = MatchInProgress::new(r#match);
        // let range = m.range();
        let duration = rng.random_range(10..60);
        commands.spawn((m, TickTimer::new(duration * 60, TimerMode::Once)));
    }
}

pub fn end_matches(
    mut commands: Commands,
    mut queue: ResMut<Queue>,
    matches_in_progress: Query<(Entity, &mut MatchInProgress, &mut TickTimer)>,
) {
    let mip: Vec<&MatchInProgress> = matches_in_progress.iter().map(|(_, m, _)| m).collect();
    let player_count = mip.iter().flat_map(|m| m.players()).count() + queue.len();

    for (e, mut m, mut timer) in matches_in_progress {
        if timer.just_finished() {
            m.finish_match();
            let players = m.teams();
            for team in players {
                for mut player in team {
                    let should_requeue = player.finished_match(m.as_ref(), player_count);
                    if should_requeue {
                        queue.insert(player);
                    } else {
                        // spawned players are logged out 
                        commands.spawn(player);
                    }
                }
            }
            commands.entity(e).despawn();
        }
    }
}
