use anyhow::Result;
use rand::{Rng, rngs::ThreadRng};
use rand_distr::{Distribution, Normal};

const TARGET_PLAYER_COUNT: usize = 1000;
const MAX_ADD_PER_TICK: usize = TARGET_PLAYER_COUNT / 2;
const MEAN_MMR: usize = 1500;
const STD_DEV: usize = 300;
const TEAM_SIZE: usize = 5;
const TEAM_COUNT: usize = 2;

const SIM_MAX_TICKS: usize = 3000;

#[derive(Debug, Clone)]
struct MatchmakerMatch {
    teams: [[Player; TEAM_SIZE]; TEAM_COUNT],
    ticks_to_complete: usize,
    ticks_since_beginning: usize,
    should_end: bool,
}

impl MatchmakerMatch {
    fn end_match(&mut self) -> [Player; TEAM_SIZE * TEAM_COUNT] {
        let mut arr: [Player; TEAM_SIZE * TEAM_COUNT] = Default::default();
        for (i, p) in self.teams.iter_mut().flatten().enumerate() {
            std::mem::swap(&mut arr[i], p);
        }

        arr
    }

    pub fn mmr_range(&self) -> usize {
        let min = self.teams.iter().flatten().map(|p| p.mmr).min().unwrap();
        let max = self.teams.iter().flatten().map(|p| p.mmr).max().unwrap();
        max - min
    }

    pub fn mmr_range_per_team(&self) -> [usize; TEAM_COUNT] {
        let mut ranges: [usize; TEAM_COUNT] = Default::default();
        (0..TEAM_COUNT).for_each(|i| {
            let min = self.teams[i].iter().map(|p| p.mmr).min().unwrap();
            let max = self.teams[i].iter().map(|p| p.mmr).max().unwrap();
            ranges[i] = max - min;
        });

        ranges
    }

    pub fn mmr_mean(&self) -> f64 {
        self.teams.iter().flatten().map(|p| p.mmr).sum::<usize>() as f64
            / self.teams.iter().flatten().count() as f64
    }

    pub fn mmr_mean_per_team(&self) -> [f64; TEAM_COUNT] {
        let mut means: [f64; TEAM_COUNT] = Default::default();
        for i in 0..TEAM_COUNT {
            means[i] = self.teams[i].iter().map(|p| p.mmr).sum::<usize>() as f64
                / self.teams[i].iter().count() as f64;
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

#[derive(Debug, Clone, Default)]
struct Player {
    mmr: usize, // how good the matchmaker thinks the player is
}

#[derive(Debug)]
struct QueuedPlayer {
    player: Player,
    wait_time: usize, // in seconds
}

impl QueuedPlayer {
    pub fn max_acceptable_mmr_range_now(&self) -> usize {
        // https://www.desmos.com/calculator/n5v6kzls65
        300 + (self.wait_time / 12).pow(2)
    }
}

#[derive(Debug)]
struct Matchmaker {
    rng: ThreadRng,
    distr: Normal<f64>,
    players: Vec<QueuedPlayer>,
    matches: Vec<MatchmakerMatch>,
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
            matches: vec![],
        };
    }

    pub fn add_player(&mut self, mmr: Option<usize>) {
        let player = if let Some(mmr) = mmr {
            Player { mmr }
        } else {
            Player {
                mmr: self.distr.sample(&mut self.rng) as usize,
            }
        };

        let mut i = 0;
        while i < self.players.len() {
            let p = self.players.get(i).unwrap();
            if player.mmr < p.player.mmr {
                break;
            }

            i += 1;
        }

        self.players.insert(
            i,
            QueuedPlayer {
                player,
                wait_time: 0,
            },
        );
    }

    pub fn tick(&mut self) {
        // let add_count = usize::max(
        //     MAX_ADD_PER_TICK
        //         * (1.0 - self.players.len() as f64 / TARGET_PLAYER_COUNT as f64) as usize,
        //     1,
        // );
        // for _ in 0..add_count {
        //     self.add_player(None);
        // }

        for qp in &mut self.players {
            qp.wait_time += 1;
        }
        let mut players_available: Vec<Player> = vec![];

        for m in self.matches.iter_mut() {
            if m.should_end {
                continue;
            }
            m.ticks_since_beginning += 1;
            if m.ticks_since_beginning >= m.ticks_to_complete {
                // the match is done
                for p in m.end_match() {
                    players_available.push(p);
                }
            }
        }

        for p in players_available {
            self.add_player(Some(p.mmr));
        }
    }

    pub fn make_all_matches(&mut self) -> Vec<MatchmakerMatch> {
        let mut i = 0;
        let mut matches = vec![];
        'top: while i < self.players.len() {
            if i + (TEAM_SIZE * TEAM_COUNT) > self.players.len() {
                break;
            }

            for j in 0..(TEAM_SIZE * TEAM_COUNT) {
                for k in 0..(TEAM_SIZE * TEAM_COUNT) {
                    let checker = &self.players[i + j];
                    let checkee = &self.players[i + k];
                    let mmr_diff =
                        (checker.player.mmr as isize - checkee.player.mmr as isize).unsigned_abs();

                    if mmr_diff > checker.max_acceptable_mmr_range_now() {
                        i += 1;
                        continue 'top;
                    }
                }
            }

            // if we arrive at this point, there is definitely a valid match at i
            matches.push(self.try_make_match(Some(i)).unwrap());
        }

        return matches;
    }

