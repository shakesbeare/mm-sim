use std::marker::PhantomData;

use bevy::prelude::*;
use rand::Rng as _;
use rand_distr::Distribution as _;
use rand_distr::Normal;
use skillratings::glicko2::Glicko2Rating;

use crate::MatchStats;
use crate::TickTimer;
use crate::lobby::private::*;
use crate::player::Player;
use crate::queue::Queue;
use crate::{MATCH_PLAYER_COUNT, TEAM_COUNT, TEAM_SIZE};

mod private {
    pub trait LobbyStatusMarker {}
}

#[derive(Debug, Default, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct WaitingForPlayers;
impl LobbyStatusMarker for WaitingForPlayers {}

#[derive(Debug, Default, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct InProgress;
impl LobbyStatusMarker for InProgress {}

#[derive(Debug, Default, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct Complete;
impl LobbyStatusMarker for Complete {}

#[derive(Debug, Default, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct TeamNumber(usize);

impl std::ops::Deref for TeamNumber {
    type Target = usize;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl TeamNumber {
    /// Creates a new instance without verifying that it is valid for any particular match
    /// # Safety
    ///     This function is safe to use if the number provided is verified or otherwise guaranteed
    ///     to contain a valid number for the lobby
    pub unsafe fn new_unchecked(n: usize) -> Self {
        Self(n)
    }

    /// Creates a new instance with a bounds check. Panics if the check fails.
    pub fn new(n: usize) -> Self {
        if n >= TEAM_COUNT {
            panic!("Attempted to create a TeamNumber not valid for current team configuration")
        }

        Self(n)
    }
}

#[derive(Debug, PartialEq, PartialOrd, Clone)]
pub enum LobbyStatus {
    /// Contains the players currently in the lobby without assigning teams
    WaitingForPlayers(Vec<Player>),
    InProgress,
    Complete(TeamNumber),
}

#[derive(Component, Clone, PartialEq, PartialOrd, Debug)]
pub struct Lobby<T: LobbyStatusMarker> {
    teams: [[Option<Player>; TEAM_SIZE]; TEAM_COUNT],
    // it feels kinda gross to have this information duplicated
    // possibly this should be an untagged union instead and then use T as the tag?
    // not a big deal for now, probably
    status: LobbyStatus,
    _marker: std::marker::PhantomData<T>,
}

impl<T: LobbyStatusMarker> AsRef<Lobby<T>> for Lobby<T> {
    fn as_ref(&self) -> &Lobby<T> {
        self
    }
}

impl Lobby<WaitingForPlayers> {
    pub fn players(&self) -> Vec<Player> {
        match self.status {
            LobbyStatus::WaitingForPlayers(ref players) => players.to_vec(),
            _ => unreachable!(),
        }
    }

    pub fn start_game(mut self) -> Lobby<InProgress> {
        for (k, player) in self.players().into_iter().enumerate() {
            let slot = k % TEAM_SIZE;
            let team = k % TEAM_COUNT;

            self.teams[team][slot] = Some(player);
        }

        // SAFETY:
        //     Markers are a zero sized type, so this shouldn't affect anything
        unsafe { std::mem::transmute::<Lobby<WaitingForPlayers>, Lobby<InProgress>>(self) }
    }
}

impl Lobby<InProgress> {
    /// Create a new lobby with given players to begin playing
    pub fn new(players: [Player; MATCH_PLAYER_COUNT]) -> Self {
        let mut teams = [[None; TEAM_SIZE]; TEAM_COUNT];
        for (k, player) in players.into_iter().enumerate() {
            let slot = k % TEAM_SIZE;
            let team = k % TEAM_COUNT;

            teams[team][slot] = Some(player);
        }

        // let teams = teams.map(|t| t.map(|p| p.unwrap()));
        Self {
            teams,
            status: LobbyStatus::InProgress,
            _marker: PhantomData,
        }
    }

    pub fn teams(&self) -> [[Player; TEAM_SIZE]; TEAM_COUNT] {
        match self.status {
            LobbyStatus::InProgress => self.teams.map(|t| t.map(|p| p.unwrap())),
            _ => unreachable!(),
        }
    }

    pub fn players(&self) -> Vec<Player> {
        match self.status {
            LobbyStatus::WaitingForPlayers(ref players) => players.to_vec(),
            _ => self
                .teams
                .as_flattened()
                .iter()
                .map(|p| p.unwrap())
                .collect(),
        }
    }

    /// Returns None if the teams have not yet been filled because the lobby is still waiting for
    /// players
    pub fn glicko_for_team(&self, team: usize) -> Glicko2Rating {
        let team = self.teams()[team];
        let rating = team.iter().map(|p| p.rating()).sum::<f64>() / TEAM_SIZE as f64;
        let volatility = team.iter().map(|p| p.volatility()).sum::<f64>() / TEAM_SIZE as f64;
        let deviation = team.iter().map(|p| p.rating_deviation()).sum::<f64>() / TEAM_SIZE as f64;

        Glicko2Rating {
            rating,
            deviation,
            volatility,
        }
    }

    pub fn glicko_for_enemies_of(&self, player: &Player) -> Glicko2Rating {
        let enemies = self.enemies_of(player);

        let rating = enemies.iter().map(|p| p.rating()).sum::<f64>() / enemies.len() as f64;
        let volatility = enemies.iter().map(|p| p.volatility()).sum::<f64>() / enemies.len() as f64;
        let deviation =
            enemies.iter().map(|p| p.rating_deviation()).sum::<f64>() / enemies.len() as f64;

        Glicko2Rating {
            rating,
            deviation,
            volatility,
        }
    }

