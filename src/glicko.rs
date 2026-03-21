//! https://www.glicko.net/glicko/glicko2.pdf

use core::f32;

use bevy::math::ops::{ln, sqrt};

use crate::{match_in_progress::MatchInProgress, player::Player};

/// Converts a Glicko rating to the Glicko-2 scale
pub const fn rating_glicko_to_glicko2(rating: f32) -> f32 {
    (rating - crate::MEAN_MMR) / 173.7178 // TODO: this constant might be based on the mean
    // mmr
}

/// Converts a Glicko RD to the Glicko-2 scale
pub const fn rd_glicko_to_glicko2(rd: f32) -> f32 {
    rd / 173.7178
}

pub const fn rating_glicko2_to_glicko(rating: f32) -> f32 {
    173.7178 * rating + 1500.0
}

pub const fn rd_glicko2_to_glicko(rd: f32) -> f32 {
    173.7178 * rd
}

/// Not clear what this represents. It's some intermediary function needed for Glicko-2
pub fn g(phi: f32) -> f32 {
    1.0 / (sqrt(1.0 + 3.0 * phi.powf(2.0) / f32::consts::PI.powf(2.0)))
}

/// Not clear what this represents. It's some intermediary function needed for Glicko-2
#[allow(non_snake_case)]
pub fn E(mu: f32, mu_j: f32, phi_j: f32) -> f32 {
    1.0 / (1.0 + (-g(phi_j) * (mu - mu_j)).exp())
}

pub fn v(player: &Player, matches: &[&MatchInProgress]) -> f32 {
    let mut sum = 0.0;

    for m in matches {
        let enemies = m.enemies_of(player);
        let count_enemies = enemies.len();
        let avg_rating: f32 = enemies
            .iter()
            .map(|p| rating_glicko_to_glicko2(p.mmr()))
            .sum::<f32>()
            / count_enemies as f32;
        let avg_deviation: f32 = enemies
            .iter()
            .map(|p| rd_glicko_to_glicko2(p.rating_deviation()))
            .sum::<f32>()
            / count_enemies as f32;
        let value = g(avg_deviation).powf(2.0)
            * E(
                rating_glicko_to_glicko2(player.mmr()),
                avg_rating,
                avg_deviation,
            )
            * (1.0
                - E(
                    rating_glicko_to_glicko2(player.mmr()),
                    avg_rating,
                    avg_deviation,
                ));
        sum += value;
    }

    1.0 / sum
}

pub fn delta(player: &Player, matches: &[&MatchInProgress]) -> f32 {
    let mut sum = 0.0;

    for m in matches {
        let enemies = m.enemies_of(player);
        let count_enemies = enemies.len();
        let avg_rating: f32 = enemies
            .iter()
            .map(|p| rating_glicko_to_glicko2(p.mmr()))
            .sum::<f32>()
            / count_enemies as f32;
        let avg_deviation: f32 = enemies
            .iter()
            .map(|p| rd_glicko_to_glicko2(p.rating_deviation()))
            .sum::<f32>()
            / count_enemies as f32;
        let score = if m.did_player_win(player).is_some_and(|r| r) {
            1.0
        } else {
            0.0
        };

        let value = g(avg_deviation)
            * (score
                - E(
                    rating_glicko_to_glicko2(player.mmr()),
                    avg_rating,
                    avg_deviation,
                ));
        sum += value;
    }

    sum * v(player, matches)
}

pub fn f(x: f32, player: &Player, matches: &[&MatchInProgress]) -> f32 {
    let a = ln(player.volatility().powf(2.0));
    (f32::consts::E.powf(x)
        * (delta(player, matches).powf(2.0)
            - rd_glicko_to_glicko2(player.rating_deviation()).powf(2.0)
            - v(player, matches)
            - f32::consts::E.powf(x))
        / 2.0
        * (rd_glicko_to_glicko2(player.rating_deviation()).powf(2.0)
            + v(player, matches)
            + f32::consts::E.powf(x))
        .powf(2.0))
        - (x - a) / crate::DEFAULT_VOLATILITY.powf(2.0)
}

#[allow(non_snake_case)]
pub fn new_volatility(player: &Player, matches: &[&MatchInProgress]) -> f32 {
    let a = ln(player.volatility().powf(2.0));
    let mut A = a;
    let mut B = {
        if delta(player, matches).powf(2.0)
            > rd_glicko_to_glicko2(player.rating_deviation()).powf(2.0) + v(player, matches)
        {
            ln(delta(player, matches).powf(2.0)
                - rd_glicko_to_glicko2(player.rating_deviation()).powf(2.0)
                - v(player, matches))
        } else {
            let mut k = 1;
            while f(a - k as f32 * crate::DEFAULT_VOLATILITY, player, matches) < 0.0 {
                k += 1;
            }

            a - k as f32 * crate::DEFAULT_VOLATILITY
        }
    };
    let mut fA = f(A, player, matches);
    let mut fB = f(B, player, matches);
    let mut prev_A = A;
    let mut prev_B = B;

    while (B - A).abs() > crate::CONVERGENCE_TOLERANCE {
        // dbg!((B - A).abs());
        // dbg!(crate::CONVERGENCE_TOLERANCE);
        let C = A + (A - B) * fA / (fB - fA);
        let fC = f(C, player, matches);
        if fC * fB <= 0.0 {
            A = B;
            fA = fB;
        } else {
            fA /= 2.0;
        }
        B = C;
        if A == prev_A && B == prev_B {
            // it looks like we got locked in the while loop, just return the old volatility
            return player.volatility();
        }
        prev_A = A;
        prev_B = B;
        fB = fC;
    }

    return f32::consts::E.powf(A / 2.0);
}

pub fn temp_rd(player: &Player, volatility: f32) -> f32 {
    sqrt(rating_glicko_to_glicko2(player.mmr()).powf(2.0) + volatility.powf(2.0))
}

pub fn new_rd(player: &Player, matches: &[&MatchInProgress], temp_rd: f32) -> f32 {
    1.0 / sqrt(1.0 / temp_rd.powf(2.0) + 1.0 / v(player, matches))
}

pub fn new_mmr(player: &Player, matches: &[&MatchInProgress], new_rd: f32) -> f32 {
    let mut sum = 0.0;

    let phi_squared = new_rd.powf(2.0);

    for m in matches {
        let enemies = m.enemies_of(player);
        let count_enemies = enemies.len();
        let avg_rating: f32 = enemies
            .iter()
            .map(|p| rating_glicko_to_glicko2(p.mmr()))
            .sum::<f32>()
            / count_enemies as f32;
        let avg_deviation: f32 = enemies
            .iter()
            .map(|p| rd_glicko_to_glicko2(p.rating_deviation()))
            .sum::<f32>()
            / count_enemies as f32;
        let score = if m.did_player_win(player).is_some_and(|r| r) {
            1.0
        } else {
            0.0
        };

        let value = g(avg_deviation)
            * (score
                - E(
                    rating_glicko_to_glicko2(player.mmr()),
                    avg_rating,
                    avg_deviation,
                ));
        sum += value;
    }

    rating_glicko_to_glicko2(player.mmr()) + phi_squared * sum
}
