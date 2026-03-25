use bevy::log::tracing;
use bevy::prelude::*;

use crate::{
    GaveUp, MATCH_PLAYER_COUNT, player::{Player, QueuedPlayer}
};

const WAIT_BEFORE_GIVE_UP: usize = 1200;

#[derive(thiserror::Error, Debug, Clone)]
pub enum MatchmakingFailure {
    #[error("Matchmaking pool does not contain enough players to create a match")]
    NotEnoughPlayers,
    #[error("Failed to remove player from queue: Player not in queue")]
    PlayerNotInQueue,
}

#[derive(Debug, Clone, Eq, PartialEq, Copy)]
pub enum MatchValidityCheckResult {
    /// Contains the last index which is guaranteed to fail. Instructs the matchmaker to move the
    /// start of the window forward to one beyond the provided index.
    InvalidMoveForward(usize),
    /// Contains the index of the player which is guaranteed to fail to make matches. Instructs the
    /// matchmaker to add that player to the skip_list and advance the end of the window forward by
    /// one.
    InvalidAddSkip(usize),
    /// The match is valid. Instructs the matchmaker to move the start of the window to one beyond
    /// the last index of the match.
    Valid,
}

impl MatchValidityCheckResult {
    pub fn is_valid(&self) -> bool {
        matches!(*self, Self::Valid)
    }

    pub fn is_invalid(&self) -> bool {
        !matches!(*self, Self::Valid)
    }
}

#[derive(Resource, Default)]
pub struct Queue {
    queue: Vec<QueuedPlayer>,
    prioritize_high: bool,
}

impl std::fmt::Debug for Queue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Queue: {:?}", self.queue)
    }
}

impl Queue {
    pub fn tick(&mut self, mut commands: Commands, mut gave_up: ResMut<GaveUp>) {
        for player in self.queue.iter_mut() {
            player.tick();
        }
        let logging_out: Vec<Player> = self
            .queue
            .iter()
            .filter(|p| p.wait_time > WAIT_BEFORE_GIVE_UP)
            .map(|p| p.player)
            .collect();
        self.queue.retain(|p| p.wait_time <= WAIT_BEFORE_GIVE_UP);
        for p in logging_out {
            gave_up.0 += 1;
            commands.spawn(p);
        }
    }

    pub fn insert(&mut self, player: Player) {
        let qp = QueuedPlayer::new(player);

        if self.queue.is_empty() {
            self.queue.push(qp);
            return;
        }

        for (i, v) in self.queue.iter().enumerate() {
            if qp.player.rating() < v.player.rating() {
                self.queue.insert(i, qp);
                return;
            }
        }

        self.queue.push(qp);
    }

    pub fn remove(&mut self, player: &Player) -> anyhow::Result<QueuedPlayer> {
        let Some((index, _)) = self
            .queue
            .iter()
            .enumerate()
            .find(|(_k, v)| v.player == *player)
        else {
            anyhow::bail!(MatchmakingFailure::PlayerNotInQueue)
        };

        let qp = self.queue.remove(index);

        Ok(qp)
    }

    pub fn get(&self, index: usize) -> Option<&QueuedPlayer> {
        self.queue.get(index)
    }

    pub fn mmr_at(&self, index: usize) -> Option<f64> {
        Some(self.queue.get(index)?.player.rating())
    }

    pub fn sr_at(&self, index: usize) -> Option<f64> {
        Some(self.queue.get(index)?.player.sr())
    }

    /// Determines if a match would be allowed between the two indicies
    /// Returns None if at least one of the indicies does not contain a player
    pub fn matching_allowed_between(&self, left: usize, right: usize) -> Option<bool> {
        let allowed_range = self.combine_allowed_ranges(left, right)?;
        let actual_range = self.range_between(left, right)?;
        Some(actual_range <= allowed_range)
    }

    /// Returns the range between the two players
    /// Returns None if at least one of the indicies does not contain a player
    pub fn range_between(&self, left: usize, right: usize) -> Option<f64> {
        let left = self.queue.get(left)?;
        let right = self.queue.get(right)?;
        Some(
            f64::max(left.player.rating(), right.player.rating())
                - f64::min(left.player.rating(), right.player.rating()),
        )
    }

    /// Combines the ranges of either player for the purposes of determining if the match is
    /// allowed.
    ///
    /// If you use only the smallest range, matching will be higher quality but may result in
    /// longer queue times, especially for those with extreme mmr.
    ///
    /// If you use only the largest range, matching will be lower quality but may result in shorter
    /// queue times, especially for those with extreme mmr.
    ///
    /// Players with long wait times want worse quality matches to get into a game, but players
    /// with short wait times don't want to get low quality matches with a short queue. This
    /// function attempts to moderate these competing desires.
    ///
    /// Returns None if at least one of the indicies does not contain a player
    pub fn combine_allowed_ranges(&self, left: usize, right: usize) -> Option<f64> {
        let left = self.queue.get(left)?;
        let right = self.queue.get(right)?;
        Some((left.max_acceptable_mmr_range_now() + right.max_acceptable_mmr_range_now()) / 2.0)
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn iter(&self) -> core::slice::Iter<'_, QueuedPlayer> {
        self.queue.iter()
    }