    /// Call to make a match with the next (TEAM_SIZE * TEAM_COUNT) players starting at hint_idx
    pub fn try_make_match(&mut self, hint_idx: Option<usize>) -> Result<MatchmakerMatch> {
        if self.players.len() < TEAM_SIZE * TEAM_COUNT {
            anyhow::bail!(MatchmakingFailure::NotEnoughPlayers);
        }

        let start_idx = if let Some(idx) = hint_idx { idx } else { 0 };

        let mut teams: [[Player; TEAM_SIZE]; TEAM_COUNT] = Default::default();
        let mut team_pos = 0;
        for offset in 0..(TEAM_SIZE * TEAM_COUNT) {
            let p = self.players.remove(start_idx);
            let team_idx = (start_idx + offset) % TEAM_COUNT;
            teams[team_idx][team_pos] = p.player;
            if team_idx == TEAM_COUNT {
                team_pos += 1;
            }
        }

        let mut rng = rand::rng();
        let match_ = MatchmakerMatch {
            teams,
            ticks_to_complete: rng.random_range(1200..2400),
            ticks_since_beginning: 0,
            should_end: false,
        };
        self.matches.push(match_.clone());

        Ok(match_)
    }
}

fn main() {
    let mut mm = Matchmaker::new();
    for _ in 0..TARGET_PLAYER_COUNT {
        mm.add_player(None);
    }

    let mut matches = vec![];

    for tick in 0..SIM_MAX_TICKS {
        mm.tick();

        if tick % 100 == 0 {
            print!("\x1B[2J\x1B[1;1H");
            println!("Tick number {}", &tick);
            println!(
                "Matchmaking pool currently contains {} players",
                mm.players.len()
            );
            println!(
                "Average wait time is {} ticks",
                mm.players.iter().map(|p| p.wait_time).sum::<usize>() as f64
                    / mm.players.len() as f64
            );
        }

        matches.append(&mut mm.make_all_matches());
    }

    let mut mean_differences = vec![];
    let mut range_per_teams = vec![];
    let mut ranges = vec![];
    for m in &matches {
        mean_differences.push(m.mmr_mean_differences());
        range_per_teams.push(m.mmr_range_per_team());
        ranges.push(m.mmr_range());
    }
    // println!("MMR Mean Differences {:?}", match_.mmr_mean_differences());
    // println!("MMR Range Per Team {:?}", match_.mmr_range_per_team());

    println!("——————");
    println!(
        "Matches made in {} ticks: {} — {} unique players",
        SIM_MAX_TICKS,
        matches.len(),
        matches.len() * TEAM_SIZE * TEAM_COUNT
    );
    println!(
        "Average MMR range within team: {:.2}",
        range_per_teams.iter().flatten().sum::<usize>() as f64
            / range_per_teams.iter().flatten().count() as f64
    );
    println!(
        "Average MMR Range {:.2}",
        ranges.iter().sum::<usize>() as f64 / ranges.iter().count() as f64
    );
}