    /// Returns None if the teams have not yet been filled because the lobby is still waiting for
    /// players
    pub fn enemies_of(&self, player: &Player) -> Vec<Player> {
        let team = self
            .teams()
            .into_iter()
            .find(|t| t.contains(player))
            .unwrap();

        self.players()
            .iter()
            .filter(|p| !team.contains(p))
            .map(|p| p.clone())
            .collect()
    }

    /// Sets the status of the match to complete
    /// Returns Err if the current match status is not LobbyStatus::InProgress
    pub fn finish_match(mut self) -> Lobby<Complete> {
        let teams = self.teams();
        let mut rng = rand::rng();
        // the team with the highest total SR (not mmr) will win the match, for now
        // SAFETY:
        //     The number is acquired from iterating the team list and is guaranteed to be valid
        let tn = unsafe {
            TeamNumber::new_unchecked(
                teams
                    .iter()
                    .map(|t| {
                        t.iter()
                            .map(|p| {
                                let normal = Normal::new(0.0, p.consistency()).unwrap();
                                let offset = normal.sample(&mut rng);
                                p.sr() + offset
                            })
                            .sum::<f64>()
                    })
                    .enumerate()
                    .max_by_key(|&(_, value)| value as usize)
                    .map(|(i, _)| i)
                    .unwrap(),
            )
        };
        self.status = LobbyStatus::Complete(tn);

        // SAFETY:
        //     The marker is zero sized so this won't affect the memory layout
        unsafe { std::mem::transmute::<Lobby<InProgress>, Lobby<Complete>>(self) }
    }
}

impl Lobby<Complete> {
    pub fn teams(&self) -> [[Player; TEAM_SIZE]; TEAM_COUNT] {
        // SAFETY:
        //     Just for code reuse in the typestate pattern. Reinterpreting the reference just for
        //     this function shouldn't affect anything
        let s = unsafe { std::mem::transmute::<&Lobby<Complete>, &Lobby<InProgress>>(self) };
        match s.status {
            LobbyStatus::InProgress => self.teams.map(|t| t.map(|p| p.unwrap())),
            _ => unreachable!(),
        }
    }

    pub fn players(&self) -> Vec<Player> {
        // SAFETY:
        //     Just for code reuse in the typestate pattern. Reinterpreting the reference just for
        //     this function shouldn't affect anything
        let s = unsafe { std::mem::transmute::<&Lobby<Complete>, &Lobby<InProgress>>(self) };
        match s.status {
            LobbyStatus::WaitingForPlayers(ref players) => players.to_vec(),
            _ => self
                .teams
                .as_flattened()
                .iter()
                .map(|p| p.unwrap())
                .collect(),
        }
    }

    pub fn get_result(&self) -> usize {
        match self.status {
            LobbyStatus::Complete(team_number) => *team_number,
            _ => unreachable!(),
        }
    }

    /// Returns None if the game has not yet ended
    pub fn did_player_win(&self, player: &Player) -> bool {
        let result = self.get_result();
        let winning_team = self.teams()[result];
        winning_team.contains(player)
    }
}

impl<T: LobbyStatusMarker> Lobby<T> {
    pub fn range(&self) -> f64 {
        let mut min = f64::MAX;
        let mut max = 0.0;
        let players = match self.status {
            LobbyStatus::WaitingForPlayers(_) => {
                let s =
                    unsafe { std::mem::transmute::<&Lobby<T>, &Lobby<WaitingForPlayers>>(self) };
                s.players()
            }
            _ => {
                let s = unsafe { std::mem::transmute::<&Lobby<T>, &Lobby<InProgress>>(self) };
                s.players()
            }
        };

        for p in players.iter() {
            if p.rating() < min {
                min = p.rating();
            }

            if p.rating() > max {
                max = p.rating();
            }
        }

        max - min
    }
}

pub fn make_matches(mut commands: Commands, mut queue: ResMut<Queue>) {
    let mut rng = rand::rng();
    let lobbies = queue.make_matches();
    for lobby in lobbies {
        let lobby = Lobby::new(lobby);
        // let range = m.range();
        let duration = rng.random_range(10..60);
        commands.spawn((lobby, TickTimer::new(duration * 60, TimerMode::Once)));
    }
}

pub fn end_matches(
    mut commands: Commands,
    mut queue: ResMut<Queue>,
    mut match_stats: ResMut<MatchStats>,
    mut world: World,
) {
    let mut matches_in_progress =
        world.query_filtered::<(Entity, &mut TickTimer), With<Lobby<InProgress>>>();
    let mip = matches_in_progress.iter(&mut world).len();
    let player_count = (mip * MATCH_PLAYER_COUNT) + queue.len();

    for (e, mut timer) in matches_in_progress.iter_mut(&mut world) {
        if timer.just_finished() {
            let mut lobby_ent = world.entity_mut(e);
            let Some(mut lobby) = lobby_ent.take::<Lobby<InProgress>>() else {
                unreachable!();
            };
            match_stats.matches_played += 1;
            let lobby = lobby.finish_match();

            let players = lobby.teams();
            for team in players {
                for mut player in team {
                    let should_requeue = player.finished_match(lobby.as_ref(), player_count);
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
