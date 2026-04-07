use std::cell::RefCell;

use bevy::prelude::*;
use rand::{Rng as _, seq::IteratorRandom as _};

use crate::{
    MatchStats,
    lobby::{InProgress, Lobby, WaitingForPlayers},
    player::{InLobby, InQueue, LoggedOut, Player},
    time::SimTime,
};

pub const STARTING_PLAYER_COUNT: usize = 7729 / 4;
pub const SOFT_MAX_PLAYERS: usize = STARTING_PLAYER_COUNT * 2;

pub fn chance_to_add(player_count: usize) -> f32 {
    let start = 0.99;
    let end = 0.01;
    let t = player_count as f32 / SOFT_MAX_PLAYERS as f32;

    return start + t * (end - start);
}

pub fn try_add_player(world: &mut World) {
    let world = RefCell::new(world);

    let mut players_waiting_query = world.borrow_mut().query::<&Lobby<WaitingForPlayers>>();
    let waiting_count = players_waiting_query
        .iter(&world.borrow())
        .flat_map(|l| l.players())
        .filter_map(|p| *p)
        .count();
    let mut players_playing_query = world.borrow_mut().query::<&Lobby<InProgress>>();
    let playing_count = players_playing_query
        .iter(&world.borrow())
        .flat_map(|l| l.players())
        .count();

    let mut player_query = world
        .borrow_mut()
        .query_filtered::<Entity, Or<(With<Player<InLobby>>, With<Player<InQueue>>)>>();
    let player_count = player_query.iter(&world.borrow()).len() + waiting_count + playing_count;

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
