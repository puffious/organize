use std::path::PathBuf;

use clap::{ArgAction, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(name = "organize", version, about = "Media file organizer for shows and movies")]
pub struct Cli {
    #[arg(short = 'v', action = ArgAction::Count, global = true)]
    pub verbose: u8,

    #[arg(long, global = true)]
    pub dry_run: bool,

    #[arg(long, global = true)]
    pub yes: bool,

    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    #[arg(long, global = true)]
    pub log_file: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    Show(ShowMovieArgs),
    Movie(ShowMovieArgs),
    Scan(ScanArgs),
}

#[derive(Debug, clap::Args, Clone)]
pub struct ShowMovieArgs {
    pub source: PathBuf,
    pub destination: PathBuf,

    #[arg(long, conflicts_with_all = ["link", "symlink"])]
    pub copy: bool,

    #[arg(long, conflicts_with_all = ["copy", "symlink"])]
    pub link: bool,

    #[arg(long, conflicts_with_all = ["copy", "link"])]
    pub symlink: bool,

    #[arg(long)]
    pub overwrite: bool,

    #[arg(long, value_enum)]
    pub on_conflict: Option<ConflictArg>,

    #[arg(long)]
    pub clean: bool,

    #[arg(long)]
    pub title: Option<String>,

    #[arg(long)]
    pub year: Option<u16>,

    #[arg(long, value_enum)]
    pub non_media: Option<NonMediaArg>,

    #[arg(long, default_value_t = false)]
    pub dry_run: bool,

    #[arg(long, default_value_t = false)]
    pub yes: bool,
}

#[derive(Debug, clap::Args, Clone)]
pub struct ScanArgs {
    pub source: PathBuf,

    #[arg(long = "type", value_enum)]
    pub r#type: Option<ScanType>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ScanType {
    Show,
    Movie,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum NonMediaArg {
    Keep,
    Ignore,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ConflictArg {
    Skip,
    Overwrite,
    Abort,
}