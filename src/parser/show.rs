use regex::Regex;
use std::sync::LazyLock;

use super::{extract_year_from_input, tokens, MediaInfo};

pub fn parse_show(input: &str) -> MediaInfo {
    let extension = tokens::extract_extension(input);
    let base = tokens::strip_extension(input);
    let normalized = tokens::normalize_name(&base);

    let (season, episode) = extract_season_episode(&normalized);
    let year = extract_year_from_input(&normalized);
    let title = tokens::extract_title(&normalized);

    MediaInfo {
        title,
        year,
        season,
        episode,
        extension,
        original_filename: input.to_string(),
        full_path: None,
    }
}

static PATTERN_MULTI: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)S(\d{1,2})E(\d{1,3})(?:E\d{1,3})+").expect("valid regex"));
static PATTERN_RANGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)S(\d{1,2})E(\d{1,3})-E(\d{1,3})").expect("valid regex"));
static PATTERN_SE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)S(\d{1,2})E(\d{1,3})").expect("valid regex"));
static PATTERN_X: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(\d{1,2})x(\d{1,3})\b").expect("valid regex"));
static PATTERN_SEASON_EPISODE_WORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)Season\s*(\d{1,2})\s*Episode\s*(\d{1,3})").expect("valid regex")
});
static PATTERN_SEASON_ONLY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bS(\d{1,2})\b").expect("valid regex"));
static PATTERN_SEASON_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)Season\s*(\d{1,2})").expect("valid regex"));
static PATTERN_EP_ONLY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bE(\d{1,3})\b").expect("valid regex"));
static PATTERN_EPISODE_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)Episode\s*(\d{1,3})").expect("valid regex"));

fn extract_season_episode(normalized: &str) -> (Option<u16>, Option<u16>) {
    if let Some(c) = PATTERN_MULTI.captures(normalized) {
        let s = c.get(1).and_then(|m| m.as_str().parse::<u16>().ok());
        let e = c.get(2).and_then(|m| m.as_str().parse::<u16>().ok());
        return (s, e);
    }

    if let Some(c) = PATTERN_RANGE.captures(normalized) {
        let s = c.get(1).and_then(|m| m.as_str().parse::<u16>().ok());
        let e = c.get(2).and_then(|m| m.as_str().parse::<u16>().ok());
        return (s, e);
    }

    if let Some(c) = PATTERN_SE.captures(normalized) {
        let s = c.get(1).and_then(|m| m.as_str().parse::<u16>().ok());
        let e = c.get(2).and_then(|m| m.as_str().parse::<u16>().ok());
        return (s, e);
    }

    if let Some(c) = PATTERN_X.captures(normalized) {
        let s = c.get(1).and_then(|m| m.as_str().parse::<u16>().ok());
        let e = c.get(2).and_then(|m| m.as_str().parse::<u16>().ok());
        return (s, e);
    }

    if let Some(c) = PATTERN_SEASON_EPISODE_WORD.captures(normalized) {
        let s = c.get(1).and_then(|m| m.as_str().parse::<u16>().ok());
        let e = c.get(2).and_then(|m| m.as_str().parse::<u16>().ok());
        return (s, e);
    }

    if let Some(c) = PATTERN_SEASON_ONLY.captures(normalized) {
        let s = c.get(1).and_then(|m| m.as_str().parse::<u16>().ok());
        return (s, None);
    }

    if let Some(c) = PATTERN_SEASON_WORD.captures(normalized) {
        let s = c.get(1).and_then(|m| m.as_str().parse::<u16>().ok());
        return (s, None);
    }

    if let Some(c) = PATTERN_EP_ONLY.captures(normalized) {
        let e = c.get(1).and_then(|m| m.as_str().parse::<u16>().ok());
        return (None, e);
    }

    if let Some(c) = PATTERN_EPISODE_WORD.captures(normalized) {
        let e = c.get(1).and_then(|m| m.as_str().parse::<u16>().ok());
        return (None, e);
    }

    (None, None)
}

#[cfg(test)]
mod tests {
    use super::parse_show;

