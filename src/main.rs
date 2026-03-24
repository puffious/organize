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
use config::{AppConfig, EffectiveOperationMode, MediaType, NonMediaMode};
use parser::{parse_movie, parse_show, MediaInfo};
use planner::{build_movie_plan, build_show_plan, Plan};
use scanner::scan_source;
use std::collections::HashSet;
use std::path::Path;
use tmdb::MetadataLookup;
use tracing::{info, warn};

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

    if !yes_mode && !prompt::confirm_execute()? {
        info!("Operation cancelled by user");
        return Ok(());
    }

    let result = executor::execute_plan(&plan, args.overwrite)?;
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

    if !yes_mode && !prompt::confirm_execute()? {
        info!("Operation cancelled by user");
        return Ok(());
    }

    let result = executor::execute_plan(&plan, args.overwrite)?;
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

    println!("Scanning: {}", args.source.display());
    println!();

    let mut parsed_ok = 0usize;
    let mut parsed_failed = 0usize;

    for file in &scan.video_files {
        let parsed = match hint {
            Some(ScanType::Show) => parse_show(&file.file_name),
            Some(ScanType::Movie) => parse_movie(&file.file_name),
            None => auto_parse(&file.file_name),
        };

        print_scan_item(file, &parsed);

        if parsed.title.is_some() {
            parsed_ok += 1;
        } else {
            parsed_failed += 1;
        }
    }

    let total = scan.video_files.len();
    println!(
        "Summary: {} files scanned, {} parsed successfully, {} failed",
        total, parsed_ok, parsed_failed
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
    println!();
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

    lookup.lookup_year(title, media_type)
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
    use super::truncate_middle;

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
}
