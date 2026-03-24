mod cli;
mod config;
mod logging;
mod executor;
mod parser;
mod planner;
mod prompt;
mod scanner;
mod tmdb;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands, ScanType};
use config::{AppConfig, ConflictMode, EffectiveOperationMode, MediaType, NonMediaMode};
use parser::{parse_movie, parse_show, MediaInfo};
use planner::{build_movie_plan, build_show_plan, Plan};
use scanner::scan_source;
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;
use tmdb::MetadataLookup;
use tracing::{info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConflictPolicy {
    Skip,
    Overwrite,
    Abort,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = AppConfig::load_and_merge(&cli)?;
    logging::init_logging(cli.verbose, config.general.log_file.as_deref())?;

    match &cli.command {
        Commands::Show(args) => run_show(args, &config, cli.yes, cli.dry_run),
        Commands::Movie(args) => run_movie(args, &config, cli.yes, cli.dry_run),
        Commands::Scan(args) => run_scan(args, &config),
    }
}

fn run_show(
    args: &cli::ShowMovieArgs,
    config: &AppConfig,
    cli_yes: bool,
    cli_dry_run: bool,
) -> Result<()> {
    let scan = scan_source(&args.source, &config.media_extensions)?;
    let non_media_mode = match args.non_media {
        Some(cli::NonMediaArg::Keep) => NonMediaMode::Keep,
        Some(cli::NonMediaArg::Ignore) => NonMediaMode::Ignore,
        None => config.non_media.mode,
    }
    .to_scanner_mode();
    let mode = EffectiveOperationMode::from_args_and_config(args, config);
    let yes_mode = cli_yes || args.yes || config.general.auto_confirm;
    let dry_run = cli_dry_run || args.dry_run;
    let tmdb_client = tmdb::TmdbClient::new(config.tmdb.api_key.clone());

    let mut parsed = Vec::with_capacity(scan.video_files.len());
    for f in &scan.video_files {
        let mut item = parse_show(&f.file_name);
        item.original_filename = f.file_name.clone();
        item.extension = f.extension.clone();
        item.full_path = Some(f.path.clone());

        if item.title.is_none() {
            item.title = parse_show(&f.parent_name).title;
        }

        if item.year.is_none() {
            item.year = parser::extract_year_from_input(&f.parent_name);
        }
        if item.season.is_none() {
            item.season = parser::extract_season_from_input(&f.parent_name);
        }
        if item.season.is_none() && is_extras_folder(&f.parent_name) {
            item.season = Some(0);
        }

        if item.year.is_none() {
            item.year = resolve_year(item.title.as_deref(), MediaType::Show, yes_mode, &tmdb_client)?;
        }

        if item.season.is_none() || item.episode.is_none() {
            resolve_season_episode_if_missing(&mut item, yes_mode)?;
        }

        parsed.push(item);
    }

    let plan = build_show_plan(
        &scan,
        &parsed,
        &args.destination,
        args.title.clone(),
        args.year,
        mode,
        non_media_mode,
    )?;

    present_plan(&plan, true, dry_run, &args.source, &args.destination)?;

    if dry_run {
        return Ok(());
    }

    let conflict_policy = resolve_conflict_policy(args, config);
    preflight_conflicts(&plan, conflict_policy)?;

    if !yes_mode && !prompt::confirm_execute()? {
        info!("Operation cancelled by user");
        return Ok(());
    }

    let result = executor::execute_plan(&plan, conflict_policy == ConflictPolicy::Overwrite)?;
    println!(
        "Execution: {} succeeded, {} failed, {} skipped",
        result.succeeded, result.failed, result.skipped
    );
    for failure in &result.failures {
        println!("FAILED: {}", failure);
    }

    if args.clean || config.general.clean_empty_dirs {
        scanner::clean_empty_dirs(&args.source)?;
    }

    result.into_exit_result()
}

fn run_movie(
    args: &cli::ShowMovieArgs,
    config: &AppConfig,
    cli_yes: bool,
    cli_dry_run: bool,
) -> Result<()> {
    let scan = scan_source(&args.source, &config.media_extensions)?;
    let non_media_mode = match args.non_media {
        Some(cli::NonMediaArg::Keep) => NonMediaMode::Keep,
        Some(cli::NonMediaArg::Ignore) => NonMediaMode::Ignore,
        None => config.non_media.mode,
    }
    .to_scanner_mode();
    let mode = EffectiveOperationMode::from_args_and_config(args, config);
    let yes_mode = cli_yes || args.yes || config.general.auto_confirm;
    let dry_run = cli_dry_run || args.dry_run;
    let tmdb_client = tmdb::TmdbClient::new(config.tmdb.api_key.clone());

    let mut parsed = Vec::with_capacity(scan.video_files.len());
    for f in &scan.video_files {
        let mut item = parse_movie(&f.file_name);
        item.original_filename = f.file_name.clone();
        item.extension = f.extension.clone();
        item.full_path = Some(f.path.clone());
        if item.title.is_none() {
            item.title = parse_movie(&f.parent_name).title;
        }
        if item.year.is_none() {
            item.year = parser::extract_year_from_input(&f.parent_name);
        }
        if item.year.is_none() {
            item.year = resolve_year(item.title.as_deref(), MediaType::Movie, yes_mode, &tmdb_client)?;
        }
        parsed.push(item);
    }

    let plan = build_movie_plan(
        &scan,
        &parsed,
        &args.destination,
        args.title.clone(),
        args.year,
        mode,
        non_media_mode,
    )?;

    present_plan(&plan, false, dry_run, &args.source, &args.destination)?;

    if dry_run {
        return Ok(());
    }

    let conflict_policy = resolve_conflict_policy(args, config);
    preflight_conflicts(&plan, conflict_policy)?;

    if !yes_mode && !prompt::confirm_execute()? {
        info!("Operation cancelled by user");
        return Ok(());
    }

    let result = executor::execute_plan(&plan, conflict_policy == ConflictPolicy::Overwrite)?;
    println!(
        "Execution: {} succeeded, {} failed, {} skipped",
        result.succeeded, result.failed, result.skipped
    );
    for failure in &result.failures {
        println!("FAILED: {}", failure);
    }

    if args.clean || config.general.clean_empty_dirs {
        scanner::clean_empty_dirs(&args.source)?;
    }

    result.into_exit_result()
}

fn run_scan(args: &cli::ScanArgs, config: &AppConfig) -> Result<()> {
    let scan = scan_source(&args.source, &config.media_extensions)?;
    let hint = args.r#type;

    if !args.json {
        println!("Scanning: {}", args.source.display());
        println!();
    }

    let mut parsed = Vec::with_capacity(scan.video_files.len());
    let mut json_items = Vec::with_capacity(scan.video_files.len());

    for file in &scan.video_files {
        let parsed_item = match hint {
            Some(ScanType::Show) => parse_show(&file.file_name),
            Some(ScanType::Movie) => parse_movie(&file.file_name),
            None => auto_parse(&file.file_name),
        };

        if args.json {
            json_items.push(to_json_scan_item(file, &parsed_item));
        } else {
            print_scan_item(file, &parsed_item);
        }
        parsed.push(parsed_item);
    }

    let summary = summarize_scan(&parsed);

    let total = scan.video_files.len();
    if args.json {
        let report = ScanJsonReport {
            source: args.source.display().to_string(),
            media_summary: MediaSummary {
                video: scan.video_files.len(),
                subtitle: scan.subtitle_files.len(),
                audio: scan.audio_files.len(),
                other: scan.other_files.len(),
            },
            parse_summary: ParseSummary {
                total,
                parsed_ok: summary.parsed_ok,
                parsed_failed: summary.parsed_failed,
                with_year: summary.with_year,
                with_season: summary.with_season,
                with_episode: summary.with_episode,
            },
            items: json_items,
        };

        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!(
        "Media Summary: video={}, subtitle={}, audio={}, other={}",
        scan.video_files.len(),
        scan.subtitle_files.len(),
        scan.audio_files.len(),
        scan.other_files.len()
    );
    println!(
        "Summary: {} files scanned, {} parsed successfully, {} failed",
        total, summary.parsed_ok, summary.parsed_failed
    );
    println!(
        "Parse Coverage: title={}/{}, year={}/{}, season={}/{}, episode={}/{}",
        summary.parsed_ok,
        total,
        summary.with_year,
        total,
        summary.with_season,
        total,
        summary.with_episode,
        total
    );

    Ok(())
}

fn auto_parse(input: &str) -> MediaInfo {
    let show = parse_show(input);
    if show.season.is_some() || show.episode.is_some() {
        return show;
    }
    parse_movie(input)
}

fn print_scan_item(file: &scanner::ScannedFile, info: &MediaInfo) {
    let confidence = parse_confidence(info);
    let kind = detected_kind(info);
    println!("  {}", file.file_name);
    println!(
        "    Title:   {}",
        info.title.clone().unwrap_or_else(|| "(not found)".to_string())
    );
    println!(
        "    Year:    {}",
        info.year
            .map(|y| y.to_string())
            .unwrap_or_else(|| "(not found)".to_string())
    );
    println!(
        "    Season:  {}",
        info.season
            .map(|v| v.to_string())
            .unwrap_or_else(|| "(not found)".to_string())
    );
    println!(
        "    Episode: {}",
        info.episode
            .map(|v| v.to_string())
            .unwrap_or_else(|| "(not found)".to_string())
    );
    println!("    Type:    video");
    println!("    Kind:    {}", kind);
    println!("    Parse:   {}", confidence);
    println!();
}

fn to_json_scan_item(file: &scanner::ScannedFile, info: &MediaInfo) -> ScanItemJson {
    ScanItemJson {
        file_name: file.file_name.clone(),
        title: info.title.clone(),
        year: info.year,
        season: info.season,
        episode: info.episode,
        media_type: "video".to_string(),
        detected_kind: detected_kind(info).to_string(),
        parse_confidence: parse_confidence(info).to_string(),
    }
}

#[derive(Debug, Default)]
struct ScanSummary {
    parsed_ok: usize,
    parsed_failed: usize,
    with_year: usize,
    with_season: usize,
    with_episode: usize,
}

#[derive(Debug, Serialize)]
struct ScanJsonReport {
    source: String,
    media_summary: MediaSummary,
    parse_summary: ParseSummary,
    items: Vec<ScanItemJson>,
}

#[derive(Debug, Serialize)]
struct MediaSummary {
    video: usize,
    subtitle: usize,
    audio: usize,
    other: usize,
}

#[derive(Debug, Serialize)]
struct ParseSummary {
    total: usize,
    parsed_ok: usize,
    parsed_failed: usize,
    with_year: usize,
    with_season: usize,
    with_episode: usize,
}

#[derive(Debug, Serialize)]
struct ScanItemJson {
    file_name: String,
    title: Option<String>,
    year: Option<u16>,
    season: Option<u16>,
    episode: Option<u16>,
    media_type: String,
    detected_kind: String,
    parse_confidence: String,
}

fn summarize_scan(items: &[MediaInfo]) -> ScanSummary {
    let mut out = ScanSummary::default();
    for info in items {
        if info.title.is_some() {
            out.parsed_ok += 1;
        } else {
            out.parsed_failed += 1;
        }
        if info.year.is_some() {
            out.with_year += 1;
        }
        if info.season.is_some() {
            out.with_season += 1;
        }
        if info.episode.is_some() {
            out.with_episode += 1;
        }
    }
    out
}

fn parse_confidence(info: &MediaInfo) -> &'static str {
    let mut score = 0u8;
    if info.title.is_some() {
        score += 2;
    }
    if info.year.is_some() {
        score += 1;
    }
    if info.season.is_some() {
        score += 1;
    }
    if info.episode.is_some() {
        score += 1;
    }

    if score >= 4 {
        "high"
    } else if score >= 2 {
        "medium"
    } else {
        "low"
    }
}

fn detected_kind(info: &MediaInfo) -> &'static str {
    if info.season.is_some() || info.episode.is_some() {
        "show"
    } else if info.title.is_some() {
        "movie"
    } else {
        "unknown"
    }
}

