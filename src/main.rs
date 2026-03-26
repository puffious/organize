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
use planner::{
    build_movie_plan, build_show_plan, ConflictKind, Plan, UnparseableItem, UnparseableReason,
};
use prompt::{ShowGroupPrompt, ShowGroupResolution};
use scanner::scan_source;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::path::PathBuf;
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
        Commands::Scan(args) => run_scan(&cli, args, &config),
        Commands::Doctor(args) => run_doctor(&cli, &config, args),
    }
}

fn scan_parse_context(file: &scanner::ScannedFile, hint: Option<ScanType>) -> ScanContext<'_> {
    let parser_mode = match hint {
        Some(ScanType::Show) => "show",
        Some(ScanType::Movie) => "movie",
        None => "auto",
    };

    ScanContext { file, parser_mode }
}

fn parse_scan_info(
    context: &ScanContext<'_>,
    hint: Option<ScanType>,
) -> (MediaInfo, ScanFieldSources) {
    let mut info = match hint {
        Some(ScanType::Show) => parse_show(&context.file.file_name),
        Some(ScanType::Movie) => parse_movie(&context.file.file_name),
        None => auto_parse(&context.file.file_name),
    };

    let mut sources = ScanFieldSources {
        title: if info.title.is_some() {
            "filename"
        } else {
            "missing"
        },
        year: if info.year.is_some() {
            "filename"
        } else {
            "missing"
        },
        season: if info.season.is_some() {
            "filename"
        } else {
            "missing"
        },
        episode: if info.episode.is_some() {
            "filename"
        } else {
            "missing"
        },
    };

    if info.title.is_none() {
        let parent_info = match hint {
            Some(ScanType::Movie) => parse_movie(&context.file.parent_name),
            _ => parse_show(&context.file.parent_name),
        };
        if let Some(title) = parent_info.title {
            info.title = Some(title);
            sources.title = "parent";
        }
    }
    if info.year.is_none() {
        if let Some(year) = parser::extract_year_from_input(&context.file.parent_name) {
            info.year = Some(year);
            sources.year = "parent";
        }
    }
    if info.season.is_none() {
        if let Some(season) = parser::extract_season_from_input(&context.file.parent_name) {
            info.season = Some(season);
            sources.season = "parent";
        }
    }

    info.original_filename = context.file.file_name.clone();
    info.extension = context.file.extension.clone();
    info.full_path = Some(context.file.path.clone());

    (info, sources)
}

fn scan_issues(info: &MediaInfo) -> Vec<String> {
    let mut issues = Vec::new();
    if info.title.is_none() {
        issues.push("missing_title".to_string());
    }
    if info.year.is_none() {
        issues.push("missing_year".to_string());
    }
    if detected_kind(info) == "show" {
        if info.season.is_none() {
            issues.push("missing_season".to_string());
        }
        if info.episode.is_none() {
            issues.push("missing_episode".to_string());
        }
    }
    issues
}

fn config_path_for_report(cli: &Cli) -> Option<PathBuf> {
    if let Some(path) = &cli.config {
        return Some(path.clone());
    }

    let local_path = PathBuf::from(".organize.toml");
    if local_path.exists() {
        return Some(local_path);
    }

    config::global_config_path().filter(|path| path.exists())
}

fn doctor_check(name: &str, status: &'static str, detail: String) -> DoctorCheck {
    DoctorCheck {
        name: name.to_string(),
        status,
        detail,
    }
}

fn path_status(path: &Path) -> &'static str {
    if path.exists() {
        "pass"
    } else {
        "fail"
    }
}

fn destination_status(path: &Path) -> (&'static str, String) {
    if path.exists() {
        return ("pass", format!("destination exists: {}", path.display()));
    }

    if let Some(parent) = path.parent() {
        if parent.exists() {
            return (
                "pass",
                format!("destination parent exists: {}", parent.display()),
            );
        }
    }

    (
        "warn",
        format!("destination parent missing: {}", path.display()),
    )
}

fn effective_config_summary(config: &AppConfig) -> String {
    format!(
        "mode={:?}, auto_confirm={}, clean_empty_dirs={}, conflict_mode={:?}",
        config.general.default_mode,
        config.general.auto_confirm,
        config.general.clean_empty_dirs,
        config.general.conflict_mode
    )
}

fn media_extensions_summary(config: &AppConfig) -> String {
    format!(
        "video={}, subtitle={}, audio={}",
        config.media_extensions.video.len(),
        config.media_extensions.subtitle.len(),
        config.media_extensions.audio.len()
    )
}

fn scan_output_report(report: &ScanJsonReport, output: Option<&Path>) -> Result<()> {
    let payload = serde_json::to_string_pretty(report)?;
    write_or_print_output(&payload, output)
}

fn doctor_output_report(report: &DoctorReport, output: Option<&Path>) -> Result<()> {
    let payload = serde_json::to_string_pretty(report)?;
    write_or_print_output(&payload, output)
}