    /// Attempt to make as many valid matches as possible
    pub fn make_matches(&mut self) -> Vec<[Player; MATCH_PLAYER_COUNT]> {
        // Consider some group of players pM through pN
        // When the mmr of pN finally the maximum allowed range for pM, we can safely move
        // the window forward so that M' = M + c + 1.
        //
        // Assume some Player p(M+k) is beyond the maximum allowed mmr range of Player p(M+c) where M+k < N and M+c
        // < N.
        //
        // When k > c, ALL matches which include Player p(M+c) will be invalid
        // Since all players further into the queue than p(M+k) will also violate p(M+c)'s max range,
        // p(M+c) cannot be matched. Thus, player p(M+c) can safely be discarded from the pool for this
        // iteration. Since players may be removed from the queue, N-M is not guaranteed to be exactly
        // 10.
        // // When k < c, ALL matches which include players between pN and p(M+c)
        // inclusive will be invalid. Since all players with lower mmr than p(M+k) will also violate
        // p(M+c)'s max range, we can skip the window forward so that M' = M + c + 1.
        let mut games: Vec<[usize; MATCH_PLAYER_COUNT]> = Vec::new();

        let skip = if self.prioritize_high {
            self.len() % MATCH_PLAYER_COUNT
        } else {
            0
        };

        let mut m = skip;
        let mut skip_list: Vec<usize> = Vec::new();
        'outer: while m < self.len() {
            let mut n = m + MATCH_PLAYER_COUNT;
            while n < self.len() {
                if !self.matching_allowed_between(m, n).unwrap() {
                    // In this scenario, no match can possibly be made beginning at pM
                    m += 1;
                    continue 'outer;
                }

                // The list M through N should contain exactly len(skip_list) more players than
                // MATCH_PLAYER_COUNT
                if n - m - skip_list.len() != MATCH_PLAYER_COUNT {
                    tracing::error!(
                        "{} - {} - {} != {}",
                        n,
                        m,
                        skip_list.len(),
                        MATCH_PLAYER_COUNT
                    );
                    tracing::error!("This shouldn't happen");
                    skip_list.clear();
                }
                let match_check_result = self.check_valid_match(m, n, &skip_list);

                if match_check_result.is_valid() {
                    // all players m through n excluding those skipped need to be added to a game,
                    // then the window needs to be moved forward such that M' = N + 1;
                    let mut game: [usize; MATCH_PLAYER_COUNT] = [0; MATCH_PLAYER_COUNT];
                    let mut game_index = 0;
                    for player_index in m..n {
                        if skip_list.contains(&player_index) {
                            continue;
                        }

                        game[game_index] = player_index;
                        game_index += 1;
                    }
                    games.push(game);

                    m = n + 1;
                    skip_list.clear();
                    continue 'outer;
                } else {
                    match match_check_result {
                        MatchValidityCheckResult::InvalidMoveForward(index) => {
                            for p in &mut self.queue[m..n] {
                                p.times_failed_to_match += 1;
                            }

                            m = index + 1;
                            skip_list.clear();
                            continue 'outer;
                        }
                        MatchValidityCheckResult::InvalidAddSkip(index) => {
                            skip_list.push(index);
                            self.queue.get_mut(index).unwrap().times_skipped += 1;
                            self.queue.get_mut(index).unwrap().times_failed_to_match += 1;
                            n += 1;
                        }
                        MatchValidityCheckResult::Valid => unreachable!(),
                    }
                }
            }

            m += 1;
            skip_list.clear();
        }

        // Grab the players out of the queue
        let mut player_games = Vec::new();
        for game in games.iter() {
            let mut player_game =
                [Player::new(Some(0.0), Some(0.0), Some(0.0), Some(0.0)); MATCH_PLAYER_COUNT];
            for (k, index) in game.iter().enumerate() {
                player_game[k] = self.queue.get(*index).unwrap().player;
            }

            player_games.push(player_game);
        }

        // Remove those players
        let remove_queue: Vec<usize> = games.into_iter().flatten().collect();
        let mut i = 0;
        self.queue.retain(|_| {
            let keep = !remove_queue.contains(&i);
            i += 1;
            keep
        });

        return player_games;
    }

    /// Checks if the match is valid given left bound index inclusive and right bound index
    /// exclusive
    pub fn check_valid_match(
        &self,
        left_bound: usize,
        right_bound: usize,
        skip_list: &[usize],
    ) -> MatchValidityCheckResult {
        for i in left_bound..right_bound {
            for j in left_bound..right_bound {
                if self.get(i) == self.get(j) {
                    continue;
                }

                if skip_list.contains(&i) || skip_list.contains(&j) {
                    continue;
                }

                if !self.matching_allowed_between(i, j).unwrap() {
                    let lower = usize::min(i, j);
                    return MatchValidityCheckResult::InvalidMoveForward(lower);
                }
            }
        }

        MatchValidityCheckResult::Valid
    }
}