fn present_plan(
    plan: &Plan,
    is_show: bool,
    dry_run: bool,
    source: &Path,
    destination: &Path,
) -> Result<()> {
    if dry_run {
        println!(
            "[DRY RUN] Organizing {}",
            if is_show { "show" } else { "movie" }
        );
        println!("Source: {}", source.display());
        println!("Destination: {}", destination.display());
        println!();
    }

    let conflicts: HashSet<_> = plan.conflicts.iter().collect();
    for op in &plan.operations {
        let marker = if conflicts.contains(&op.destination) {
            "[CONFLICT]"
        } else {
            "[OK]"
        };
        let src = truncate_middle(&op.source.display().to_string(), 56);
        let dst = truncate_middle(&op.destination.display().to_string(), 56);
        println!("  {} {} -> {}", marker, src, dst);
    }

    if !plan.conflicts.is_empty() {
        println!();
        warn!("{} conflicts detected", plan.conflicts.len());
        for path in &plan.conflicts {
            println!("CONFLICT: {}", path.display());
        }
    }

    if !plan.unparseable.is_empty() {
        println!();
        for item in &plan.unparseable {
            println!("SKIPPED: {} ({})", item.path.display(), item.reason);
        }
    }

    println!();
    println!(
        "Summary: {} files to process, {} conflicts, {} skipped",
        plan.operations.len(),
        plan.conflicts.len(),
        plan.unparseable.len()
    );

    Ok(())
}

