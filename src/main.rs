mod cli;
mod config;
mod executor;
mod logging;
mod parser;
mod planner;
mod prompt;
mod scanner;
mod tmdb;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands, ConfidenceArg, ScanType};
use config::{AppConfig, ConflictMode, EffectiveOperationMode, MediaType, NonMediaMode};
use parser::{parse_movie, parse_show, MediaInfo};
use planner::{build_movie_plan, build_show_plan, ConflictKind, Plan};
use scanner::scan_source;
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
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
            item.year = resolve_year(
                item.title.as_deref(),
                MediaType::Show,
                yes_mode,
                &tmdb_client,
            )?;
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
    preflight_destination_access(&plan)?;

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
            item.year = resolve_year(
                item.title.as_deref(),
                MediaType::Movie,
                yes_mode,
                &tmdb_client,
            )?;
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
    preflight_destination_access(&plan)?;

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
    let min_confidence = args.min_confidence.map(confidence_arg_to_rank).unwrap_or(0);

    for file in &scan.video_files {
        let parsed_item = match hint {
            Some(ScanType::Show) => parse_show(&file.file_name),
            Some(ScanType::Movie) => parse_movie(&file.file_name),
            None => auto_parse(&file.file_name),
        };

        let include = should_include_scan_item(&parsed_item, args.only_failed, min_confidence);

        if include {
            if args.json {
                json_items.push(to_json_scan_item(file, &parsed_item));
            } else {
                print_scan_item(file, &parsed_item);
            }
        }
        parsed.push(parsed_item);
    }

    let summary = summarize_scan(&parsed);
    let emitted_summary = summarize_scan_items_json(&json_items, args.json);

    let total = scan.video_files.len();
    let emitted_total = if args.json {
        json_items.len()
    } else {
        parsed
            .iter()
            .filter(|info| should_include_scan_item(info, args.only_failed, min_confidence))
            .count()
    };
    let omitted_by_filters = total.saturating_sub(emitted_total);
    if args.json {
        let report = ScanJsonReport {
            source: args.source.display().to_string(),
            filters: ScanFiltersJson {
                only_failed: args.only_failed,
                min_confidence: args
                    .min_confidence
                    .map(|v| format!("{:?}", v).to_ascii_lowercase()),
            },
            media_summary: MediaSummary {
                video: scan.video_files.len(),
                subtitle: scan.subtitle_files.len(),
                audio: scan.audio_files.len(),
                other: scan.other_files.len(),
            },
            parse_summary: ParseSummary {
                total_scanned: total,
                total_emitted: emitted_total,
                omitted_by_filters,
                parsed_ok: emitted_summary.parsed_ok,
                parsed_failed: emitted_summary.parsed_failed,
                with_year: emitted_summary.with_year,
                with_season: emitted_summary.with_season,
                with_episode: emitted_summary.with_episode,
            },
            items: json_items,
        };

        let payload = serde_json::to_string_pretty(&report)?;
        write_or_print_output(&payload, args.output.as_deref())?;
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
        "Summary: {} files scanned, {} emitted, {} parsed successfully, {} failed",
        total, emitted_total, summary.parsed_ok, summary.parsed_failed
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
    if args.only_failed || args.min_confidence.is_some() {
        println!(
            "Filters: only_failed={}, min_confidence={}",
            args.only_failed,
            args.min_confidence
                .map(|v| format!("{:?}", v).to_ascii_lowercase())
                .unwrap_or_else(|| "none".to_string())
        );
    }

    Ok(())
}

fn write_or_print_output(content: &str, output: Option<&Path>) -> Result<()> {
    if let Some(path) = output {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
        println!("Wrote report to {}", path.display());
    } else {
        println!("{}", content);
    }
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
        info.title
            .clone()
            .unwrap_or_else(|| "(not found)".to_string())
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
        source_path: file.path.display().to_string(),
        extension: file.extension.clone(),
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
    filters: ScanFiltersJson,
    media_summary: MediaSummary,
    parse_summary: ParseSummary,
    items: Vec<ScanItemJson>,
}

#[derive(Debug, Serialize)]
struct ScanFiltersJson {
    only_failed: bool,
    min_confidence: Option<String>,
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
    total_scanned: usize,
    total_emitted: usize,
    omitted_by_filters: usize,
    parsed_ok: usize,
    parsed_failed: usize,
    with_year: usize,
    with_season: usize,
    with_episode: usize,
}

