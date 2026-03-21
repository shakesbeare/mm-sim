use bevy::log::tracing;
use bevy::math::ops::sqrt;
use bevy::prelude::*;
use rand::Rng as _;
use rand_distr::Distribution as _;
use rand_distr::Normal;

use crate::DEFAULT_VOLATILITY;
use crate::MAX_MMR;
use crate::glicko;
use crate::match_in_progress::MatchInProgress;
use crate::{MEAN_MMR, STD_DEV};

#[derive(Component, Copy, Debug, Clone, Default, PartialEq)]
pub struct Player {
    /// How good the player actually is in the simulation
    /// Used for determining the match result
    sr: f32,
    /// How consistently the player performs at their SR
    /// Used for determining the match result
    /// This is used for the standard deviation later when their actual performance is sampled
    consistency: f32,
    /// How good the matchmaker thinks the player is
    /// Used for matchmaking
    mmr: f32,
    /// How many matches a player has played
    matches_played: usize,
    /// How many ticks have elapsed since the player last played a match
    time_since_last_match: usize,
    /// How uncertain the matchmaker is with the performance of a player
    rating_deviation: f32,
    /// How inconsistent the player performs
    volatility: f32,
}

impl Player {
    pub fn new(
        mmr: Option<f32>,
        sr: Option<f32>,
        rd: Option<f32>,
        volatility: Option<f32>,
    ) -> Self {
        let mut rng = rand::rng();
        let normal = Normal::new(MEAN_MMR as f64, STD_DEV as f64).unwrap();

        Self {
            sr: sr.unwrap_or(normal.sample(&mut rng) as f32),
            consistency: rng.random_range(0.0..STD_DEV / 2.0),
            mmr: mmr.unwrap_or(MEAN_MMR),
            matches_played: 0,
            time_since_last_match: usize::MAX,
            rating_deviation: rd.unwrap_or(DEFAULT_VOLATILITY),
            volatility: volatility.unwrap_or(DEFAULT_VOLATILITY),
        }
    }

    pub fn mmr(&self) -> f32 {
        self.mmr
    }

    pub fn sr(&self) -> f32 {
        self.sr
    }

    pub fn consistency(&self) -> f32 {
        self.consistency
    }

    pub fn rating_deviation(&self) -> f32 {
        self.rating_deviation
    }

    pub fn volatility(&self) -> f32 {
        self.volatility
    }

    pub fn matches_played(&self) -> usize {
        self.matches_played
    }

    pub fn update_mm_stats(&mut self, mmr: f32, rating_deviation: f32, volatility: f32) {
        let delta_mmr = mmr - self.mmr;
        if delta_mmr > STD_DEV {
            self.mmr += STD_DEV;
        } else {
            self.mmr = mmr.clamp(0.0, MAX_MMR);
        }
        self.rating_deviation = rating_deviation;
        self.volatility = volatility;
    }

    pub fn update_sr(&mut self, won: bool) {
        let mut rng = rand::rng();
        let try_change_sr: usize = rng.random_range(0..100) + if won { 25 } else { 0 };

        let sr_change_value: f32 = rng.random_range(-30.0..30.0) + if won { 10.0 } else { 0.0 };

        if try_change_sr > 50 {
            self.sr += sr_change_value;
        }
    }

    /// Returns true if the player has decided to keep queuing
    /// Returns false if the player has decided to log off
    pub fn finished_match(&mut self, match_: &MatchInProgress, player_count: usize) -> bool {
        let mut rng = rand::rng();

        let winners = match_.get_result().unwrap();
        let players = match_.teams();
        let won = players[winners].contains(self);

        let new_volatility = glicko::new_volatility(self, &[match_]);
        let new_rd = glicko::new_rd(self, &[match_], glicko::temp_rd(self, new_volatility));
        let new_mmr = glicko::new_mmr(self, &[match_], new_rd);

        let new_rd = glicko::rd_glicko2_to_glicko(new_rd);
        let new_mmr = glicko::rating_glicko2_to_glicko(new_mmr);

        if new_volatility.is_nan() || new_rd.is_nan() || new_mmr.is_nan() {
            tracing::error!("Something went wrong");
            tracing::error!("{}", new_volatility);
            tracing::error!("{}", new_rd);
            tracing::error!("{}", new_mmr);
        }


        self.update_mm_stats(new_mmr, new_rd, new_volatility);
        self.update_sr(won);
        self.matches_played += 1;

        let attempt = rng.random_range(0.0..1.0) + if won {
            -0.25
        } else {
            0.25
        };

        attempt <= crate::player_management::chance_to_quit(player_count)
    }

    /// Increases the player's rating deviation based on inactivity
    pub fn update_deviation_for_inactivity(&mut self) {
        self.rating_deviation = f32::min(
            sqrt(
                self.rating_deviation.powf(2.0)
                    + crate::TIME_TO_RESET.powf(2.0)
                        * crate::ticks_to_rating_periods(self.time_since_last_match),
            ),
            crate::DEFAULT_DEVIATION,
        );
    }

    /// Decreases the player's rating deviation based on a number of games played
    pub fn update_deviation_after_game(&mut self) -> f32 {
        todo!()
    }
}

#[derive(Component, Default, PartialEq)]
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
            self.player.mmr, self.wait_time
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

    pub fn max_acceptable_mmr_range_now(&self) -> f32 {
        300.0 + self.wait_time as f32 * 8.33
    }
}
