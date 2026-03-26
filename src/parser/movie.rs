use super::{extract_year_from_input, tokens, MediaInfo};

pub fn parse_movie(input: &str) -> MediaInfo {
    let extension = tokens::extract_extension(input);
    let base = tokens::strip_extension(input);
    let normalized = tokens::normalize_name(&base);

    let year = extract_year_from_input(&normalized);
    let title = tokens::extract_title(&normalized);

    MediaInfo {
        title,
        year,
        season: None,
        episode: None,
        extension,
        original_filename: input.to_string(),
        full_path: None,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_movie;

    #[test]
    fn parses_movie_with_year_token() {
        let info = parse_movie("Movie.Name.2023.1080p.BluRay.x265.mkv");
        assert_eq!(info.title.as_deref(), Some("Movie Name"));
        assert_eq!(info.year, Some(2023));
        assert_eq!(info.season, None);
        assert_eq!(info.episode, None);
    }

    #[test]
    fn parses_batman_style() {
        let info = parse_movie("The Batman (2022) (1080p BluRay x265 10bit Tigole).mkv");
        assert_eq!(info.title.as_deref(), Some("The Batman"));
        assert_eq!(info.year, Some(2022));
    }

    #[test]
    fn parses_breaking_bad_like_as_movie_for_shared_parser_case() {
        let info = parse_movie("Breaking.Bad.S01E01.Pilot.1080p.BluRay.x265.HEVC.10bit-CAKES.mkv");
        assert_eq!(info.title.as_deref(), Some("Breaking Bad"));
    }

    #[test]
    fn parses_bracket_heavy_movie_release() {
        let info = parse_movie("Movie.Title.[2023].[1080p].[WEB-DL].[x265].mkv");
        assert_eq!(info.title.as_deref(), Some("Movie Title"));
        assert_eq!(info.year, Some(2023));
    }

    #[test]
    fn parses_remux_movie_release() {
        let info = parse_movie("Movie Title (2021) [4K Remux HEVC TrueHD].mkv");
        assert_eq!(info.title.as_deref(), Some("Movie Title"));
        assert_eq!(info.year, Some(2021));
    }
}
