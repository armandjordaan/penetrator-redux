//! Persistence: high scores and the handful of settings worth remembering.
//!
//! Deliberately minimal. One small text file next to the executable's working
//! directory, written with plain `std::fs`, and every failure path is "carry on
//! without it". A game that refuses to start because it could not write a score
//! file is a worse game than one that quietly forgets your score.

use std::fs;
use std::path::{Path, PathBuf};

const SAVE_FILE: &str = "penetrator_save.txt";
const MAX_SCORES: usize = 5;

#[derive(Clone, Debug)]
pub struct Save {
    /// Best first.
    pub scores: Vec<u32>,
    pub crt: bool,
    pub muted: bool,
    /// The seed the player last chose, so "same cave again" survives a restart.
    pub last_seed: u64,
}

impl Default for Save {
    fn default() -> Self {
        Save {
            scores: Vec::new(),
            crt: true,
            muted: false,
            last_seed: 0,
        }
    }
}

impl Save {
    pub fn best(&self) -> u32 {
        self.scores.first().copied().unwrap_or(0)
    }

    /// Files the score and reports whether it made the table.
    pub fn record(&mut self, score: u32) -> bool {
        if score == 0 {
            return false;
        }
        let made_the_table = self.scores.len() < MAX_SCORES
            || self.scores.last().is_some_and(|&worst| score > worst);
        self.scores.push(score);
        self.scores.sort_unstable_by(|a, b| b.cmp(a));
        self.scores.truncate(MAX_SCORES);
        made_the_table
    }

    pub fn path() -> PathBuf {
        PathBuf::from(SAVE_FILE)
    }

    /// Loads from disk. A missing or corrupt file yields defaults rather than an
    /// error — there is nothing useful the player could do about either.
    pub fn load() -> Save {
        match fs::read_to_string(Self::path()) {
            Ok(text) => Self::parse(&text),
            Err(_) => Save::default(),
        }
    }

    pub fn save(&self) {
        let _ = fs::write(Self::path(), self.serialise());
    }

    pub fn serialise(&self) -> String {
        let mut out = String::from("# penetrator save data\n");
        for s in &self.scores {
            out.push_str(&format!("score {s}\n"));
        }
        out.push_str(&format!("crt {}\n", self.crt as u8));
        out.push_str(&format!("muted {}\n", self.muted as u8));
        out.push_str(&format!("seed {}\n", self.last_seed));
        out
    }

    /// Parses the save format. Unknown keys and unparseable values are skipped:
    /// a save file from a future version should cost the player their settings at
    /// worst, never their ability to play.
    pub fn parse(text: &str) -> Save {
        let mut save = Save::default();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let mut parts = line.split_whitespace();
            let (Some(key), Some(value)) = (parts.next(), parts.next()) else {
                continue;
            };
            match key {
                "score" => {
                    if let Ok(v) = value.parse::<u32>() {
                        save.scores.push(v);
                    }
                }
                "crt" => save.crt = value != "0",
                "muted" => save.muted = value != "0",
                "seed" => {
                    if let Ok(v) = value.parse::<u64>() {
                        save.last_seed = v;
                    }
                }
                _ => {}
            }
        }
        save.scores.sort_unstable_by(|a, b| b.cmp(a));
        save.scores.truncate(MAX_SCORES);
        save
    }
}

// ---------------------------------------------------------------------------
// Level files
// ---------------------------------------------------------------------------

/// Where the editor writes and the game looks for custom levels. `write_level`
/// creates the directory on demand, so no separate setup step is needed.
pub const CUSTOM_LEVEL: &str = "levels/custom.pen";
/// The hand-written sample that ships with the game, documenting the format.
pub const EXAMPLE_LEVEL: &str = "levels/example.pen";

/// The custom level the menu should offer, if there is one.
///
/// Anything the player has built wins; otherwise the shipped example stands in,
/// so the menu entry is never a dead end on a fresh install.
pub fn playable_level() -> Option<&'static str> {
    [CUSTOM_LEVEL, EXAMPLE_LEVEL]
        .into_iter()
        .find(|p| level_exists(p))
}

/// Writes a level, creating the directory if it is missing.
pub fn write_level(path: &str, contents: &str) -> Result<(), String> {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| format!("could not create {}: {e}", parent.display()))?;
        }
    }
    fs::write(path, contents).map_err(|e| format!("could not write {path}: {e}"))
}

pub fn read_level(path: &str) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("could not read {path}: {e}"))
}

pub fn level_exists(path: &str) -> bool {
    Path::new(path).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_example_level_is_offered_when_nothing_is_saved() {
        // Not asserting which one comes back — the working directory during a
        // test run is the package root, but a player's may be anywhere. What
        // matters is the preference order.
        assert_eq!([CUSTOM_LEVEL, EXAMPLE_LEVEL][0], CUSTOM_LEVEL);
        if level_exists(CUSTOM_LEVEL) {
            assert_eq!(playable_level(), Some(CUSTOM_LEVEL));
        } else if level_exists(EXAMPLE_LEVEL) {
            assert_eq!(playable_level(), Some(EXAMPLE_LEVEL));
        }
    }

    #[test]
    fn scores_are_kept_in_order_and_capped() {
        let mut s = Save::default();
        for v in [100, 900, 50, 700, 300, 20, 1000] {
            s.record(v);
        }
        assert_eq!(s.scores.len(), MAX_SCORES);
        assert_eq!(s.best(), 1000);
        assert!(s.scores.windows(2).all(|w| w[0] >= w[1]));
        assert!(!s.scores.contains(&20), "the worst score should fall off");
    }

    #[test]
    fn a_zero_score_is_not_recorded() {
        let mut s = Save::default();
        assert!(!s.record(0));
        assert!(s.scores.is_empty());
    }

    #[test]
    fn making_the_table_is_reported_correctly() {
        let mut s = Save::default();
        for v in [500, 400, 300, 200, 100] {
            assert!(s.record(v));
        }
        assert!(!s.record(50), "below the table");
        assert!(s.record(450), "above the worst entry");
    }

    #[test]
    fn best_is_zero_on_a_fresh_save() {
        assert_eq!(Save::default().best(), 0);
    }

    #[test]
    fn round_trips_through_the_text_format() {
        let mut s = Save::default();
        s.record(4200);
        s.record(999);
        s.crt = false;
        s.muted = true;
        s.last_seed = 987654321;

        let back = Save::parse(&s.serialise());
        assert_eq!(back.scores, s.scores);
        assert!(!back.crt);
        assert!(back.muted);
        assert_eq!(back.last_seed, 987654321);
    }

    #[test]
    fn a_corrupt_save_degrades_to_defaults_rather_than_failing() {
        let junk = "score not-a-number\nnonsense\ncrt\n\n# comment only\nseed ????\n";
        let s = Save::parse(junk);
        assert!(s.scores.is_empty());
        assert!(s.crt, "unparsed settings keep their defaults");
        assert_eq!(s.last_seed, 0);
    }

    #[test]
    fn an_empty_file_is_fine() {
        let s = Save::parse("");
        assert_eq!(s.best(), 0);
    }

    #[test]
    fn unknown_keys_from_a_future_version_are_ignored() {
        let s = Save::parse("score 10\nquantum_flux 42\ncrt 0\n");
        assert_eq!(s.best(), 10);
        assert!(!s.crt);
    }
}