fn print_text_scan_summary(
    scan: &scanner::ScanResult,
    total: usize,
    emitted_total: usize,
    summary: &ScanSummary,
) {
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
        "Kinds: show={}, movie={}, unknown={}",
        summary.detected_show, summary.detected_movie, summary.detected_unknown
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
    println!(
        "Issues: missing_title={}, missing_year={}, missing_season={}, missing_episode={}",
        summary.missing_title,
        summary.missing_year,
        summary.missing_season,
        summary.missing_episode
    );
    println!(
        "Fallbacks: parent-assisted={}",
        summary.used_parent_fallback
    );
}

fn print_scan_filters(args: &cli::ScanArgs) {
    if args.only_failed || args.min_confidence.is_some() {
        println!(
            "Filters: only_failed={}, min_confidence={}",
            args.only_failed,
            args.min_confidence
                .map(|v| format!("{:?}", v).to_ascii_lowercase())
                .unwrap_or_else(|| "none".to_string())
        );
    }
}

fn print_doctor_check(check: &DoctorCheck) {
    println!(
        "  [{}] {} - {}",
        check.status.to_ascii_uppercase(),
        check.name,
        check.detail
    );
}

fn print_doctor_header(report: &DoctorReport) {
    println!("Doctor:");
    if let Some(path) = &report.config_path {
        println!("  Config path: {}", path);
    } else {
        println!("  Config path: default discovery");
    }
    println!();
}

fn print_doctor_footer(report: &DoctorReport) {
    let passes = report
        .checks
        .iter()
        .filter(|check| check.status == "pass")
        .count();
    let warns = report
        .checks
        .iter()
        .filter(|check| check.status == "warn")
        .count();
    let fails = report
        .checks
        .iter()
        .filter(|check| check.status == "fail")
        .count();
    println!();
    println!("Summary: {} pass, {} warn, {} fail", passes, warns, fails);
}

fn run_scan(cli: &Cli, args: &cli::ScanArgs, config: &AppConfig) -> Result<()> {
    let scan = scan_source(&args.source, &config.media_extensions)?;
    let min_confidence = args.min_confidence.map(confidence_arg_to_rank).unwrap_or(0);

    if !args.json {
        println!("Scanning: {}", args.source.display());
        println!();
    }

    let mut reports = Vec::with_capacity(scan.video_files.len());
    let mut json_items = Vec::with_capacity(scan.video_files.len());

    for file in &scan.video_files {
        let report = scan_item_report(file, args.r#type);
        if should_include_scan_report(&report, args.only_failed, min_confidence) {
            if args.json {
                json_items.push(report.to_json_item(file));
            } else {
                print_scan_report(
                    file,
                    &report,
                    ScanRenderOptions {
                        verbose: cli.verbose > 0,
                    },
                );
            }
        }
        reports.push(report);
    }

    let total = reports.len();
    let emitted_total = json_items.len().max(
        reports
            .iter()
            .filter(|report| should_include_scan_report(report, args.only_failed, min_confidence))
            .count(),
    );
    let omitted_by_filters = total.saturating_sub(emitted_total);
    let summary = scan_summary_from_reports(&reports);

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
                parsed_ok: summary.parsed_ok,
                parsed_failed: summary.parsed_failed,
                with_year: summary.with_year,
                with_season: summary.with_season,
                with_episode: summary.with_episode,
                detected_show: summary.detected_show,
                detected_movie: summary.detected_movie,
                detected_unknown: summary.detected_unknown,
                used_parent_fallback: summary.used_parent_fallback,
                missing_title: summary.missing_title,
                missing_year: summary.missing_year,
                missing_season: summary.missing_season,
                missing_episode: summary.missing_episode,
            },
            diagnostics_summary: summarize_diagnostics(&reports),
            items: json_items,
        };
        return scan_output_report(&report, args.output.as_deref());
    }

    print_text_scan_summary(&scan, total, emitted_total, &summary);
    print_scan_filters(args);
    Ok(())
}

fn run_doctor(cli: &Cli, config: &AppConfig, args: &cli::DoctorArgs) -> Result<()> {
    let report = doctor_report(cli, config, args);
    if args.json {
        return doctor_output_report(&report, args.output.as_deref());
    }
    print_doctor_report(&report);
    Ok(())
}

fn doctor_report(cli: &Cli, config: &AppConfig, args: &cli::DoctorArgs) -> DoctorReport {
    let config_path = config_path_for_report(cli).map(|path| path.display().to_string());
    let mut checks = Vec::new();

    checks.push(doctor_check(
        "config",
        "pass",
        config_path
            .clone()
            .unwrap_or_else(|| "using default config discovery".to_string()),
    ));
    checks.push(doctor_check(
        "effective_config",
        "pass",
        effective_config_summary(config),
    ));
    checks.push(doctor_check(
        "tmdb_api_key",
        if config.tmdb.api_key.is_empty() {
            "warn"
        } else {
            "pass"
        },
        if config.tmdb.api_key.is_empty() {
            "TMDB API key not configured".to_string()
        } else {
            "TMDB API key is available".to_string()
        },
    ));

    if let Some(source) = &args.source {
        checks.push(doctor_check(
            "source",
            path_status(source),
            if source.exists() {
                format!("source exists: {}", source.display())
            } else {
                format!("source missing: {}", source.display())
            },
        ));
    }

    if let Some(destination) = &args.destination {
        let (status, detail) = destination_status(destination);
        checks.push(doctor_check("destination", status, detail));
    }

    checks.push(doctor_check(
        "media_extensions",
        "pass",
        media_extensions_summary(config),
    ));

    DoctorReport {
        config_path,
        tmdb_api_key_present: !config.tmdb.api_key.is_empty(),
        checks,
    }
}

