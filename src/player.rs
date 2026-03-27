use bevy::log::tracing;
use bevy::math::ops::sqrt;
use bevy::prelude::*;
use rand::Rng as _;
use rand_distr::Distribution as _;
use rand_distr::Normal;
use skillratings::{
    Outcomes,
    glicko2::{Glicko2Config, Glicko2Rating, glicko2},
};

use crate::DEFAULT_VOLATILITY;
use crate::GLICKO_CONFIG;
use crate::MAX_MMR;
use crate::lobby::Lobby;
use crate::{MEAN_MMR, STD_DEV};

#[derive(Component, Copy, Debug, Clone, Default, PartialEq, PartialOrd)]
pub struct Player {
    /// How good the player actually is in the simulation
    /// Used for determining the match result
    sr: f64,
    /// How consistently the player performs at their SR
    /// Used for determining the match result
    /// This is used for the standard deviation later when their actual performance is sampled
    consistency: f64,
    /// How good the matchmaker thinks the player is
    /// Used for matchmaking
    rating: f64,
    /// How many matches a player has played
    matches_played: usize,
    /// How many ticks have elapsed since the player last played a match
    time_since_last_match: usize,
    /// How uncertain the matchmaker is with the performance of a player
    rating_deviation: f64,
    /// How inconsistent the player performs
    volatility: f64,
}

impl Player {
    pub fn new(
        rating: Option<f64>,
        sr: Option<f64>,
        rd: Option<f64>,
        volatility: Option<f64>,
    ) -> Self {
        let mut rng = rand::rng();
        let normal = Normal::new(MEAN_MMR, STD_DEV).unwrap();

        Self {
            sr: sr.unwrap_or(normal.sample(&mut rng)),
            consistency: rng.random_range(0.0..STD_DEV / 2.0),
            rating: rating.unwrap_or(MEAN_MMR),
            matches_played: 0,
            time_since_last_match: usize::MAX,
            rating_deviation: rd.unwrap_or(DEFAULT_VOLATILITY),
            volatility: volatility.unwrap_or(DEFAULT_VOLATILITY),
        }
    }

    pub fn rating(&self) -> f64 {
        self.rating
    }

    pub fn sr(&self) -> f64 {
        self.sr
    }

    pub fn consistency(&self) -> f64 {
        self.consistency
    }

    pub fn rating_deviation(&self) -> f64 {
        self.rating_deviation
    }

    pub fn volatility(&self) -> f64 {
        self.volatility
    }

    pub fn matches_played(&self) -> usize {
        self.matches_played
    }

    pub fn update_mm_stats(&mut self, rating: f64, rating_deviation: f64, volatility: f64) {
        self.rating = rating;
        self.rating_deviation = rating_deviation;
        self.volatility = volatility;
    }

    pub fn update_sr(&mut self, won: bool) {
        let mut rng = rand::rng();
        let try_change_sr: usize = rng.random_range(0..100) + if won { 25 } else { 0 };

        let sr_change_value: f64 = rng.random_range(-30.0..30.0) + if won { 10.0 } else { 0.0 };

        if try_change_sr > 50 {
            self.sr += sr_change_value;
        }
    }

    /// Returns true if the player has decided to keep queuing
    /// Returns false if the player has decided to log off
    /// Will panic if the lobby is not LobbyStatus::Complete
    pub fn finished_match(&mut self, lobby: &Lobby, player_count: usize) -> bool {
        let mut rng = rand::rng();

        let winners = lobby.get_result().unwrap();
        let players = lobby.teams().unwrap();
        let won = players[winners].contains(self);

        let outcome = match won {
            true => Outcomes::WIN,
            false => Outcomes::LOSS,
        };


        let player_glicko = Glicko2Rating { 
            rating: self.rating,
            deviation: self.rating_deviation,
            volatility: self.volatility,

        };

        let enemies_glicko = lobby.glicko_for_enemies_of(self).unwrap();
        let (new_player_glicko, _) = glicko2(&player_glicko, &enemies_glicko, &outcome, &GLICKO_CONFIG);


        self.update_mm_stats(new_player_glicko.rating, new_player_glicko.deviation, new_player_glicko.volatility);
        self.update_sr(won);
        self.matches_played += 1;

        let attempt = rng.random_range(0.0..1.0) + if won { -0.25 } else { 0.25 };

        attempt > crate::player_management::chance_to_quit(player_count)
    }
}

#[derive(Component, Default, PartialEq, Clone, Copy)]
pub struct QueuedPlayer {
    pub wait_time: usize, // in ticks
    pub player: Player,
    pub times_skipped: usize,
    pub times_failed_to_match: usize,
}

impl std::fmt::Debug for QueuedPlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Player(MMR: {}, Wait Time: {})",
            self.player.rating, self.wait_time
        )
    }
}

impl QueuedPlayer {
    pub fn new(player: Player) -> Self {
        Self {
            wait_time: 0,
            player,
            times_skipped: 0,
            times_failed_to_match: 0,
        }
    }

    pub fn tick(&mut self) {
        self.wait_time += 1;
    }

    pub fn max_acceptable_mmr_range_now(&self) -> f64 {
        if self.wait_time < 900 { 
            STD_DEV + 1.01_f64.powf(self.wait_time as f64 / 0.7)
        } else {
            f64::MAX
        }
    }
}
