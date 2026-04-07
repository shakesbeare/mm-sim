use std::cell::RefCell;
use std::marker::PhantomData;

use bevy::prelude::*;
use extra_collections::arr::Arr;
use rand::Rng as _;
use rand_distr::Distribution as _;
use rand_distr::Normal;
use skillratings::glicko2::Glicko2Rating;
use std::any::TypeId;

use crate::MatchStats;
use crate::TickTimer;
use crate::lobby::private::*;
use crate::player::InLobby;
use crate::player::InQueue;
use crate::player::IntoPlayerList;
use crate::player::IntoPlayerListMut;
use crate::player::Player;
use crate::time::latency_between;
use crate::{MATCH_PLAYER_COUNT, TEAM_COUNT, TEAM_SIZE};

mod private {
    pub trait LobbyStatusMarker {}
}

#[derive(Component, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct WaitingForPlayers;
impl LobbyStatusMarker for WaitingForPlayers {}

#[derive(Component, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct InProgress;
impl LobbyStatusMarker for InProgress {}

#[derive(Component, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct Complete;
impl LobbyStatusMarker for Complete {}

#[derive(Component, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct TeamNumber(usize);

#[derive(Debug, Default, PartialEq, PartialOrd, Clone, Copy)]
#[repr(C)]
pub struct FinishedMatch {
    teams: [[Player<InLobby>; TEAM_SIZE]; TEAM_COUNT],
    winner: TeamNumber,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum LobbyError {
    #[error("Not enough players to start lobby")]
    NotEnoughPlayers,
}

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

/// Contains extra information which may be used depending on the typestate of the lobby
/// Lobby<T> only guarantees that the associated variant is safe to access
#[derive(Copy, Clone)]
#[repr(C)]
pub union LobbyStatusData {
    /// The players currently waiting in the lobby. This collection is not sorted into teams or in
    /// any particular order. Used in Lobby<WaitingForPlayers>.
    waiting_for_players: [Option<Player<InQueue>>; MATCH_PLAYER_COUNT],
    in_progress: [[Player<InLobby>; TEAM_SIZE]; TEAM_COUNT],
    /// The team number (index) of the winner of the match. Used in Lobby<Complete>
    complete: FinishedMatch,
}

#[derive(Component, Clone)]
pub struct Lobby<T: LobbyStatusMarker> {
    status_data: LobbyStatusData,
    offset: usize,
    _marker: std::marker::PhantomData<T>,
}

impl<T: LobbyStatusMarker> AsRef<Lobby<T>> for Lobby<T> {
    fn as_ref(&self) -> &Lobby<T> {
        self
    }
}

impl Lobby<WaitingForPlayers> {
    /// Create a new lobby containing only the given player
    pub fn create(player: Player<InQueue>) -> Self {
        let mut lobby = Self {
            status_data: LobbyStatusData {
                waiting_for_players: std::array::from_fn(|_| None),
            },
            offset: player.offset(),
            _marker: PhantomData,
        };

        lobby.add_player(player);
        lobby
    }

    /// Return the underlying lobby. Does not allocate
    pub fn players(&self) -> &[Option<Player<InQueue>>; MATCH_PLAYER_COUNT] {
        unsafe { &self.status_data.waiting_for_players }
    }

    /// Return the underlying lobby mutable. Does not allocate
    pub fn players_mut(&mut self) -> &mut [Option<Player<InQueue>>; MATCH_PLAYER_COUNT] {
        unsafe { &mut self.status_data.waiting_for_players }
    }

    pub fn add_player(&mut self, player: Player<InQueue>) {
        let players = unsafe { &mut self.status_data.waiting_for_players };
        let index = players.iter().position(|e| e.is_none()).unwrap();
        players[index] = Some(player);
    }

