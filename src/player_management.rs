use bevy::prelude::*;
use rand::{Rng as _, seq::IteratorRandom as _};

use crate::{r#match::Match, player::Player, queue::Queue};

pub const STARTING_PLAYER_COUNT: usize = 4155; // Half of peak players of smite 2 in last 24-hours
pub const SOFT_MAX_PLAYERS: usize = STARTING_PLAYER_COUNT * 6;

pub fn chance_to_quit(player_count: usize) -> f32 {
    let start = 0.05;
    let end = 0.95;
    let t = player_count as f32 / SOFT_MAX_PLAYERS as f32;

    return start + t * (end - start);
}

pub fn chance_to_add(player_count: usize) -> f32 {
    let start = 1.0;
    let end = 0.0;
    let t = player_count as f32 / SOFT_MAX_PLAYERS as f32;

    return start + t * (end - start);
}

pub fn try_add_player(mut commands: Commands, mut queue: ResMut<Queue>, mip: Query<&Match>, logged_out_players: Query<(Entity, &Player)>) {
    let player_count = mip.iter().flat_map(|m| m.players()).count() + queue.len();
    let mut rng = rand::rng();
    let attempt = rng.random_range(0.0..1.0);

    if attempt <= chance_to_add(player_count) {
        let Some((e, player)) = logged_out_players.iter().choose(&mut rng) else {
            queue.insert(Player::new(None, None, None, None));
            return
        };

        // Despawning the player logs them in!
        commands.entity(e).despawn();

        // Queue them up!
        queue.insert(*player);
    }
}