fn truncate_middle(value: &str, max_len: usize) -> String {
    if value.len() <= max_len || max_len < 8 {
        return value.to_string();
    }

    let keep = (max_len - 3) / 2;
    let start = &value[..keep];
    let end = &value[value.len() - keep..];
    format!("{}...{}", start, end)
}

fn resolve_conflict_policy(args: &cli::ShowMovieArgs, config: &AppConfig) -> ConflictPolicy {
    if args.overwrite {
        return ConflictPolicy::Overwrite;
    }

    match args.on_conflict {
        Some(cli::ConflictArg::Overwrite) => ConflictPolicy::Overwrite,
        Some(cli::ConflictArg::Abort) => ConflictPolicy::Abort,
        Some(cli::ConflictArg::Skip) => ConflictPolicy::Skip,
        None => match config.general.conflict_mode {
            ConflictMode::Skip => ConflictPolicy::Skip,
            ConflictMode::Overwrite => ConflictPolicy::Overwrite,
            ConflictMode::Abort => ConflictPolicy::Abort,
        },
    }
}

fn preflight_conflicts(plan: &Plan, policy: ConflictPolicy) -> Result<()> {
    if plan.conflicts.is_empty() {
        return Ok(());
    }

    match policy {
        ConflictPolicy::Skip | ConflictPolicy::Overwrite => Ok(()),
        ConflictPolicy::Abort => {
            anyhow::bail!(
                "aborting due to {} conflict(s); rerun with --on-conflict skip or --on-conflict overwrite",
                plan.conflicts.len()
            )
        }
    }
}

