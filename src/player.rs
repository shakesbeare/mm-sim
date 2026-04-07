use std::marker::PhantomData;

use bevy::log::tracing;
use bevy::math::ops::sqrt;
use bevy::prelude::*;
use rand::Rng as _;
use rand::seq::IndexedRandom as _;
use rand_distr::Distribution as _;
use rand_distr::Normal;
use skillratings::{
    Outcomes,
    glicko2::{Glicko2Config, Glicko2Rating, glicko2},
};

use crate::DEFAULT_VOLATILITY;
use crate::GLICKO_CONFIG;
use crate::MAX_MMR;
use crate::lobby::Complete;
use crate::lobby::Lobby;
use crate::time::MIN_LATENCY;
use crate::{MEAN_MMR, STD_DEV};

pub trait IntoPlayerList: Component {
    fn into(&self) -> Vec<&Player<Any>>;
}

pub trait IntoPlayerListMut: Component {
    fn into(&mut self) -> Vec<&mut Player<Any>>;
}

mod private {
    pub trait PlayerStatus {}
}

#[derive(Component, Copy, Debug, Clone, Default, PartialEq, PartialOrd)]
pub struct LoggedOut;
impl private::PlayerStatus for LoggedOut {}

#[derive(Component, Copy, Debug, Clone, Default, PartialEq, PartialOrd)]
pub struct InQueue;
impl private::PlayerStatus for InQueue {}

#[derive(Component, Copy, Debug, Clone, Default, PartialEq, PartialOrd)]
pub struct InLobby;
impl private::PlayerStatus for InLobby {}

#[derive(Component, Copy, Debug, Clone, Default, PartialEq, PartialOrd)]
pub struct Any;
impl private::PlayerStatus for Any {}

#[derive(Component, Copy, Debug, Clone, PartialEq, PartialOrd)]
pub struct Player<T: private::PlayerStatus> {
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
    /// How fast this player learns
    learning_mult: f64,
    /// How many matches a player has played
    matches_played: usize,
    /// How many ticks have elapsed since the player last played a match
    time_since_last_match: usize,
    /// How uncertain the matchmaker is with the performance of a player
    rating_deviation: f64,
    /// How inconsistent the player performs
    volatility: f64,
    /// The queue stats for the player
    queue_stats: QueueStats,
    /// 1.0 frustration is equivalent to a 1-second delay in returning to queue
    frustration: f64,
    /// The timezone this player resides in stored as an offset from base time
    /// Should be between 0 and 23 inclusive
    offset: usize,
    _marker: PhantomData<T>,
}

impl Default for Player<InQueue> {
    fn default() -> Self {
        Self::new(None, None, None, None, None)
    }
}

impl Default for Player<InLobby> {
    fn default() -> Self {
        Self {
            sr: Default::default(),
            consistency: Default::default(),
            rating: Default::default(),
            learning_mult: Default::default(),
            matches_played: Default::default(),
            time_since_last_match: Default::default(),
            rating_deviation: Default::default(),
            volatility: Default::default(),
            queue_stats: Default::default(),
            frustration: Default::default(),
            offset: Default::default(),
            _marker: Default::default(),
        }
    }
}

impl Player<InQueue> {
    pub fn new(
        rating: Option<f64>,
        sr: Option<f64>,
        rd: Option<f64>,
        volatility: Option<f64>,
        offset: Option<usize>,
    ) -> Self {
        let mut rng = rand::rng();
        Self {
            sr: sr.unwrap_or_else(|| {
                let normal = Normal::new(MEAN_MMR, STD_DEV).unwrap();
                normal.sample(&mut rng)
            }),
            consistency: rng.random_range(0.0..STD_DEV),
            learning_mult: rng.random_range(0.1..5.0),
            rating: rating.unwrap_or(MEAN_MMR),
            matches_played: 0,
            time_since_last_match: usize::MAX,
            rating_deviation: rd.unwrap_or(DEFAULT_VOLATILITY),
            volatility: volatility.unwrap_or(DEFAULT_VOLATILITY),
            queue_stats: QueueStats {
                wait_time: Some(0),
                max_wait_time: 0,
            },
            frustration: 0.0,
            offset: offset.unwrap_or(rng.random_range(0..24)),
            _marker: PhantomData,
        }
    }

