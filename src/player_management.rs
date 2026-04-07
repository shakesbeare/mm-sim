use std::cell::RefCell;

use bevy::{ecs::query::QueryData, prelude::*};
use rand::{Rng as _, seq::IteratorRandom as _, seq::SliceRandom as _};
use rand_distr::{Distribution as _, weighted::WeightedIndex};

use crate::{
    MatchStats,
    lobby::{InProgress, Lobby, WaitingForPlayers},
    player::{Any, InLobby, InQueue, IntoPlayerList, IntoPlayerListMut, LoggedOut, Player},
    time::SimTime,
};

pub const TARGET_PLAYER_COUNT: usize = 5000;
pub const SOFT_MAX_PLAYERS: usize = TARGET_PLAYER_COUNT / 3;

#[derive(
    Resource, Default, Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Deref, DerefMut,
)]
pub struct PlayerCount(usize);

#[derive(QueryData)]
pub struct PlayerQuery {
    playing: Option<&'static Player<InLobby>>,
    queued: Option<&'static Player<InQueue>>,
    logged_out: Option<&'static Player<LoggedOut>>,
    waiting_for_players: Option<&'static Lobby<WaitingForPlayers>>,
    in_progress: Option<&'static Lobby<InProgress>>,
}

pub trait FlattenPlayerQuery<'w> {
    fn flatten(self) -> Vec<&'w Player<Any>>;
}

impl<'w, 's> FlattenPlayerQuery<'w> for Query<'w, 's, PlayerQuery> {
    fn flatten(self) -> Vec<&'w Player<Any>> {
        let mut out = Vec::new();

        for pq in self {
            if let Some(playing) = pq.playing {
                out.push(playing.as_any());
            }

            if let Some(queued) = pq.queued {
                out.push(queued.as_any());
            }

            if let Some(logged_out) = pq.logged_out {
                out.push(logged_out.as_any());
            }

            if let Some(w) = pq.waiting_for_players {
                for p in w.players().iter().filter(|p| p.is_some()) {
                    out.push(p.as_ref().unwrap().as_any());
                }
            }

            if let Some(ip) = pq.in_progress {
                for p in ip.players() {
                    out.push(p.as_any());
                }
            }
        }

        out
    }
}

pub fn chance_to_add(player_count: usize) -> f32 {
    let start = 0.90;
    let end = 0.10;
    let t = player_count as f32 / SOFT_MAX_PLAYERS as f32;

    return start + t * (end - start);
}

pub fn try_add_player(world: &mut World) {
    let world = RefCell::new(world);
    let player_count = {
        let world = world.borrow();
        let pc = world.resource::<PlayerCount>();
        pc.0
    };

    let mut rng = rand::rng();
    let attempt = rng.random_range(0.0..1.0);

    if attempt <= chance_to_add(player_count) {
        let mut world = world.borrow_mut();
        let offset = rng.random_range(0..24);
        let new_player = Player::new(None, None, None, None, Some(offset));
        world.spawn(new_player);
        return;
    }
}

pub fn unfrustrated_players(world: &mut World) {
    let mut rng = rand::rng();
    let mut logged_out_players = world.query::<(Entity, &mut Player<LoggedOut>)>();
    let mut to_return = Vec::new();
    let sim_time = world.resource::<SimTime>();

    for (e, p) in logged_out_players.iter(world) {
        let attempt = rng.random_range(0.0..1.0);
        let datetime = sim_time.datetime_from_offset(p.offset());
        if p.frustration() <= 0.0 && attempt < datetime.likelihood_to_play() {
            to_return.push(e);
        }
    }

    for e in to_return {
        let mut ent = world.entity_mut(e);
        let p = ent.take::<Player<LoggedOut>>().unwrap().login();
        ent.insert(p);
    }
}

pub fn frustrated_players(world: &mut World) {
    let mut rng = rand::rng();
    let mut logouts = Vec::new();
    let mut lobbies = world.query::<&mut Lobby<WaitingForPlayers>>();
    let sim_time = world.resource::<SimTime>().clone();
    for mut lobby in lobbies.iter_mut(world) {
        let mut mark_sweep = Vec::new();

        for (i, p) in lobby.players().iter().enumerate().filter_map(|(i, p)| {
            if p.is_some() {
                Some((i, p.unwrap()))
            } else {
                None
            }
        }) {
            let attempt = rng.random_range(0.0..1.0);
            let datetime = sim_time.datetime_from_offset(p.offset());
            if p.frustration() > 3600.0 * 5.0 || attempt >= datetime.likelihood_to_play() {
                mark_sweep.push(i);
            }
        }

        for i in mark_sweep.iter() {
            let players = lobby.players_mut();
            let p = players[*i]
                .take()
                .unwrap_or_else(|| {
                    println!("{:?}", mark_sweep);
                    println!("{:?}", players[*i]);
                    println!("{:?}", players);
                    panic!(
                        "Attempted to logout player {} but that player does not exist",
                        i
                    );
                })
                .logout();
            logouts.push(p);
        }
    }

    for p in logouts {
        world.spawn(p);
    }
}

pub fn give_up_queue(world: &mut World) {
    let mut count = 0;
    let mut logouts = Vec::new();
    let mut lobbies = world.query::<&mut Lobby<WaitingForPlayers>>();
    for mut lobby in lobbies.iter_mut(world) {
        let mut mark_sweep = Vec::new();
        for (i, p) in lobby.players().iter().filter_map(|p| *p).enumerate() {
            if p.queue_stats().wait_time().unwrap() > 1200 {
                mark_sweep.push(i);
            }
        }

        for i in mark_sweep {
            count += 1;
            let players = lobby.players_mut();
            let p = players[i].take().unwrap().logout();
            logouts.push(p);
        }
    }

    for p in logouts {
        world.spawn(p);
    }

    let mut match_stats = world.resource_mut::<MatchStats>();
    match_stats.gave_up += count;
}

/// If the total player count breaches HARD_MAX_PLAYERS, kill logged out players until the cap is
/// respected.
///
/// Players with higher MMR and lower frustration are less likely to be chosen
pub fn kill_player(
    player_count: Query<PlayerQuery>,
    players: Query<(Entity, &Player<LoggedOut>)>,
    mut commands: Commands,
) {
    let player_count = player_count.flatten().len();
    if player_count <= TARGET_PLAYER_COUNT {
        return;
    }
    let mut rng = rand::rng();
    let weights: Vec<f64> = players
        .iter()
        .map(|(_, p)| p.frustration() / 1.0 + p.rating() + p.matches_played().pow(2) as f64)
        .collect();
    let dist = WeightedIndex::new(&weights).unwrap_or_else(|_| {
        panic!("Invalid weight!");
    });

    for _ in 0..(player_count - TARGET_PLAYER_COUNT) {
        let index = dist.sample(&mut rng);
        let (e, _) = players.iter().nth(index).unwrap();
        commands.entity(e).despawn();
    }
}