fn resolve_year(
    title: Option<&str>,
    media_type: MediaType,
    yes_mode: bool,
    lookup: &impl MetadataLookup,
) -> Result<Option<u16>> {
    let Some(title) = title else {
        return Ok(None);
    };

    if !yes_mode {
        if let Some(year) = prompt::ask_for_year()? {
            return Ok(Some(year));
        }
    }

    match lookup.lookup_year(title, media_type) {
        Ok(year) => Ok(year),
        Err(err) => {
            warn!("TMDB lookup failed for '{}': {}", title, err);
            Ok(None)
        }
    }
}

fn resolve_season_episode_if_missing(info: &mut MediaInfo, yes_mode: bool) -> Result<()> {
    if info.season.is_some() && info.episode.is_some() {
        return Ok(());
    }

    if yes_mode {
        return Ok(());
    }

    if let Some((season, episode)) = prompt::ask_for_season_episode()? {
        if info.season.is_none() {
            info.season = Some(season);
        }
        if info.episode.is_none() {
            info.episode = Some(episode);
        }
    }
    Ok(())
}

fn is_extras_folder(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["extras", "featurettes", "behind the scenes", "specials"]
        .iter()
        .any(|token| n.contains(token))
}

#[cfg(test)]
mod tests {
    use super::{
        detected_kind, parse_confidence, preflight_conflicts, resolve_conflict_policy, summarize_scan,
        to_json_scan_item, truncate_middle, ConflictPolicy,
    };
    use crate::config::{AppConfig, ConflictMode};
    use crate::parser::MediaInfo;
    use crate::planner::Plan;
    use crate::tmdb::MetadataLookup;

