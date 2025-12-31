use rand::rngs::ThreadRng;
use rand_distr::{Normal, Distribution};
use anyhow::Result;

const TARGET_PLAYER_COUNT: usize = 1000;
const MEAN_MMR: usize = 1500;
const STD_DEV: usize = 1000;
const TEAM_SIZE: usize = 5;
const TEAM_COUNT: usize = 2;

#[derive(Debug)]
struct MatchmakerMatch {
    teams: [[Player; TEAM_SIZE];TEAM_COUNT],
}

impl MatchmakerMatch {
    pub fn mmr_range(&self) -> usize {
        let min = self.teams.iter().flatten().map(|p| p.mmr).min().unwrap();
        let max = self.teams.iter().flatten().map(|p| p.mmr).max().unwrap();
        max - min
    }

    pub fn mmr_range_per_team(&self) -> [usize; TEAM_COUNT] {
        let mut ranges: [usize; TEAM_COUNT] = Default::default();
        for i in 0..TEAM_COUNT { 
            let min = self.teams[i].iter().map(|p| p.mmr).min().unwrap();
            let max = self.teams[i].iter().map(|p| p.mmr).max().unwrap();
            ranges[i] = max - min;
        }

        ranges
    }

    pub fn mmr_mean(&self) -> f64 {
        self.teams.iter().flatten().map(|p| p.mmr).sum::<usize>() as f64 / self.teams.iter().flatten().count() as f64
    }

    pub fn mmr_mean_per_team(&self) -> [f64; TEAM_COUNT] {
        let mut means: [f64; TEAM_COUNT] = Default::default();
        for i in 0..TEAM_COUNT {
            means[i] = self.teams[i].iter().map(|p| p.mmr).sum::<usize>() as f64 / self.teams[i].iter().count() as f64;
        }

        means
    }

    pub fn mmr_mean_differences(&self) -> [[f64; TEAM_COUNT]; TEAM_COUNT] {
        let means = self.mmr_mean_per_team();
        let mut means_diff: [[f64; TEAM_COUNT]; TEAM_COUNT] = Default::default();
        for i in 0..TEAM_COUNT {
            for j in 0..TEAM_COUNT {
                means_diff[i][j] = means[i] - means[j];
            }
        }

        means_diff
    }
}

#[derive(Debug)]
struct Player {
    mmr: usize, // how good the matchmaker thinks the player is
}

impl Default for Player {
    fn default() -> Self {
        Player {
            mmr: 0
        }
    }
}

#[derive(Debug)]
struct Matchmaker {
    rng: ThreadRng,
    distr: Normal<f64>,
    players: Vec<Player>,
}

#[derive(thiserror::Error, Debug, Clone)]
pub enum MatchmakingFailure {
    #[error("Matchmaking pool does not contain enough players to create a match")]
    NotEnoughPlayers,
}

impl Matchmaker {
    pub fn new() -> Self {
        let rng = rand::rng();
        let normal = Normal::new(MEAN_MMR as f64, STD_DEV as f64).unwrap();

        return Self {
            rng,
            distr: normal,
            players: vec![],
        }
    }

    pub fn add_player(&mut self, mmr: Option<usize>) {
        let player = if let Some(mmr) = mmr {
            Player { 
                mmr,
            }
        } else {
            Player {
                mmr: self.distr.sample(&mut self.rng) as usize,
            }
        };

        let mut i = 0;
        while i < self.players.len() {
            let p = self.players.get(i).unwrap();
            if player.mmr < p.mmr {
                break;
            }

            i += 1;
        }

        self.players.insert(i, player);
    }

    /// Attempts to make a match with a sliding window
    /// Begins at hint_idx, if present
    pub fn try_make_match(&mut self, hint_idx: Option<usize>) -> Result<MatchmakerMatch> {
        if self.players.len() < TEAM_SIZE * TEAM_COUNT {
            anyhow::bail!(MatchmakingFailure::NotEnoughPlayers);
        }

        let start_idx = if let Some(idx) = hint_idx {
            idx
        } else {
            0
        };

        // Naive first impl
        // The list is sorted, so just take the first available entries
        let mut teams: [[Player; TEAM_SIZE]; TEAM_COUNT] = Default::default();
        let mut team_pos = 0;
        for offset in 0..(TEAM_SIZE * TEAM_COUNT) {
            let p = self.players.remove(start_idx);
            let team_idx = (start_idx + offset) % TEAM_COUNT;
            teams[team_idx][team_pos] = p;
            if team_idx == TEAM_COUNT - 1 {
                team_pos += 1;
            }
        }

        Ok(MatchmakerMatch {
            teams,
        })
    }
}

fn main() {
    let mut mm = Matchmaker::new();
    for _ in 0..TARGET_PLAYER_COUNT {
        mm.add_player(None);
    }

    let mut mean_differences = vec![];
    let mut range_per_teams = vec![];
    let mut ranges = vec![];
    while let Ok(m) = mm.try_make_match(None) {
        mean_differences.push(m.mmr_mean_differences());
        range_per_teams.push(m.mmr_range_per_team());
        ranges.push(m.mmr_range());
    }
    // println!("MMR Mean Differences {:?}", match_.mmr_mean_differences());
    // println!("MMR Range Per Team {:?}", match_.mmr_range_per_team());
    println!("Average MMR Range {}", ranges.iter().sum::<usize>() as f64 / ranges.iter().count() as f64);
}