    pub fn new_wide() -> Self {
        let mut rng = rand::rng();
        let normal = Normal::new(MEAN_MMR, STD_DEV * 2.0).unwrap();
        let sr = normal.sample(&mut rng);
        Self::new(None, Some(sr), None, None, None)
    }

    pub fn new_narrow() -> Self {
        let mut rng = rand::rng();
        let normal = Normal::new(MEAN_MMR, STD_DEV / 2.0).unwrap();
        let sr = normal.sample(&mut rng);
        Self::new(None, Some(sr), None, None, None)
    }

    pub fn new_beginner() -> Self {
        let mut rng = rand::rng();
        let normal = Normal::new(MEAN_MMR / 2.0, STD_DEV).unwrap();
        let sr = normal.sample(&mut rng);
        Self::new(None, Some(sr), None, None, None)
    }

    pub fn new_smurf() -> Self {
        let mut rng = rand::rng();
        let normal = Normal::new(MEAN_MMR * 2.0, STD_DEV).unwrap();
        let sr = normal.sample(&mut rng);
        Self::new(None, Some(sr), None, None, None)
    }

    pub fn new_random_archetype() -> Self {
        let mut rng = rand::rng();
        let choices = [
            Self::new_wide,
            Self::new_narrow,
            Self::new_beginner,
            Self::new_smurf,
        ];
        let archetype = choices.choose(&mut rng).unwrap();
        archetype()
    }

    pub fn max_rating_range(&self) -> f64 {
        let wait_time = self.queue_stats.wait_time.unwrap();
        if wait_time < 900 {
            STD_DEV + 1.01_f64.powf(wait_time as f64 / 0.7)
        } else {
            f64::MAX
        }
    }

    pub fn max_latency_range(&self) -> usize {
        let wait_time = self.queue_stats.wait_time.unwrap();
        MIN_LATENCY + wait_time
        // 300
    }

    pub fn join_lobby(mut self) -> Player<InLobby> {
        self.queue_stats.wait_time = None;
        unsafe { std::mem::transmute::<Self, Player<InLobby>>(self) }
    }

    pub fn tick(&mut self) {
        if let Some(v) = self.queue_stats.wait_time.as_mut() {
            *v += 1;
        }

        if self.queue_stats.wait_time.unwrap_or(0) > 300 {
            self.frustration += 0.001;
        }
    }
}

impl Player<InLobby> {
    /// Consumes the Player, updating their stats and returning Player<InQueue> for that player
    pub fn finished_match(mut self, lobby: &Lobby<Complete>) -> Player<InQueue> {
        let winners = lobby.get_result();
        let players = lobby.teams();
        let won = players[winners].contains(&self);

        let outcome = match won {
            true => Outcomes::WIN,
            false => Outcomes::LOSS,
        };

        let player_glicko = Glicko2Rating {
            rating: self.rating,
            deviation: self.rating_deviation,
            volatility: self.volatility,
        };

        let enemies_glicko = lobby.glicko_for_enemies_of(&self);
        let (new_player_glicko, _) =
            glicko2(&player_glicko, &enemies_glicko, &outcome, &GLICKO_CONFIG);

        self.update_mm_stats(
            new_player_glicko.rating,
            new_player_glicko.deviation,
            new_player_glicko.volatility,
        );
        self.update_sr(won);
        self.matches_played += 1;

        self.queue_stats.wait_time = Some(0);
        let mut rng = rand::rng();
        let normal = Normal::new(0.0, 5.0 * 60.0).unwrap();
        let frustration_jitter: f64 = f64::abs(normal.sample(&mut rng));
        if !won {
            self.frustration += 20.0 * frustration_jitter;
        } else {
            self.frustration += frustration_jitter;
        }
        unsafe { std::mem::transmute::<Self, Player<InQueue>>(self) }
    }
}