    struct FailingLookup;

    impl MetadataLookup for FailingLookup {
        fn lookup_year(&self, _title: &str, _media_type: crate::config::MediaType) -> anyhow::Result<Option<u16>> {
            anyhow::bail!("network unavailable")
        }
    }

    fn args(overwrite: bool, on_conflict: Option<crate::cli::ConflictArg>) -> crate::cli::ShowMovieArgs {
        crate::cli::ShowMovieArgs {
            source: std::path::PathBuf::from("/tmp/src"),
            destination: std::path::PathBuf::from("/tmp/dst"),
            copy: false,
            link: false,
            symlink: false,
            overwrite,
            on_conflict,
            clean: false,
            title: None,
            year: None,
            non_media: None,
            dry_run: false,
            yes: false,
        }
    }

    fn config_with_conflict_mode(mode: ConflictMode) -> AppConfig {
        let mut cfg = AppConfig::default();
        cfg.general.conflict_mode = mode;
        cfg
    }

    #[test]
    fn truncate_middle_short_string_unchanged() {
        assert_eq!(truncate_middle("short", 20), "short");
    }

    #[test]
    fn truncate_middle_long_string_uses_ellipsis() {
        let value = "/this/is/a/very/long/path/that/needs/truncation/file.mkv";
        let out = truncate_middle(value, 24);
        assert!(out.contains("..."));
        assert!(out.len() <= 25);
    }

    #[test]
    fn conflict_policy_overwrite_flag_wins() {
        let a = args(true, Some(crate::cli::ConflictArg::Abort));
        let cfg = config_with_conflict_mode(ConflictMode::Skip);
        assert_eq!(resolve_conflict_policy(&a, &cfg), ConflictPolicy::Overwrite);
    }

    #[test]
    fn conflict_policy_uses_on_conflict_value() {
        let cfg = config_with_conflict_mode(ConflictMode::Skip);
        assert_eq!(
            resolve_conflict_policy(&args(false, Some(crate::cli::ConflictArg::Skip)), &cfg),
            ConflictPolicy::Skip
        );
        assert_eq!(
            resolve_conflict_policy(&args(false, Some(crate::cli::ConflictArg::Overwrite)), &cfg),
            ConflictPolicy::Overwrite
        );
        assert_eq!(
            resolve_conflict_policy(&args(false, Some(crate::cli::ConflictArg::Abort)), &cfg),
            ConflictPolicy::Abort
        );
    }