    #[test]
    fn parses_example_game_changer() {
        let info = parse_show(
            "Game Changer (2019) S05E01 (1080p DRPO WEB-DL H264 SDR AAC 2.0 English - HONE).mkv",
        );
        assert_eq!(info.title.as_deref(), Some("Game Changer"));
        assert_eq!(info.year, Some(2019));
        assert_eq!(info.season, Some(5));
        assert_eq!(info.episode, Some(1));
    }

    #[test]
    fn parses_dot_release() {
        let info = parse_show("Game.Changer.S01E01.1080p.DRPO.WEB-DL.AAC2.0.x264-FiZ.mkv");
        assert_eq!(info.title.as_deref(), Some("Game Changer"));
        assert_eq!(info.year, None);
        assert_eq!(info.season, Some(1));
        assert_eq!(info.episode, Some(1));
    }

    #[test]
    fn parses_black_mirror_style() {
        let info = parse_show(
            "Black Mirror (2011) - S04E01 - USS Callister (1080p BluRay x265 Panda).mkv",
        );
        assert_eq!(info.title.as_deref(), Some("Black Mirror"));
        assert_eq!(info.year, Some(2011));
        assert_eq!(info.season, Some(4));
        assert_eq!(info.episode, Some(1));
    }

    #[test]
    fn parses_office_us() {
        let info = parse_show("The.Office.US.S01E01.Pilot.720p.BluRay.x264-DEMAND.mkv");
        assert_eq!(info.title.as_deref(), Some("The Office US"));
        assert_eq!(info.season, Some(1));
        assert_eq!(info.episode, Some(1));
    }

    #[test]
    fn parses_multi_episode() {
        let info = parse_show("Show.Name.S01E01E02.1080p.mkv");
        assert_eq!(info.title.as_deref(), Some("Show Name"));
        assert_eq!(info.season, Some(1));
        assert_eq!(info.episode, Some(1));
    }

    #[test]
    fn parses_breaking_bad() {
        let info = parse_show("Breaking.Bad.S01E01.Pilot.1080p.BluRay.x265.HEVC.10bit-CAKES.mkv");
        assert_eq!(info.title.as_deref(), Some("Breaking Bad"));
        assert_eq!(info.season, Some(1));
        assert_eq!(info.episode, Some(1));
    }

    #[test]
    fn parses_folder_game_changer() {
        let info = parse_show(
            "Game Changer (2019) S05 (1080p DRPO WEB-DL H264 SDR AAC 2.0 English - HONE)",
        );
        assert_eq!(info.title.as_deref(), Some("Game Changer"));
        assert_eq!(info.year, Some(2019));
        assert_eq!(info.season, Some(5));
    }

    #[test]
    fn parses_folder_game_changer_dotted() {
        let info = parse_show("Game.Changer.S01.1080p.DRPO.WEB-DL.AAC2.0.x264-FiZ");
        assert_eq!(info.title.as_deref(), Some("Game Changer"));
        assert_eq!(info.year, None);
        assert_eq!(info.season, Some(1));
    }

    #[test]
    fn parses_folder_name_patterns() {
        let info = parse_show(
            "Black Mirror (2011) Season 4 S04 (1080p BluRay x265 HEVC 10bit AAC 5.1 Panda)",
        );
        assert_eq!(info.title.as_deref(), Some("Black Mirror"));
        assert_eq!(info.year, Some(2011));
        assert_eq!(info.season, Some(4));
    }

    #[test]
    fn parses_x_episode_pattern() {
        let info = parse_show("Show.Name.1x03.1080p.WEB-DL.mkv");
        assert_eq!(info.title.as_deref(), Some("Show Name"));
        assert_eq!(info.season, Some(1));
        assert_eq!(info.episode, Some(3));
    }

    #[test]
    fn parses_season_episode_words() {
        let info = parse_show("Show Name Season 2 Episode 5 1080p.mkv");
        assert_eq!(info.title.as_deref(), Some("Show Name"));
        assert_eq!(info.season, Some(2));
        assert_eq!(info.episode, Some(5));
    }
}
