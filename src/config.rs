use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::cli::{Cli, Commands, NonMediaArg, ShowMovieArgs};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    Show,
    Movie,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectiveOperationMode {
    Move,
    Copy,
    HardLink,
    SymLink,
}

impl EffectiveOperationMode {
    pub fn from_args_and_config(args: &ShowMovieArgs, config: &AppConfig) -> Self {
        if args.copy {
            return Self::Copy;
        }
        if args.link {
            return Self::HardLink;
        }
        if args.symlink {
            return Self::SymLink;
        }
        match config.general.default_mode.as_str() {
            "copy" => Self::Copy,
            "hardlink" => Self::HardLink,
            "symlink" => Self::SymLink,
            _ => Self::Move,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub general: GeneralConfig,
    pub tmdb: TmdbConfig,
    pub naming: NamingConfig,
    pub media_extensions: MediaExtensions,
    pub non_media: NonMediaConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            tmdb: TmdbConfig::default(),
            naming: NamingConfig::default(),
            media_extensions: MediaExtensions::default(),
            non_media: NonMediaConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn load_and_merge(cli: &Cli) -> Result<Self> {
        let mut cfg = AppConfig::default();

        if let Some(global_path) = global_config_path() {
            if global_path.exists() {
                let incoming = load_file(&global_path)?;
                cfg.merge(incoming);
            }
        }

        let local_path = PathBuf::from(".organize.toml");
        if local_path.exists() {
            let incoming = load_file(&local_path)?;
            cfg.merge(incoming);
        }

        if let Some(path) = &cli.config {
            let incoming = load_file(path)?;
            cfg.merge(incoming);
        }

        if let Some(path) = &cli.log_file {
            cfg.general.log_file = Some(path.clone());
        }
        if cli.yes {
            cfg.general.auto_confirm = true;
        }

        match &cli.command {
            Commands::Show(args) | Commands::Movie(args) => {
                if args.clean {
                    cfg.general.clean_empty_dirs = true;
                }
                if let Some(mode) = args.non_media {
                    cfg.non_media.mode = match mode {
                        NonMediaArg::Keep => NonMediaMode::Keep,
                        NonMediaArg::Ignore => NonMediaMode::Ignore,
                    };
                }
                if args.yes {
                    cfg.general.auto_confirm = true;
                }
            }
            Commands::Scan(_) => {}
        }

        if cfg.tmdb.api_key.is_empty() {
            if let Ok(env_key) = std::env::var("TMDB_API_KEY") {
                cfg.tmdb.api_key = env_key;
            }
        }

        Ok(cfg)
    }

    fn merge(&mut self, incoming: PartialConfig) {
        if let Some(g) = incoming.general {
            self.general.merge(g);
        }
        if let Some(t) = incoming.tmdb {
            self.tmdb.merge(t);
        }
        if let Some(n) = incoming.naming {
            self.naming.merge(n);
        }
        if let Some(m) = incoming.media_extensions {
            self.media_extensions.merge(m);
        }
        if let Some(nm) = incoming.non_media {
            self.non_media.merge(nm);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    pub default_mode: String,
    pub auto_confirm: bool,
    pub clean_empty_dirs: bool,
    pub log_file: Option<PathBuf>,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            default_mode: "move".to_string(),
            auto_confirm: false,
            clean_empty_dirs: false,
            log_file: None,
        }
    }
}

impl GeneralConfig {
    fn merge(&mut self, incoming: PartialGeneralConfig) {
        if let Some(v) = incoming.default_mode {
            self.default_mode = v;
        }
        if let Some(v) = incoming.auto_confirm {
            self.auto_confirm = v;
        }
        if let Some(v) = incoming.clean_empty_dirs {
            self.clean_empty_dirs = v;
        }
        if let Some(v) = incoming.log_file {
            self.log_file = if v.is_empty() { None } else { Some(v.into()) };
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TmdbConfig {
    pub api_key: String,
}

impl TmdbConfig {
    fn merge(&mut self, incoming: PartialTmdbConfig) {
        if let Some(v) = incoming.api_key {
            self.api_key = v;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamingConfig {
    pub show_folder: String,
    pub season_folder: String,
    pub movie_folder: String,
}

impl Default for NamingConfig {
    fn default() -> Self {
        Self {
            show_folder: "{Series Title} ({Series Year})".to_string(),
            season_folder: "Season {Season:00}".to_string(),
            movie_folder: "{Movie Title} ({Movie Year})".to_string(),
        }
    }
}

impl NamingConfig {
    fn merge(&mut self, incoming: PartialNamingConfig) {
        if let Some(v) = incoming.show_folder {
            self.show_folder = v;
        }
        if let Some(v) = incoming.season_folder {
            self.season_folder = v;
        }
        if let Some(v) = incoming.movie_folder {
            self.movie_folder = v;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaExtensions {
    pub video: Vec<String>,
    pub subtitle: Vec<String>,
    pub audio: Vec<String>,
}

impl Default for MediaExtensions {
    fn default() -> Self {
        Self {
            video: vec![
                ".mkv", ".mp4", ".avi", ".m4v", ".wmv", ".flv", ".webm", ".ts", ".m2ts",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            subtitle: vec![".srt", ".ass", ".ssa", ".sub", ".idx", ".vtt", ".sup", ".smi"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            audio: vec![
                ".mp3", ".flac", ".aac", ".ac3", ".dts", ".mka", ".ogg", ".wav", ".wma", ".eac3",
                ".m4a",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }
}

impl MediaExtensions {
    fn merge(&mut self, incoming: PartialMediaExtensions) {
        if let Some(v) = incoming.video {
            self.video = v;
        }
        if let Some(v) = incoming.subtitle {
            self.subtitle = v;
        }
        if let Some(v) = incoming.audio {
            self.audio = v;
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NonMediaMode {
    Keep,
    Ignore,
}

impl NonMediaMode {
    pub fn to_scanner_mode(self) -> crate::scanner::NonMediaPolicy {
        match self {
            NonMediaMode::Keep => crate::scanner::NonMediaPolicy::Keep,
            NonMediaMode::Ignore => crate::scanner::NonMediaPolicy::Ignore,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NonMediaConfig {
    pub mode: NonMediaMode,
}

impl Default for NonMediaConfig {
    fn default() -> Self {
        Self {
            mode: NonMediaMode::Keep,
        }
    }
}

impl NonMediaConfig {
    fn merge(&mut self, incoming: PartialNonMediaConfig) {
        if let Some(v) = incoming.mode {
            self.mode = v;
        }
    }
}

#[derive(Debug, Deserialize)]
struct PartialConfig {
    general: Option<PartialGeneralConfig>,
    tmdb: Option<PartialTmdbConfig>,
    naming: Option<PartialNamingConfig>,
    media_extensions: Option<PartialMediaExtensions>,
    non_media: Option<PartialNonMediaConfig>,
}

#[derive(Debug, Deserialize)]
struct PartialGeneralConfig {
    default_mode: Option<String>,
    auto_confirm: Option<bool>,
    clean_empty_dirs: Option<bool>,
    log_file: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PartialTmdbConfig {
    api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PartialNamingConfig {
    show_folder: Option<String>,
    season_folder: Option<String>,
    movie_folder: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PartialMediaExtensions {
    video: Option<Vec<String>>,
    subtitle: Option<Vec<String>>,
    audio: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct PartialNonMediaConfig {
    mode: Option<NonMediaMode>,
}

fn load_file(path: &PathBuf) -> Result<PartialConfig> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;
    let parsed = toml::from_str::<PartialConfig>(&content)
        .with_context(|| format!("failed to parse config file {}", path.display()))?;
    Ok(parsed)
}

fn global_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("organize").join("config.toml"))
}