    #[test]
    fn conflict_policy_falls_back_to_config() {
        let args = args(false, None);
        let cfg_overwrite = config_with_conflict_mode(ConflictMode::Overwrite);
        let cfg_abort = config_with_conflict_mode(ConflictMode::Abort);
        let cfg_skip = config_with_conflict_mode(ConflictMode::Skip);

        assert_eq!(resolve_conflict_policy(&args, &cfg_overwrite), ConflictPolicy::Overwrite);
        assert_eq!(resolve_conflict_policy(&args, &cfg_abort), ConflictPolicy::Abort);
        assert_eq!(resolve_conflict_policy(&args, &cfg_skip), ConflictPolicy::Skip);
    }

    #[test]
    fn preflight_conflicts_aborts_when_configured() {
        let mut plan = Plan::default();
        plan.conflicts.push(std::path::PathBuf::from("/tmp/existing.mkv"));
        assert!(preflight_conflicts(&plan, ConflictPolicy::Abort).is_err());
        assert!(preflight_conflicts(&plan, ConflictPolicy::Skip).is_ok());
        assert!(preflight_conflicts(&plan, ConflictPolicy::Overwrite).is_ok());
    }

    #[test]
    fn summarize_scan_counts_parse_coverage() {
        let items = vec![
            MediaInfo {
                title: Some("A".to_string()),
                year: Some(2020),
                season: Some(1),
                episode: Some(1),
                ..Default::default()
            },
            MediaInfo {
                title: Some("B".to_string()),
                year: None,
                season: Some(2),
                episode: None,
                ..Default::default()
            },
            MediaInfo::default(),
        ];

        let summary = summarize_scan(&items);
        assert_eq!(summary.parsed_ok, 2);
        assert_eq!(summary.parsed_failed, 1);
        assert_eq!(summary.with_year, 1);
        assert_eq!(summary.with_season, 2);
        assert_eq!(summary.with_episode, 1);
    }

    #[test]
    fn parse_confidence_classifies_expected_levels() {
        let high = MediaInfo {
            title: Some("Show".to_string()),
            year: Some(2020),
            season: Some(1),
            episode: Some(2),
            ..Default::default()
        };
        let medium = MediaInfo {
            title: Some("Movie".to_string()),
            ..Default::default()
        };
        let low = MediaInfo::default();

        assert_eq!(parse_confidence(&high), "high");
        assert_eq!(parse_confidence(&medium), "medium");
        assert_eq!(parse_confidence(&low), "low");
    }

    #[test]
    fn detected_kind_prefers_show_when_episode_or_season_present() {
        let show_info = MediaInfo {
            title: Some("The Show".to_string()),
            season: Some(1),
            ..Default::default()
        };
        let movie_info = MediaInfo {
            title: Some("The Movie".to_string()),
            ..Default::default()
        };
        let unknown = MediaInfo::default();

        assert_eq!(detected_kind(&show_info), "show");
        assert_eq!(detected_kind(&movie_info), "movie");
        assert_eq!(detected_kind(&unknown), "unknown");
    }

    #[test]
    fn resolve_year_returns_none_on_lookup_failure() {
        let lookup = FailingLookup;
        let year = super::resolve_year(
            Some("Some Title"),
            crate::config::MediaType::Movie,
            true,
            &lookup,
        )
        .expect("resolve_year should not fail when lookup errors");
        assert_eq!(year, None);
    }

    #[test]
    fn to_json_scan_item_includes_kind_and_confidence() {
        let file = crate::scanner::ScannedFile {
            path: std::path::PathBuf::from("/tmp/Show.S01E01.mkv"),
            file_name: "Show.S01E01.mkv".to_string(),
            parent_name: "Show".to_string(),
            extension: ".mkv".to_string(),
        };
        let info = MediaInfo {
            title: Some("Show".to_string()),
            season: Some(1),
            episode: Some(1),
            ..Default::default()
        };

        let json_item = to_json_scan_item(&file, &info);
        assert_eq!(json_item.file_name, "Show.S01E01.mkv");
        assert_eq!(json_item.media_type, "video");
        assert_eq!(json_item.detected_kind, "show");
        assert_eq!(json_item.parse_confidence, "high");
    }
}
