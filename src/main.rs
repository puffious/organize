mod cli;
mod config;
mod logging;
mod executor;
mod parser;
mod planner;
mod prompt;
mod scanner;
mod tmdb;

use anyhow::{Context, Result};
use clap::Parser;
use cli::{Cli, Commands, ScanType};
use config::{AppConfig, EffectiveOperationMode, MediaType, NonMediaMode};
use parser::{parse_movie, parse_show, MediaInfo};
use planner::{build_movie_plan, build_show_plan, Plan};
use scanner::scan_source;
use tmdb::MetadataLookup;
use tracing::{info, warn};

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = AppConfig::load_and_merge(&cli)?;
    logging::init_logging(cli.verbose, config.general.log_file.as_deref())?;

    match &cli.command {
        Commands::Show(args) => run_show(args, &config, cli.yes),
        Commands::Movie(args) => run_movie(args, &config, cli.yes),
        Commands::Scan(args) => run_scan(args, &config),
    }
}

fn run_show(args: &cli::ShowMovieArgs, config: &AppConfig, cli_yes: bool) -> Result<()> {
    let scan = scan_source(&args.source, &config.media_extensions)?;
    let non_media_mode = args
        .non_media
        .unwrap_or(config.non_media.mode)
        .to_scanner_mode();
    let mode = EffectiveOperationMode::from_args_and_config(args, config);
    let yes_mode = cli_yes || args.yes || config.general.auto_confirm;
    let tmdb_client = tmdb::TmdbClient::new(config.tmdb.api_key.clone());

    let parsed = scan
        .video_files
        .iter()
        .map(|f| {
            let mut parsed = parse_show(&f.file_name);
            parsed.original_filename = f.file_name.clone();
            parsed.extension = f.extension.clone();
            parsed.full_path = Some(f.path.clone());

            if parsed.year.is_none() {
                parsed.year = parser::extract_year_from_input(&f.parent_name);
            }
            if parsed.season.is_none() {
                parsed.season = parser::extract_season_from_input(&f.parent_name);
            }

            if parsed.year.is_none() {
                parsed.year = resolve_year(
                    parsed.title.as_deref(),
                    MediaType::Show,
                    yes_mode,
                    &tmdb_client,
                )?;
            }

            if parsed.season.is_none() || parsed.episode.is_none() {
                resolve_season_episode_if_missing(&mut parsed, yes_mode)?;
            }

            parsed
        })
        .collect::<Vec<_>>();

    let plan = build_show_plan(
        &scan,
        &parsed,
        &args.destination,
        args.title.clone(),
        args.year,
        mode,
        non_media_mode,
    )?;

    present_plan(&plan, true, args.dry_run || cli_dry_run(config))?;

    if args.dry_run || cli_dry_run(config) {
        return Ok(());
    }

    if !yes_mode && !prompt::confirm_execute()? {
        info!("Operation cancelled by user");
        return Ok(());
    }

    let result = executor::execute_plan(&plan, args.overwrite)?;

    if args.clean || config.general.clean_empty_dirs {
        scanner::clean_empty_dirs(&args.source)?;
    }

    result.into_exit_result()
}

fn run_movie(args: &cli::ShowMovieArgs, config: &AppConfig, cli_yes: bool) -> Result<()> {
    let scan = scan_source(&args.source, &config.media_extensions)?;
    let non_media_mode = args
        .non_media
        .unwrap_or(config.non_media.mode)
        .to_scanner_mode();
    let mode = EffectiveOperationMode::from_args_and_config(args, config);
    let yes_mode = cli_yes || args.yes || config.general.auto_confirm;
    let tmdb_client = tmdb::TmdbClient::new(config.tmdb.api_key.clone());

    let parsed = scan
        .video_files
        .iter()
        .map(|f| {
            let mut parsed = parse_movie(&f.file_name);
            parsed.original_filename = f.file_name.clone();
            parsed.extension = f.extension.clone();
            parsed.full_path = Some(f.path.clone());
            if parsed.year.is_none() {
                parsed.year = parser::extract_year_from_input(&f.parent_name);
            }
            if parsed.year.is_none() {
                parsed.year = resolve_year(
                    parsed.title.as_deref(),
                    MediaType::Movie,
                    yes_mode,
                    &tmdb_client,
                )?;
            }
            parsed
        })
        .collect::<Vec<_>>();

    let plan = build_movie_plan(
        &scan,
        &parsed,
        &args.destination,
        args.title.clone(),
        args.year,
        mode,
        non_media_mode,
    )?;

    present_plan(&plan, false, args.dry_run || cli_dry_run(config))?;

    if args.dry_run || cli_dry_run(config) {
        return Ok(());
    }

    if !yes_mode && !prompt::confirm_execute()? {
        info!("Operation cancelled by user");
        return Ok(());
    }

    let result = executor::execute_plan(&plan, args.overwrite)?;

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

fn present_plan(plan: &Plan, is_show: bool, dry_run: bool) -> Result<()> {
    if dry_run {
        println!(
            "[DRY RUN] Organizing {}",
            if is_show { "show" } else { "movie" }
        );
        println!();
    }

    for op in &plan.operations {
        println!("{} -> {}", op.source.display(), op.destination.display());
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

fn cli_dry_run(_config: &AppConfig) -> bool {
    false
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
