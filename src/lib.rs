#![allow(unused_imports)]
#![allow(clippy::too_many_arguments)]

pub mod queue; 
pub mod display;
pub mod match_in_progress;
pub mod player;
pub mod glicko;
pub mod ring_buffer;
pub mod player_management;
pub mod fs;

use bevy::prelude::*;

/// If the player is unrated, set the rating to 1500 and the RD (aka the standard deviation) to
/// 350.
pub const MEAN_MMR: f32 = 1500.0;
pub const MAX_MMR: f32 = 20_000.0;
pub const STD_DEV: f32 = 350.0;
pub const DEFAULT_DEVIATION: f32 = STD_DEV;
/// Unrated players should get volatility of 0.06, but this value may be adjusted based on the
/// application
pub const DEFAULT_VOLATILITY: f32 = 0.06;
/// The time measured in rating periods before the player's deviation returns to the default
pub const TIME_TO_RESET: f32 = 100.;

/// Constrains the change in volatility over time
/// Reasonable choices are between 0.3 and 1.2
/// Small values prevent volatility measures from changing by large amounts
///     -> Thus, the system is more stable with small values
pub const VOLATILITY_CONSTRAINT: f32 = 0.3;

/// Convergence tolerance relevant for Glicko-2 calculations
pub const CONVERGENCE_TOLERANCE: f32 = 0.000001;

pub const TEAM_SIZE: usize = 5;
pub const TEAM_COUNT: usize = 2;
pub const MATCH_PLAYER_COUNT: usize = TEAM_SIZE * TEAM_COUNT;

/// Converts a tick count into a number of rating periods
const fn ticks_to_rating_periods(ticks: usize) -> f32 {
    // One tick is equivalet to one simulated second
    // One match lasts between 10 and 60 minutes or 600 to 3600 seconds aka ticks
    // Average match length is therefore (3600 + 600) / 2, or 2100 seconds/ticks
    // Glicko-2 recommends at least 10-15 games per rating period
    ticks as f32 / 2100.0 * 15.0
}

#[derive(Component, Default)]
pub struct TickTimer {
    duration: usize,
    elapsed: usize,
    finished: bool,
    times_finished_since_last_check: usize,
    mode: TimerMode,
}

impl TickTimer {
    pub fn new(duration: usize, mode: TimerMode) -> Self {
        Self {
            duration,
            mode,
            ..Default::default()
        }
    }

    /// Returns true if the timer is currently finished
    #[inline]
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Returns true if the timer has finished at least once since it was last checked
    #[inline]
    pub fn just_finished(&mut self) -> bool {
        let res = self.times_finished_since_last_check > 0;
        self.times_finished_since_last_check = 0;
        res
    }

    #[inline]
    pub fn elapsed(&self) -> usize {
        self.elapsed
    }

    #[inline]
    pub fn duration(&self) -> usize {
        self.duration
    }

    #[inline]
    pub fn mode(&self) -> TimerMode {
        self.mode
    }

    pub fn tick(&mut self) -> &Self {
        if self.mode != TimerMode::Repeating && self.is_finished() {
            return self;
        }

        self.elapsed += 1;
        self.finished = self.elapsed() >= self.duration();

        if self.is_finished() {
            if self.mode == TimerMode::Repeating {
                self.elapsed = 0;
                self.times_finished_since_last_check += 1;
            } else {
                self.times_finished_since_last_check += 1;
            }
        } else {
            self.times_finished_since_last_check = 0;
        }

        self
    }
}