fn print_doctor_report(report: &DoctorReport) {
    print_doctor_header(report);
    for check in &report.checks {
        print_doctor_check(check);
    }
    print_doctor_footer(report);
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

        parsed.push(item);
    }

    let skipped_show_groups = resolve_missing_show_fields(&mut parsed, yes_mode)?;

    resolve_missing_years(
        &mut parsed,
        args.year,
        MediaType::Show,
        yes_mode,
        &tmdb_client,
    )?;

    let mut plan = build_show_plan(
        &scan,
        &parsed,
        &args.destination,
        args.title.clone(),
        args.year,
        mode,
        non_media_mode,
    )?;
    if !skipped_show_groups.is_empty() {
        let skipped_paths: HashSet<_> = skipped_show_groups
            .iter()
            .map(|item| item.path.clone())
            .collect();
        plan.unparseable.retain(|item| {
            !(item.reason == UnparseableReason::MissingSeasonOrEpisode
                && skipped_paths.contains(&item.path))
        });
        plan.unparseable.extend(skipped_show_groups);
    }

    finalize_plan(args, config, &plan, true, dry_run, yes_mode)
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
        parsed.push(item);
    }

    resolve_missing_years(
        &mut parsed,
        args.year,
        MediaType::Movie,
        yes_mode,
        &tmdb_client,
    )?;

    let plan = build_movie_plan(
        &scan,
        &parsed,
        &args.destination,
        args.title.clone(),
        args.year,
        mode,
        non_media_mode,
    )?;

    finalize_plan(args, config, &plan, false, dry_run, yes_mode)
}

fn finalize_plan(
    args: &cli::ShowMovieArgs,
    config: &AppConfig,
    plan: &Plan,
    is_show: bool,
    dry_run: bool,
    yes_mode: bool,
) -> Result<()> {
    present_plan(plan, is_show, dry_run, &args.destination)?;

    if dry_run {
        return Ok(());
    }

    let conflict_policy = resolve_conflict_policy(args, config);
    preflight_conflicts(plan, conflict_policy)?;
    preflight_destination_access(plan)?;

    if !yes_mode && !prompt::confirm_execute()? {
        info!("Operation cancelled by user");
        return Ok(());
    }

    let result = executor::execute_plan(plan, conflict_policy == ConflictPolicy::Overwrite)?;
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

#[derive(Debug, Clone, Serialize)]
struct ScanDiagnostics {
    parser_mode: String,
    title_source: String,
    year_source: String,
    season_source: String,
    episode_source: String,
    issues: Vec<String>,
}

#[derive(Debug, Default)]
struct ScanSummary {
    parsed_ok: usize,
    parsed_failed: usize,
    with_year: usize,
    with_season: usize,
    with_episode: usize,
    detected_show: usize,
    detected_movie: usize,
    detected_unknown: usize,
    used_parent_fallback: usize,
    missing_title: usize,
    missing_year: usize,
    missing_season: usize,
    missing_episode: usize,
}

#[derive(Debug, Default, Serialize)]
struct IssueSummary {
    missing_title: usize,
    missing_year: usize,
    missing_season: usize,
    missing_episode: usize,
}

#[derive(Debug, Default, Serialize)]
struct FallbackSummary {
    title: usize,
    year: usize,
    season: usize,
    episode: usize,
}

#[derive(Debug, Default, Serialize)]
struct DetectedKindSummary {
    show: usize,
    movie: usize,
    unknown: usize,
}

#[derive(Debug, Default, Serialize)]
struct ParseDiagnosticsSummary {
    issue_summary: IssueSummary,
    fallback_summary: FallbackSummary,
    detected_kind_summary: DetectedKindSummary,
}

#[derive(Debug, Clone, Copy)]
struct ScanFieldSources {
    title: &'static str,
    year: &'static str,
    season: &'static str,
    episode: &'static str,
}

impl Default for ScanFieldSources {
    fn default() -> Self {
        Self {
            title: "missing",
            year: "missing",
            season: "missing",
            episode: "missing",
        }
    }
}

#[derive(Debug, Clone)]
struct ScanItemReport {
    info: MediaInfo,
    diagnostics: ScanDiagnostics,
}

#[derive(Debug, Clone)]
struct ScanContext<'a> {
    file: &'a scanner::ScannedFile,
    parser_mode: &'static str,
}

#[derive(Debug, Clone, Copy)]
struct ScanRenderOptions {
    verbose: bool,
}

#[derive(Debug, Clone, Serialize)]
struct DoctorCheck {
    name: String,
    status: &'static str,
    detail: String,
}

#[derive(Debug, Serialize)]
struct DoctorReport {
    config_path: Option<String>,
    tmdb_api_key_present: bool,
    checks: Vec<DoctorCheck>,
}