#[derive(Debug, Serialize)]
struct ScanItemJson {
    file_name: String,
    source_path: String,
    extension: String,
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

fn summarize_scan_items_json(items: &[ScanItemJson], enabled: bool) -> ScanSummary {
    if !enabled {
        return ScanSummary::default();
    }
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

fn should_include_scan_item(info: &MediaInfo, only_failed: bool, min_confidence_rank: u8) -> bool {
    if only_failed && info.title.is_some() {
        return false;
    }
    confidence_rank(parse_confidence(info)) >= min_confidence_rank
}

fn confidence_arg_to_rank(value: ConfidenceArg) -> u8 {
    match value {
        ConfidenceArg::Low => 1,
        ConfidenceArg::Medium => 2,
        ConfidenceArg::High => 3,
    }
}

fn confidence_rank(value: &str) -> u8 {
    match value {
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
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

        if !plan.conflict_details.is_empty() {
            let files = plan
                .conflict_details
                .iter()
                .filter(|c| c.kind == ConflictKind::ExistingFile)
                .count();
            let dirs = plan
                .conflict_details
                .iter()
                .filter(|c| c.kind == ConflictKind::ExistingDirectory)
                .count();
            let parent_file = plan
                .conflict_details
                .iter()
                .filter(|c| c.kind == ConflictKind::ParentPathIsFile)
                .count();
            println!(
                "Conflict Types: existing-file={}, existing-directory={}, parent-path-file={}",
                files, dirs, parent_file
            );

            for detail in &plan.conflict_details {
                let kind = match detail.kind {
                    ConflictKind::ExistingFile => "existing-file",
                    ConflictKind::ExistingDirectory => "existing-directory",
                    ConflictKind::ParentPathIsFile => "parent-path-file",
                };
                if let Some(blocked_by) = &detail.blocked_by {
                    println!(
                        "CONFLICT [{}]: {} (blocked by file: {})",
                        kind,
                        detail.path.display(),
                        blocked_by.display()
                    );
                } else {
                    println!("CONFLICT [{}]: {}", kind, detail.path.display());
                }
            }
        } else {
            for path in &plan.conflicts {
                println!("CONFLICT: {}", path.display());
            }
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

fn preflight_destination_access(plan: &Plan) -> Result<()> {
    let mut read_only_blocked = Vec::new();
    let mut parent_file_blocked = Vec::new();
    let mut seen = HashSet::new();

    for op in &plan.operations {
        if let Some(parent) = op.destination.parent() {
            let key = parent.to_path_buf();
            if seen.insert(key.clone()) {
                let probe = nearest_existing_parent(parent);
                if let Some(existing) = probe {
                    if existing.is_file() {
                        parent_file_blocked.push((parent.to_path_buf(), existing.to_path_buf()));
                    } else if is_read_only_dir(existing) {
                        read_only_blocked.push(existing.to_path_buf());
                    }
                }
            }
        }
    }

    if read_only_blocked.is_empty() && parent_file_blocked.is_empty() {
        return Ok(());
    }

    let mut reasons = Vec::new();

    if !read_only_blocked.is_empty() {
        let joined = read_only_blocked
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        reasons.push(format!(
            "read-only directories: {} (adjust permissions or choose a different destination)",
            joined
        ));
    }

    if !parent_file_blocked.is_empty() {
        let joined = parent_file_blocked
            .iter()
            .map(|(parent, blocker)| {
                format!("{} blocked by {}", parent.display(), blocker.display())
            })
            .collect::<Vec<_>>()
            .join(", ");
        reasons.push(format!(
            "destination parent path collides with existing file: {}",
            joined
        ));
    }

    anyhow::bail!("destination preflight failed: {}", reasons.join("; "))
}

fn nearest_existing_parent(path: &Path) -> Option<&Path> {
    let mut current = Some(path);
    while let Some(p) = current {
        if p.exists() {
            return Some(p);
        }
        current = p.parent();
    }
    None
}

fn is_read_only_dir(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.permissions().readonly())
        .unwrap_or(false)
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
        confidence_arg_to_rank, confidence_rank, detected_kind, is_read_only_dir, parse_confidence,
        preflight_conflicts, preflight_destination_access, resolve_conflict_policy,
        should_include_scan_item, summarize_scan, to_json_scan_item, truncate_middle,
        ConflictPolicy,
    };
    use crate::config::{AppConfig, ConflictMode};
    use crate::parser::MediaInfo;
    use crate::planner::Plan;
    use crate::tmdb::MetadataLookup;
    use tempfile::tempdir;

    struct FailingLookup;

    impl MetadataLookup for FailingLookup {
        fn lookup_year(
            &self,
            _title: &str,
            _media_type: crate::config::MediaType,
        ) -> anyhow::Result<Option<u16>> {
            anyhow::bail!("network unavailable")
        }
    }

    fn args(
        overwrite: bool,
        on_conflict: Option<crate::cli::ConflictArg>,
    ) -> crate::cli::ShowMovieArgs {
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

        assert_eq!(
            resolve_conflict_policy(&args, &cfg_overwrite),
            ConflictPolicy::Overwrite
        );
        assert_eq!(
            resolve_conflict_policy(&args, &cfg_abort),
            ConflictPolicy::Abort
        );
        assert_eq!(
            resolve_conflict_policy(&args, &cfg_skip),
            ConflictPolicy::Skip
        );
    }

    #[test]
    fn preflight_conflicts_aborts_when_configured() {
        let mut plan = Plan::default();
        plan.conflicts
            .push(std::path::PathBuf::from("/tmp/existing.mkv"));
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
        assert_eq!(json_item.source_path, "/tmp/Show.S01E01.mkv");
        assert_eq!(json_item.extension, ".mkv");
        assert_eq!(json_item.media_type, "video");
        assert_eq!(json_item.detected_kind, "show");
        assert_eq!(json_item.parse_confidence, "high");
    }

    #[test]
    fn confidence_rank_orders_levels() {
        assert!(confidence_rank("high") > confidence_rank("medium"));
        assert!(confidence_rank("medium") > confidence_rank("low"));
        assert_eq!(confidence_rank("unknown"), 0);
    }

    #[test]
    fn confidence_arg_to_rank_matches_strings() {
        assert_eq!(confidence_arg_to_rank(crate::cli::ConfidenceArg::Low), 1);
        assert_eq!(confidence_arg_to_rank(crate::cli::ConfidenceArg::Medium), 2);
        assert_eq!(confidence_arg_to_rank(crate::cli::ConfidenceArg::High), 3);
    }

    #[test]
    fn should_include_scan_item_applies_filters() {
        let ok = MediaInfo {
            title: Some("Show".to_string()),
            year: Some(2020),
            season: Some(1),
            episode: Some(1),
            ..Default::default()
        };
        let failed = MediaInfo::default();

        assert!(should_include_scan_item(&ok, false, 1));
        assert!(should_include_scan_item(&ok, false, 3));
        assert!(!should_include_scan_item(&ok, true, 1));
        assert!(should_include_scan_item(&failed, true, 1));
        assert!(!should_include_scan_item(&failed, false, 2));
    }

    #[test]
    fn omitted_by_filters_count_math_is_stable() {
        let total = 10usize;
        let emitted = 4usize;
        let omitted = total.saturating_sub(emitted);
        assert_eq!(omitted, 6);
    }

    #[test]
    fn preflight_destination_access_accepts_writable_temp_path() {
        let dir = tempdir().expect("create tempdir");
        let plan = Plan {
            operations: vec![crate::planner::Operation {
                source: dir.path().join("src/file.mkv"),
                destination: dir.path().join("dest/file.mkv"),
                kind: crate::planner::OperationKind::Copy,
            }],
            ..Default::default()
        };

        assert!(preflight_destination_access(&plan).is_ok());
    }

    #[test]
    fn is_read_only_dir_returns_false_for_writable_temp_path() {
        let dir = tempdir().expect("create tempdir");
        assert!(!is_read_only_dir(dir.path()));
    }

    #[test]
    fn preflight_destination_access_rejects_parent_path_file_collision() {
        let dir = tempdir().expect("create tempdir");
        let blocker = dir.path().join("dest");
        std::fs::write(&blocker, b"file blocker").expect("write blocker file");

        let plan = Plan {
            operations: vec![crate::planner::Operation {
                source: dir.path().join("src/file.mkv"),
                destination: blocker.join("child/file.mkv"),
                kind: crate::planner::OperationKind::Copy,
            }],
            ..Default::default()
        };

        let err = preflight_destination_access(&plan)
            .expect_err("expected parent file collision to fail");
        let message = err.to_string();
        assert!(message.contains("parent path collides with existing file"));
    }

    #[test]
    fn write_or_print_output_writes_file_when_path_provided() {
        let dir = tempdir().expect("create tempdir");
        let path = dir.path().join("reports/scan.json");

        super::write_or_print_output("{\"ok\":true}", Some(path.as_path()))
            .expect("write output file");

        let written = std::fs::read_to_string(path).expect("read written file");
        assert_eq!(written, "{\"ok\":true}");
    }
}