    /// Attempts to convert the lobby into a Lobby<InProgress>
    pub fn start_game(mut self) -> Result<Lobby<InProgress>, LobbyError> {
        let player_count = self.players().iter().filter(|p| p.is_some()).count();
        if player_count != MATCH_PLAYER_COUNT {
            return Err(LobbyError::NotEnoughPlayers);
        }
        let mut teams: [[Option<Player<InQueue>>; TEAM_SIZE]; TEAM_COUNT] =
            std::array::from_fn(|_| std::array::from_fn(|_| None));

        for i in 0..player_count {
            let slot = i % TEAM_SIZE;
            let team = i % TEAM_COUNT;
            let player = unsafe { self.status_data.waiting_for_players[i] };

            teams[team][slot] = player;
        }

        // Now we have to guarantee that the correct union variant is valid
        self.status_data.in_progress = teams.map(|t| t.map(|p| p.unwrap().join_lobby()));

        // SAFETY:
        //     Markers are a zero sized type, so this shouldn't affect anything
        Ok(unsafe { std::mem::transmute::<Lobby<WaitingForPlayers>, Lobby<InProgress>>(self) })
    }

    pub fn range(&self) -> f64 {
        let mut min = f64::MAX;
        let mut max = 0.0;
        let players = self.players();

        for p in players.iter() {
            if p.is_none() {
                continue;
            }
            let p = p.unwrap();

            if p.rating() < min {
                min = p.rating();
            }

            if p.rating() > max {
                max = p.rating();
            }
        }

        max - min
    }

    pub fn can_accept(&self, audition: &Player<InQueue>) -> bool {
        if latency_between(self.offset, audition.offset()) > audition.max_latency_range() {
            return false;
        }

        self.players().iter().filter_map(|p| *p).all(|p| {
            let combined_range = (p.max_rating_range() + audition.max_rating_range()) / 2.0;
            let actual_range = p.range_to(audition.as_any());
            actual_range <= combined_range
        })
    }

    pub fn can_merge(&self, audition: &Lobby<WaitingForPlayers>) -> bool {
        let self_count = self.players().len();
        let other_count = audition.players().len();
        if self_count + other_count > MATCH_PLAYER_COUNT {
            return false;
        }
        let mut can_merge = true;

        for aud_p in audition.players().iter().filter_map(|p| *p) {
            can_merge = can_merge && self.can_accept(&aud_p);
        }

        can_merge
    }

    pub fn tick(&mut self) {
        let players = unsafe { &mut self.status_data.waiting_for_players };
        for p in players {
            if let Some(p) = p.as_mut() {
                p.tick();
            }
        }
    }
}

impl Lobby<InProgress> {
    // Since Lobby<InProgress> does not have an associated StatusData union variant, it should
    // *never* access self.status_data

    /// Create a new lobby with given players to begin playing
    pub fn new(players: [Player<InLobby>; MATCH_PLAYER_COUNT]) -> Self {
        let mut teams = [[None; TEAM_SIZE]; TEAM_COUNT];
        for (k, player) in players.into_iter().enumerate() {
            let slot = k % TEAM_SIZE;
            let team = k % TEAM_COUNT;

            teams[team][slot] = Some(player);
        }

        // let teams = teams.map(|t| t.map(|p| p.unwrap()));
        Self {
            status_data: LobbyStatusData {
                in_progress: teams.map(|t| t.map(|p| p.unwrap())),
            },
            offset: 0,
            _marker: PhantomData,
        }
    }

    pub fn teams(&self) -> &[[Player<InLobby>; TEAM_SIZE]; TEAM_COUNT] {
        unsafe { &self.status_data.in_progress }
    }

    /// Players are not guaranteed to be in the same order as when they were initially added to the
    /// lobby
    pub fn players(&self) -> &[Player<InLobby>; MATCH_PLAYER_COUNT] {
        unsafe {
            std::mem::transmute::<_, &[Player<InLobby>; MATCH_PLAYER_COUNT]>(
                &self.status_data.in_progress,
            )
        }
    }