#[derive(Debug, Serialize)]
struct ScanJsonReport {
    source: String,
    filters: ScanFiltersJson,
    media_summary: MediaSummary,
    parse_summary: ParseSummary,
    diagnostics_summary: ParseDiagnosticsSummary,
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
    detected_show: usize,
    detected_movie: usize,
    detected_unknown: usize,
    used_parent_fallback: usize,
    missing_title: usize,
    missing_year: usize,
    missing_season: usize,
    missing_episode: usize,
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
    parser_mode: String,
    title_source: String,
    year_source: String,
    season_source: String,
    episode_source: String,
    issues: Vec<String>,
}

impl ScanItemReport {
    fn to_json_item(&self, file: &scanner::ScannedFile) -> ScanItemJson {
        ScanItemJson {
            file_name: file.file_name.clone(),
            source_path: file.path.display().to_string(),
            extension: file.extension.clone(),
            title: self.info.title.clone(),
            year: self.info.year,
            season: self.info.season,
            episode: self.info.episode,
            media_type: "video".to_string(),
            detected_kind: detected_kind(&self.info).to_string(),
            parse_confidence: parse_confidence(&self.info).to_string(),
            parser_mode: self.diagnostics.parser_mode.clone(),
            title_source: self.diagnostics.title_source.clone(),
            year_source: self.diagnostics.year_source.clone(),
            season_source: self.diagnostics.season_source.clone(),
            episode_source: self.diagnostics.episode_source.clone(),
            issues: self.diagnostics.issues.clone(),
        }
    }
}

fn summarize_diagnostics(items: &[ScanItemReport]) -> ParseDiagnosticsSummary {
    let mut out = ParseDiagnosticsSummary::default();
    for item in items {
        match detected_kind(&item.info) {
            "show" => out.detected_kind_summary.show += 1,
            "movie" => out.detected_kind_summary.movie += 1,
            _ => out.detected_kind_summary.unknown += 1,
        }
        if item.diagnostics.title_source == "parent" {
            out.fallback_summary.title += 1;
        }
        if item.diagnostics.year_source == "parent" {
            out.fallback_summary.year += 1;
        }
        if item.diagnostics.season_source == "parent" {
            out.fallback_summary.season += 1;
        }
        if item.diagnostics.episode_source == "parent" {
            out.fallback_summary.episode += 1;
        }
        for issue in &item.diagnostics.issues {
            match issue.as_str() {
                "missing_title" => out.issue_summary.missing_title += 1,
                "missing_year" => out.issue_summary.missing_year += 1,
                "missing_season" => out.issue_summary.missing_season += 1,
                "missing_episode" => out.issue_summary.missing_episode += 1,
                _ => {}
            }
        }
    }
    out
}

fn scan_item_report(file: &scanner::ScannedFile, hint: Option<ScanType>) -> ScanItemReport {
    let context = scan_parse_context(file, hint);
    let (info, sources) = parse_scan_info(&context, hint);
    let issues = scan_issues(&info);

    ScanItemReport {
        info,
        diagnostics: ScanDiagnostics {
            parser_mode: context.parser_mode.to_string(),
            title_source: sources.title.to_string(),
            year_source: sources.year.to_string(),
            season_source: sources.season.to_string(),
            episode_source: sources.episode.to_string(),
            issues,
        },
    }
}

fn scan_summary_from_reports(items: &[ScanItemReport]) -> ScanSummary {
    let mut out = ScanSummary::default();
    for item in items {
        let info = &item.info;
        if info.title.is_some() {
            out.parsed_ok += 1;
        } else {
            out.parsed_failed += 1;
        }
        if info.year.is_some() {
            out.with_year += 1;
        } else {
            out.missing_year += 1;
        }
        if info.season.is_some() {
            out.with_season += 1;
        } else {
            out.missing_season += 1;
        }
        if info.episode.is_some() {
            out.with_episode += 1;
        } else {
            out.missing_episode += 1;
        }
        if info.title.is_none() {
            out.missing_title += 1;
        }
        match detected_kind(info) {
            "show" => out.detected_show += 1,
            "movie" => out.detected_movie += 1,
            _ => out.detected_unknown += 1,
        }
        if item.diagnostics.title_source == "parent"
            || item.diagnostics.year_source == "parent"
            || item.diagnostics.season_source == "parent"
            || item.diagnostics.episode_source == "parent"
        {
            out.used_parent_fallback += 1;
        }
    }
    out
}

fn should_include_scan_report(
    report: &ScanItemReport,
    only_failed: bool,
    min_confidence_rank: u8,
) -> bool {
    if only_failed && report.info.title.is_some() {
        return false;
    }
    confidence_rank(parse_confidence(&report.info)) >= min_confidence_rank
}