impl Player<LoggedOut> {
    pub fn tick(&mut self) {
        self.frustration -= 1.0;
    }

    pub fn login(mut self) -> Player<InQueue> {
        self.queue_stats.wait_time = Some(0);
        unsafe { std::mem::transmute::<Self, Player<InQueue>>(self) }
    }
}

impl<T: private::PlayerStatus> Player<T> {
    #[inline]
    pub fn offset(&self) -> usize {
        self.offset
    }

    #[inline]
    pub fn frustration(&self) -> f64 {
        self.frustration
    }

    #[inline]
    pub fn as_any(&self) -> &Player<Any> {
        unsafe { std::mem::transmute::<&Self, &Player<Any>>(self) }
    }

    #[inline]
    pub fn as_any_mut(&mut self) -> &mut Player<Any> {
        unsafe { std::mem::transmute::<&mut Self, &mut Player<Any>>(self) }
    }

    #[inline]
    pub fn into_any(self) -> Player<Any> {
        unsafe { std::mem::transmute::<Self, Player<Any>>(self) }
    }

    #[inline]
    pub fn queue_stats(&self) -> QueueStats {
        self.queue_stats
    }

    #[inline]
    pub fn rating(&self) -> f64 {
        self.rating
    }

    #[inline]
    pub fn sr(&self) -> f64 {
        self.sr
    }

    #[inline]
    pub fn consistency(&self) -> f64 {
        self.consistency
    }

    #[inline]
    pub fn rating_deviation(&self) -> f64 {
        self.rating_deviation
    }

    #[inline]
    pub fn volatility(&self) -> f64 {
        self.volatility
    }

    #[inline]
    pub fn matches_played(&self) -> usize {
        self.matches_played
    }

    #[inline]
    pub fn update_mm_stats(&mut self, rating: f64, rating_deviation: f64, volatility: f64) {
        self.rating = rating;
        self.rating_deviation = rating_deviation;
        self.volatility = volatility;
    }

    pub fn update_sr(&mut self, won: bool) {
        let mut rng = rand::rng();
        let try_change_sr: usize = rng.random_range(0..100) + if won { 25 } else { 0 };

        let sr_change_value: f64 =
            rng.random_range(-50.0..50.0) + if !won { 20.0 * self.learning_mult } else { 0.0 };

        if try_change_sr > 50 {
            self.sr += sr_change_value;
        }
    }

    #[inline]
    pub fn range_to(&self, other: &Player<Any>) -> f64 {
        f64::max(self.rating(), other.rating()) - f64::min(self.rating(), other.rating())
    }

    pub fn logout(mut self) -> Player<LoggedOut> {
        let mut rng = rand::rng();
        let normal = Normal::new(0.0, 24.0 * 60.0).unwrap();
        let jitter = f64::abs(normal.sample(&mut rng));
        self.frustration += jitter;
        unsafe { std::mem::transmute::<Player<T>, Player<LoggedOut>>(self) }
    }
}

impl<T> IntoPlayerList for Player<T>
where
    T: Send + Sync + private::PlayerStatus + Component,
{
    fn into(&self) -> Vec<&Player<Any>> {
        vec![self.as_any()]
    }
}

impl<T> IntoPlayerListMut for Player<T>
where
    T: Send + Sync + private::PlayerStatus + Component,
{
    fn into(&mut self) -> Vec<&mut Player<Any>> {
        vec![self.as_any_mut()]
    }
}

#[derive(Copy, Debug, Clone, Default, PartialEq, PartialOrd)]
pub struct QueueStats {
    wait_time: Option<usize>,
    max_wait_time: usize,
}

impl QueueStats {
    pub fn wait_time(&self) -> Option<usize> {
        self.wait_time
    }

    pub fn max_wait_time(&self) -> usize {
        self.max_wait_time
    }
}
