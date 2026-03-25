pub mod movie;
pub mod show;
pub mod tokens;

use regex::Regex;
use std::sync::LazyLock;

#[derive(Debug, Clone, Default)]
pub struct MediaInfo {
    pub title: Option<String>,
    pub year: Option<u16>,
    pub season: Option<u16>,
    pub episode: Option<u16>,
    pub extension: String,
    pub original_filename: String,
    pub full_path: Option<std::path::PathBuf>,
}

pub fn parse_show(input: &str) -> MediaInfo {
    show::parse_show(input)
}

pub fn parse_movie(input: &str) -> MediaInfo {
    movie::parse_movie(input)
}

static YEAR_PAREN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\((19\d{2}|20\d{2})\)").expect("valid regex"));
static YEAR_STANDALONE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|[^0-9])(19\d{2}|20\d{2})(?:[^0-9]|$)").expect("valid regex"));

pub fn extract_year_from_input(input: &str) -> Option<u16> {
    if let Some(c) = YEAR_PAREN.captures(input) {
        return c.get(1).and_then(|m| m.as_str().parse::<u16>().ok());
    }

    YEAR_STANDALONE
        .captures(input)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<u16>().ok())
}

static SEASON_S: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bS(\d{1,2})\b").expect("valid regex"));
static SEASON_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)Season\s*(\d{1,2})").expect("valid regex"));

pub fn extract_season_from_input(input: &str) -> Option<u16> {
    let lower = input.to_ascii_lowercase();
    if ["special", "specials", "extras", "featurette", "featurettes"]
        .iter()
        .any(|token| lower.contains(token))
    {
        return Some(0);
    }

    if let Some(c) = SEASON_S.captures(input) {
        if let Some(season) = c.get(1).and_then(|m| m.as_str().parse::<u16>().ok()) {
            return Some(season);
        }
    }

    if let Some(c) = SEASON_WORD.captures(input) {
        if let Some(season) = c.get(1).and_then(|m| m.as_str().parse::<u16>().ok()) {
            return Some(season);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::extract_season_from_input;

    #[test]
    fn detects_numeric_season_patterns() {
        assert_eq!(extract_season_from_input("Show.S02.1080p"), Some(2));
        assert_eq!(extract_season_from_input("Show Season 3 Pack"), Some(3));
    }

    #[test]
    fn maps_specials_and_extras_to_season_zero() {
        assert_eq!(extract_season_from_input("Show Specials"), Some(0));
        assert_eq!(extract_season_from_input("Show Featurettes Pack"), Some(0));
        assert_eq!(extract_season_from_input("Show Extras"), Some(0));
    }
}