fn print_scan_report(
    file: &scanner::ScannedFile,
    report: &ScanItemReport,
    options: ScanRenderOptions,
) {
    let confidence = parse_confidence(&report.info);
    let kind = detected_kind(&report.info);
    println!("  {}", file.file_name);
    println!(
        "    Title:   {}",
        report
            .info
            .title
            .clone()
            .unwrap_or_else(|| "(not found)".to_string())
    );
    println!(
        "    Year:    {}",
        report
            .info
            .year
            .map(|year| year.to_string())
            .unwrap_or_else(|| "(not found)".to_string())
    );
    println!(
        "    Season:  {}",
        report
            .info
            .season
            .map(|season| season.to_string())
            .unwrap_or_else(|| "(not found)".to_string())
    );
    println!(
        "    Episode: {}",
        report
            .info
            .episode
            .map(|episode| episode.to_string())
            .unwrap_or_else(|| "(not found)".to_string())
    );
    println!("    Type:    video");
    println!("    Kind:    {}", kind);
    println!("    Parse:   {}", confidence);
    if options.verbose {
        println!("    Parser:  {}", report.diagnostics.parser_mode);
        println!(
            "    Source:  title={}, year={}, season={}, episode={}",
            report.diagnostics.title_source,
            report.diagnostics.year_source,
            report.diagnostics.season_source,
            report.diagnostics.episode_source
        );
        if !report.diagnostics.issues.is_empty() {
            println!("    Issues:  {}", report.diagnostics.issues.join(", "));
        }
    }
    println!();
}

#[cfg(test)]
fn summarize_scan(items: &[MediaInfo]) -> ScanSummary {
    let reports = items
        .iter()
        .cloned()
        .map(|info| ScanItemReport {
            info,
            diagnostics: ScanDiagnostics {
                parser_mode: "test".to_string(),
                title_source: "missing".to_string(),
                year_source: "missing".to_string(),
                season_source: "missing".to_string(),
                episode_source: "missing".to_string(),
                issues: Vec::new(),
            },
        })
        .collect::<Vec<_>>();
    scan_summary_from_reports(&reports)
}

#[cfg(test)]
fn to_json_scan_item(file: &scanner::ScannedFile, info: &MediaInfo) -> ScanItemJson {
    ScanItemReport {
        info: info.clone(),
        diagnostics: ScanDiagnostics {
            parser_mode: "test".to_string(),
            title_source: "missing".to_string(),
            year_source: "missing".to_string(),
            season_source: "missing".to_string(),
            episode_source: "missing".to_string(),
            issues: Vec::new(),
        },
    }
    .to_json_item(file)
}

#[cfg(test)]
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