    /// Return the underlying lobby mutable. Does not allocate
    pub fn players_mut(&mut self) -> &mut [Player<InLobby>; MATCH_PLAYER_COUNT] {
        unsafe {
            std::mem::transmute::<_, &mut [Player<InLobby>; MATCH_PLAYER_COUNT]>(
                &mut self.status_data.in_progress,
            )
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

    pub fn glicko_for_enemies_of(&self, player: &Player<InLobby>) -> Glicko2Rating {
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
    pub fn enemies_of(&self, player: &Player<InLobby>) -> Vec<&Player<InLobby>> {
        let team = self.teams().iter().find(|t| t.contains(player)).unwrap();

        self.players()
            .iter()
            .filter(|p| !team.contains(p))
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

        // Now, guarantee that the new union variant is valid
        self.status_data.complete = FinishedMatch {
            teams: unsafe { self.status_data.in_progress },
            winner: tn,
        };

        // SAFETY:
        //     The marker is zero sized so this won't affect the memory layout
        unsafe { std::mem::transmute::<Lobby<InProgress>, Lobby<Complete>>(self) }
    }

    pub fn range(&self) -> f64 {
        let mut min = f64::MAX;
        let mut max = 0.0;
        let players = self.players();

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

impl Lobby<Complete> {
    pub fn teams(&self) -> &[[Player<InLobby>; TEAM_SIZE]; TEAM_COUNT] {
        unsafe { &self.status_data.complete.teams }
    }

    pub fn players(&self) -> [&Player<InLobby>; MATCH_PLAYER_COUNT] {
        std::array::from_fn(|i| {
            let slot = i % TEAM_SIZE;
            let team = i % TEAM_COUNT;
            unsafe { &self.status_data.complete.teams[team][slot] }
        })
    }

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

    pub fn glicko_for_enemies_of(&self, player: &Player<InLobby>) -> Glicko2Rating {
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
    pub fn enemies_of(&self, player: &Player<InLobby>) -> Vec<&Player<InLobby>> {
        let team = self.teams().iter().find(|t| t.contains(player)).unwrap();

        self.players()
            .iter()
            .filter(|p| !team.contains(p))
            .copied()
            .collect()
    }

    pub fn get_result(&self) -> usize {
        unsafe { *self.status_data.complete.winner }
    }

    /// Returns None if the game has not yet ended
    pub fn did_player_win(&self, player: &Player<InLobby>) -> bool {
        let result = self.get_result();
        let winning_team = self.teams()[result];
        winning_team.contains(player)
    }

    pub fn range(&self) -> f64 {
        let mut min = f64::MAX;
        let mut max = 0.0;
        let players = self.players();

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

impl<T: LobbyStatusMarker> Lobby<T> {}

impl IntoPlayerList for Lobby<InProgress> {
    fn into(&self) -> Vec<&Player<crate::player::Any>> {
        self.players().iter().map(|p| p.as_any()).collect()
    }
}

impl IntoPlayerListMut for Lobby<InProgress> {
    fn into(&mut self) -> Vec<&mut Player<crate::player::Any>> {
        self.players_mut()
            .iter_mut()
            .map(|p| p.as_any_mut())
            .collect()
    }
}

impl IntoPlayerList for Lobby<WaitingForPlayers> {
    fn into(&self) -> Vec<&Player<crate::player::Any>> {
        self.players()
            .iter()
            .filter(|p| p.is_some())
            .map(|p| p.as_ref().unwrap().as_any())
            .collect()
    }
}

impl IntoPlayerListMut for Lobby<WaitingForPlayers> {
    fn into(&mut self) -> Vec<&mut Player<crate::player::Any>> {
        self.players_mut()
            .iter_mut()
            .filter(|p| p.is_some())
            .map(|p| p.as_mut().unwrap().as_any_mut())
            .collect()
    }
}

// 1) sort all players into lobbies (Lobby<WaitingForPlayers>)
//     a) if a valid lobby exists, put them into that lobby
//     b) if no valid lobby exists, create a new one
//  2) for each lobby, try to merge it with all other lobbies
//
//  If, at any point, a lobby hits MATCH_PLAYER_COUNT players, start the match

// step 1
// step 2
// start the games

pub fn add_players_to_lobbies(
    mut commands: Commands,
    players: Query<(Entity, &Player<InQueue>)>,
    mut lobbies: Query<&mut Lobby<WaitingForPlayers>>,
) {
    'outer: for (pe, p) in players.iter() {
        for mut l in lobbies.iter_mut() {
            if l.players().iter().filter_map(|p| *p).count() == MATCH_PLAYER_COUNT {
                continue;
            }

            if l.can_accept(p) {
                commands.entity(pe).despawn();
                l.add_player(*p);
                continue 'outer;
            }
        }

        // no valid lobbies exist
        let new_lobby = Lobby::create(*p);
        commands.entity(pe).despawn();
        commands.spawn(new_lobby);
    }
}

pub fn merge_lobbies(
    mut commands: Commands,
    mut lobbies: Query<(Entity, &mut Lobby<WaitingForPlayers>)>,
) {
    let mut combinations = lobbies.iter_combinations_mut();
    while let Some([(_, mut l1), (e2, l2)]) = combinations.fetch_next() {
        if l1.can_merge(&l2) {
            for player in l2.players().iter().filter_map(|p| *p) {
                l1.add_player(player);
                commands.entity(e2).despawn();
            }
        }
    }
}

pub fn start_lobbies(
    world: &mut World,
    // mut commands: Commands,
    // lobbies: Query<(Entity, &mut Lobby<WaitingForPlayers>)>,
) {
    let mut todo: Vec<Entity> = Vec::new();
    let mut lobbies_query = world.query::<(Entity, &Lobby<WaitingForPlayers>)>();
    for (e, l) in lobbies_query.iter(world) {
        if l.players().iter().filter_map(|p| *p).count() == MATCH_PLAYER_COUNT {
            todo.push(e);
        }
    }

    let mut rng = rand::rng();
    for e in todo {
        let mut ent = world.entity_mut(e);
        // should be guaranteed
        let lobby = ent.take::<Lobby<WaitingForPlayers>>().unwrap();
        let new_lobby = lobby.start_game().unwrap();
        let duration = {
            let start = 10;
            let end = 60;
            let min_mean_sr: usize = new_lobby
                .teams()
                .iter()
                .map(|t| t.iter().map(|p| p.sr() as usize).sum())
                .min()
                .unwrap();
            let max_mean_sr: usize = new_lobby
                .teams()
                .iter()
                .map(|t| t.iter().map(|p| p.sr() as usize).sum())
                .max()
                .unwrap();
            let t = usize::max((max_mean_sr - min_mean_sr) / max_mean_sr, 1);
            start + (1 / t) * (end - start)
        };
        ent.insert(new_lobby);
        ent.insert(TickTimer::new(duration * 60, TimerMode::Once));
    }
}

pub fn end_matches(world: &mut World) {
    let world = RefCell::new(world);
    let mut matches_in_progress = world
        .borrow_mut()
        .query_filtered::<(Entity, &mut TickTimer), With<Lobby<InProgress>>>();
    let mut lobbies_ready_to_finish = Vec::new();

    for (e, mut timer) in matches_in_progress.iter_mut(&mut world.borrow_mut()) {
        if timer.just_finished() {
            lobbies_ready_to_finish.push(e);
        }
    }

    for e in lobbies_ready_to_finish {
        {
            let mut world = world.borrow_mut();
            let mut match_stats = world.resource_mut::<MatchStats>();
            match_stats.matches_played += 1;
        }

        let lobby = {
            let mut world = world.borrow_mut();
            let mut lobby_ent = world.entity_mut(e);
            let Some(lobby) = lobby_ent.take::<Lobby<InProgress>>() else {
                unreachable!();
            };
            lobby_ent.despawn();
            lobby
        };

        let mut world = world.borrow_mut();
        let lobby = lobby.finish_match();
        let players = lobby.teams();
        for team in players {
            for player in team {
                let player = player.finished_match(lobby.as_ref());
                world.spawn(player);
            }
        }
    }
}
