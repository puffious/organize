use regex::Regex;

pub fn normalize_name(input: &str) -> String {
    input
        .replace(['.', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn strip_extension(name: &str) -> String {
    match name.rfind('.') {
        Some(idx) => name[..idx].to_string(),
        None => name.to_string(),
    }
}

pub fn extract_extension(name: &str) -> String {
    match name.rfind('.') {
        Some(idx) => name[idx..].to_ascii_lowercase(),
        None => String::new(),
    }
}

pub fn title_boundary_index(normalized: &str) -> usize {
    let boundary_patterns = [
        Regex::new(r"(?i)\bS\d{1,2}E\d{1,3}(?:E\d{1,3})*(?:-E\d{1,3})?\b").expect("valid regex"),
        Regex::new(r"(?i)\bS\d{1,2}\b").expect("valid regex"),
        Regex::new(r"(?i)\bSeason\s*\d{1,2}\b").expect("valid regex"),
        Regex::new(r"\((19\d{2}|20\d{2})\)").expect("valid regex"),
        Regex::new(r"\b(19\d{2}|20\d{2})\b").expect("valid regex"),
        Regex::new(r"(?i)\b(480p|720p|1080p|2160p|4K)\b").expect("valid regex"),
        Regex::new(r"(?i)\b(BluRay|BRRip|WEB-DL|WEBRip|HDTV|DVDRip|DRPO)\b").expect("valid regex"),
        Regex::new(r"(?i)\b(x264|x265|H\.?264|H\.?265|HEVC|AVC|AV1|XviD|DivX)\b")
            .expect("valid regex"),
        Regex::new(r"(?i)\b(AAC(2\.0|5\.1)?|AC3|DTS|FLAC|MP3|EAC3|Atmos|TrueHD)\b")
            .expect("valid regex"),
        Regex::new(r"(?i)\b(SDR|HDR|HDR10\+?|DV|DoVi|Dolby\s*Vision)\b").expect("valid regex"),
    ];

    let mut min = normalized.len();
    for pattern in boundary_patterns {
        if let Some(m) = pattern.find(normalized) {
            min = min.min(m.start());
        }
    }
    min
}

pub fn clean_title(raw: &str) -> String {
    raw.trim()
        .trim_end_matches('-')
        .trim_end_matches(':')
        .trim()
        .to_string()
}