fn present_plan(plan: &Plan, is_show: bool, dry_run: bool, destination: &Path) -> Result<()> {
    let collection_name = infer_collection_name(plan, destination);

    if dry_run {
        if let Some(name) = collection_name {
            println!(
                "[DRY RUN] Organizing {}: {}",
                if is_show { "show" } else { "movie" },
                name
            );
        } else {
            println!(
                "[DRY RUN] Organizing {}",
                if is_show { "show" } else { "movie" }
            );
        }
        println!();
    }

    let conflicts: HashSet<_> = plan.conflicts.iter().collect();
    let mut grouped = BTreeMap::<String, Vec<(String, bool)>>::new();
    for op in &plan.operations {
        let relative = op
            .destination
            .strip_prefix(destination)
            .map(|path| path.to_path_buf())
            .unwrap_or_else(|_| op.destination.clone());

        let group = relative
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| ".".to_string());
        let file_name = relative
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| relative.display().to_string());

        grouped
            .entry(group)
            .or_default()
            .push((file_name, conflicts.contains(&op.destination)));
    }

    println!("Destination Preview:");
    for (group, files) in grouped.iter_mut() {
        files.sort_by(|a, b| a.0.cmp(&b.0));
        println!("  {}/", truncate_middle(group, 120));
        for (file_name, conflict) in files {
            let marker = if *conflict { "[CONFLICT]" } else { "[OK]" };
            println!("    {} {}", marker, truncate_middle(file_name, 96));
        }
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

    let skipped_group_items: Vec<_> = plan
        .unparseable
        .iter()
        .filter(|item| item.reason == UnparseableReason::UserSkippedInteractiveResolution)
        .collect();
    print_skipped_group_summaries(&summarize_skipped_groups(&skipped_group_items));

    let visible_unparseable: Vec<_> = plan
        .unparseable
        .iter()
        .filter(|item| item.reason.should_display())
        .collect();

    if !visible_unparseable.is_empty() {
        println!();
        for item in &visible_unparseable {
            println!(
                "SKIPPED: {} ({})",
                item.path.display(),
                item.reason.description()
            );
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
    let char_count = value.chars().count();
    if char_count <= max_len || max_len < 8 {
        return value.to_string();
    }

    let keep = (max_len - 3) / 2;
    let start: String = value.chars().take(keep).collect();
    let end: String = value
        .chars()
        .rev()
        .take(keep)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
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
        if let Some(year) = prompt::ask_for_year(Some(title))? {
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

fn resolve_missing_years(
    parsed: &mut [MediaInfo],
    forced_year: Option<u16>,
    media_type: MediaType,
    yes_mode: bool,
    lookup: &impl MetadataLookup,
) -> Result<()> {
    if let Some(year) = forced_year {
        for item in parsed.iter_mut() {
            if item.year.is_none() {
                item.year = Some(year);
            }
        }
        return Ok(());
    }

    let mut per_title_year = HashMap::<String, Option<u16>>::new();
    for item in parsed.iter() {
        if item.year.is_some() {
            continue;
        }
        if let Some(title) = item.title.as_ref() {
            per_title_year.entry(title.clone()).or_insert(None);
        }
    }

    if per_title_year.is_empty() {
        return Ok(());
    }

    if !yes_mode {
        for (title, year) in per_title_year.iter_mut() {
            if year.is_none() {
                *year = prompt::ask_for_year(Some(title))?;
            }
        }
    }

    for (title, year) in per_title_year.iter_mut() {
        if year.is_some() {
            continue;
        }
        *year = resolve_year(Some(title), media_type, true, lookup)?;
    }

    for item in parsed.iter_mut() {
        if item.year.is_some() {
            continue;
        }
        if let Some(title) = item.title.as_ref() {
            if let Some(year) = per_title_year.get(title).copied().flatten() {
                item.year = Some(year);
            }
        }
    }

    Ok(())
}

fn infer_collection_name(plan: &Plan, destination: &Path) -> Option<String> {
    let first = plan.operations.first()?;
    let relative = first.destination.strip_prefix(destination).ok()?;
    relative
        .components()
        .next()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
}

#[derive(Debug, Clone)]
struct ShowPromptGroup {
    parent_path: String,
    title: Option<String>,
    indices: Vec<usize>,
    missing_season: bool,
    missing_episode: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileGroupPreview {
    parent_path: String,
    file_count: usize,
    sample_files: Vec<String>,
    remaining_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SkippedGroupSummary {
    parent_path: String,
    file_count: usize,
    sample_files: Vec<String>,
}

const GROUP_PREVIEW_SAMPLE_LIMIT: usize = 3;

fn resolve_missing_show_fields(
    parsed: &mut [MediaInfo],
    yes_mode: bool,
) -> Result<Vec<UnparseableItem>> {
    if yes_mode {
        return Ok(Vec::new());
    }

    let groups = collect_show_prompt_groups(parsed);
    let mut skipped = Vec::new();

    for group in groups {
        print_show_group_preview(&build_group_preview(parsed, &group));

        let resolution = prompt::ask_for_show_group_metadata(&ShowGroupPrompt {
            title: group.title.clone(),
            parent_path: group.parent_path.clone(),
            file_count: group.indices.len(),
            missing_season: group.missing_season,
            missing_episode: group.missing_episode,
        })?;

        match resolution {
            Some(resolution) => apply_show_group_resolution(parsed, &group.indices, resolution),
            None => skipped.extend(group.indices.iter().filter_map(|index| {
                parsed[*index]
                    .full_path
                    .clone()
                    .map(|path| UnparseableItem {
                        path,
                        reason: UnparseableReason::UserSkippedInteractiveResolution,
                    })
            })),
        }
    }

    Ok(skipped)
}

fn build_group_preview(parsed: &[MediaInfo], group: &ShowPromptGroup) -> FileGroupPreview {
    let mut sample_files = group
        .indices
        .iter()
        .map(|index| parsed[*index].original_filename.clone())
        .collect::<Vec<_>>();
    sample_files.sort();
    let remaining_count = sample_files
        .len()
        .saturating_sub(GROUP_PREVIEW_SAMPLE_LIMIT);
    sample_files.truncate(GROUP_PREVIEW_SAMPLE_LIMIT);

    FileGroupPreview {
        parent_path: group.parent_path.clone(),
        file_count: group.indices.len(),
        sample_files,
        remaining_count,
    }
}

fn print_show_group_preview(preview: &FileGroupPreview) {
    println!("Group preview:");
    println!("  Folder: {}", truncate_middle(&preview.parent_path, 120));
    println!("  Files: {}", preview.file_count);
    if !preview.sample_files.is_empty() {
        println!("  Samples:");
        for file_name in &preview.sample_files {
            println!("    - {}", truncate_middle(file_name, 96));
        }
        if preview.remaining_count > 0 {
            println!("    - ... and {} more", preview.remaining_count);
        }
    }
    println!();
}

fn summarize_skipped_groups(items: &[&UnparseableItem]) -> Vec<SkippedGroupSummary> {
    let mut grouped = BTreeMap::<String, Vec<String>>::new();

    for item in items {
        let parent_path = item
            .path
            .parent()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| ".".to_string());
        let file_name = item
            .path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| item.path.display().to_string());
        grouped.entry(parent_path).or_default().push(file_name);
    }

    grouped
        .into_iter()
        .map(|(parent_path, mut file_names)| {
            file_names.sort();
            let file_count = file_names.len();
            file_names.truncate(GROUP_PREVIEW_SAMPLE_LIMIT);
            SkippedGroupSummary {
                parent_path,
                file_count,
                sample_files: file_names,
            }
        })
        .collect()
}

fn print_skipped_group_summaries(groups: &[SkippedGroupSummary]) {
    if groups.is_empty() {
        return;
    }

    println!();
    println!("Skipped Groups:");
    for group in groups {
        let file_label = if group.file_count == 1 {
            "file"
        } else {
            "files"
        };
        println!(
            "  - {}: {} {}",
            truncate_middle(&group.parent_path, 120),
            group.file_count,
            file_label
        );
        if !group.sample_files.is_empty() {
            println!(
                "    Samples: {}",
                group
                    .sample_files
                    .iter()
                    .map(|name| truncate_middle(name, 96))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
}

fn collect_show_prompt_groups(parsed: &[MediaInfo]) -> Vec<ShowPromptGroup> {
    let mut grouped = BTreeMap::<String, ShowPromptGroup>::new();

    for (index, item) in parsed.iter().enumerate() {
        if item.season.is_some() && item.episode.is_some() {
            continue;
        }

        let parent_path = item
            .full_path
            .as_ref()
            .and_then(|path| path.parent())
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| ".".to_string());

        let group = grouped
            .entry(parent_path.clone())
            .or_insert_with(|| ShowPromptGroup {
                parent_path,
                title: item.title.clone(),
                indices: Vec::new(),
                missing_season: false,
                missing_episode: false,
            });

        if group.title.is_none() {
            group.title = item.title.clone();
        }
        group.indices.push(index);
        group.missing_season |= item.season.is_none();
        group.missing_episode |= item.episode.is_none();
    }

    grouped.into_values().collect()
}

fn apply_show_group_resolution(
    parsed: &mut [MediaInfo],
    indices: &[usize],
    resolution: ShowGroupResolution,
) {
    for index in indices {
        let item = &mut parsed[*index];
        if item.season.is_none() {
            item.season = resolution.season;
        }
        if item.episode.is_none() {
            item.episode = resolution.episode;
        }
    }
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
        apply_show_group_resolution, build_group_preview, collect_show_prompt_groups,
        confidence_arg_to_rank, confidence_rank, detected_kind, infer_collection_name,
        is_read_only_dir, parse_confidence, preflight_conflicts, preflight_destination_access,
        resolve_conflict_policy, resolve_missing_show_fields, resolve_missing_years,
        should_include_scan_item, summarize_scan, summarize_skipped_groups, to_json_scan_item,
        truncate_middle, ConflictPolicy,
    };
    use crate::config::{AppConfig, ConflictMode};
    use crate::parser::MediaInfo;
    use crate::planner::{Operation, OperationKind, Plan};
    use crate::prompt::ShowGroupResolution;
    use crate::tmdb::MetadataLookup;
    use tempfile::tempdir;

    struct FailingLookup;

    struct FixedLookup {
        year: Option<u16>,
    }

    impl MetadataLookup for FailingLookup {
        fn lookup_year(
            &self,
            _title: &str,
            _media_type: crate::config::MediaType,
        ) -> anyhow::Result<Option<u16>> {
            anyhow::bail!("network unavailable")
        }
    }

    impl MetadataLookup for FixedLookup {
        fn lookup_year(
            &self,
            _title: &str,
            _media_type: crate::config::MediaType,
        ) -> anyhow::Result<Option<u16>> {
            Ok(self.year)
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
    fn truncate_middle_handles_utf8_without_panicking() {
        let value = "/movies/अंदाज़-अपना-अपना/épisode-finale-東京.mkv";
        let out = truncate_middle(value, 18);
        assert!(out.contains("..."));
        assert!(out.chars().count() <= 18);
    }

    #[test]
    fn resolve_missing_years_applies_forced_year_to_all_missing() {
        let mut items = vec![
            MediaInfo {
                title: Some("Game Changer".to_string()),
                year: None,
                ..Default::default()
            },
            MediaInfo {
                title: Some("Game Changer".to_string()),
                year: Some(2018),
                ..Default::default()
            },
        ];

        resolve_missing_years(
            &mut items,
            Some(2019),
            crate::config::MediaType::Show,
            true,
            &FailingLookup,
        )
        .expect("forced year should resolve without lookup");

        assert_eq!(items[0].year, Some(2019));
        assert_eq!(items[1].year, Some(2018));
    }

    #[test]
    fn resolve_missing_years_uses_lookup_when_unresolved() {
        let mut items = vec![
            MediaInfo {
                title: Some("Game Changer".to_string()),
                year: None,
                ..Default::default()
            },
            MediaInfo {
                title: Some("Game Changer".to_string()),
                year: None,
                ..Default::default()
            },
        ];

        resolve_missing_years(
            &mut items,
            None,
            crate::config::MediaType::Show,
            true,
            &FixedLookup { year: Some(2019) },
        )
        .expect("lookup should resolve shared title year");

        assert_eq!(items[0].year, Some(2019));
        assert_eq!(items[1].year, Some(2019));
    }

    #[test]
    fn infer_collection_name_uses_first_destination_component() {
        let destination = std::path::PathBuf::from("/library/shows");
        let mut plan = Plan::default();
        plan.operations.push(Operation {
            source: std::path::PathBuf::from("/downloads/Game.Changer.S01E01.mkv"),
            destination: destination
                .join("Game Changer (2019)")
                .join("Season 01")
                .join("Game.Changer.S01E01.mkv"),
            kind: OperationKind::Move,
        });

        let folder = infer_collection_name(&plan, &destination);
        assert_eq!(folder.as_deref(), Some("Game Changer (2019)"));
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
    fn collect_show_prompt_groups_groups_by_parent_folder() {
        let items = vec![
            MediaInfo {
                title: Some("Show".to_string()),
                season: None,
                episode: Some(1),
                full_path: Some(std::path::PathBuf::from("/tmp/downloads/show/ep1.mkv")),
                ..Default::default()
            },
            MediaInfo {
                title: Some("Show".to_string()),
                season: None,
                episode: Some(2),
                full_path: Some(std::path::PathBuf::from("/tmp/downloads/show/ep2.mkv")),
                ..Default::default()
            },
            MediaInfo {
                title: Some("Other".to_string()),
                season: Some(1),
                episode: None,
                full_path: Some(std::path::PathBuf::from("/tmp/downloads/other/ep1.mkv")),
                ..Default::default()
            },
        ];

        let groups = collect_show_prompt_groups(&items);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].indices, vec![2]);
        assert_eq!(groups[0].parent_path, "/tmp/downloads/other");
        assert!(!groups[0].missing_season);
        assert!(groups[0].missing_episode);
        assert_eq!(groups[1].indices, vec![0, 1]);
        assert_eq!(groups[1].parent_path, "/tmp/downloads/show");
        assert!(groups[1].missing_season);
        assert!(!groups[1].missing_episode);
    }

    #[test]
    fn build_group_preview_collects_sorted_samples_and_remaining_count() {
        let items = vec![
            MediaInfo {
                original_filename: "c.mkv".to_string(),
                ..Default::default()
            },
            MediaInfo {
                original_filename: "a.mkv".to_string(),
                ..Default::default()
            },
            MediaInfo {
                original_filename: "d.mkv".to_string(),
                ..Default::default()
            },
            MediaInfo {
                original_filename: "b.mkv".to_string(),
                ..Default::default()
            },
        ];
        let group = super::ShowPromptGroup {
            parent_path: "/tmp/downloads/show".to_string(),
            title: Some("Show".to_string()),
            indices: vec![0, 1, 2, 3],
            missing_season: true,
            missing_episode: false,
        };

        let preview = build_group_preview(&items, &group);
        assert_eq!(preview.parent_path, "/tmp/downloads/show");
        assert_eq!(preview.file_count, 4);
        assert_eq!(preview.sample_files, vec!["a.mkv", "b.mkv", "c.mkv"]);
        assert_eq!(preview.remaining_count, 1);
    }

    #[test]
    fn summarize_skipped_groups_groups_by_parent_folder() {
        let items = [
            crate::planner::UnparseableItem {
                path: std::path::PathBuf::from("/tmp/downloads/show/a.mkv"),
                reason: crate::planner::UnparseableReason::UserSkippedInteractiveResolution,
            },
            crate::planner::UnparseableItem {
                path: std::path::PathBuf::from("/tmp/downloads/show/b.mkv"),
                reason: crate::planner::UnparseableReason::UserSkippedInteractiveResolution,
            },
            crate::planner::UnparseableItem {
                path: std::path::PathBuf::from("/tmp/downloads/other/c.mkv"),
                reason: crate::planner::UnparseableReason::UserSkippedInteractiveResolution,
            },
        ];
        let refs = items.iter().collect::<Vec<_>>();

        let groups = summarize_skipped_groups(&refs);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].parent_path, "/tmp/downloads/other");
        assert_eq!(groups[0].file_count, 1);
        assert_eq!(groups[0].sample_files, vec!["c.mkv"]);
        assert_eq!(groups[1].parent_path, "/tmp/downloads/show");
        assert_eq!(groups[1].file_count, 2);
        assert_eq!(groups[1].sample_files, vec!["a.mkv", "b.mkv"]);
    }

    #[test]
    fn apply_show_group_resolution_only_fills_missing_fields() {
        let mut items = vec![
            MediaInfo {
                season: Some(4),
                episode: None,
                ..Default::default()
            },
            MediaInfo {
                season: None,
                episode: Some(9),
                ..Default::default()
            },
        ];

        apply_show_group_resolution(
            &mut items,
            &[0, 1],
            ShowGroupResolution {
                season: Some(2),
                episode: Some(7),
            },
        );

        assert_eq!(items[0].season, Some(4));
        assert_eq!(items[0].episode, Some(7));
        assert_eq!(items[1].season, Some(2));
        assert_eq!(items[1].episode, Some(9));
    }

    #[test]
    fn resolve_missing_show_fields_yes_mode_skips_prompting() {
        let mut items = vec![MediaInfo {
            title: Some("Show".to_string()),
            season: None,
            episode: None,
            full_path: Some(std::path::PathBuf::from("/tmp/downloads/show/ep1.mkv")),
            ..Default::default()
        }];

        let skipped =
            resolve_missing_show_fields(&mut items, true).expect("yes mode should not fail");
        assert!(skipped.is_empty());
        assert_eq!(items[0].season, None);
        assert_eq!(items[0].episode, None);
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
