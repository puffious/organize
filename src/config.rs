use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::cli::{Cli, Commands, NonMediaArg, ShowMovieArgs};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    Show,
    Movie,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EffectiveOperationMode {
    Move,
    Copy,
    HardLink,
    SymLink,
}

impl Default for EffectiveOperationMode {
    fn default() -> Self {
        Self::Move
    }
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
        config.general.default_mode
    }
}

fn normalize_extensions(values: &mut [String]) {
    for value in values {
        *value = value.trim().to_ascii_lowercase();
        if !value.is_empty() && !value.starts_with('.') {
            value.insert(0, '.');
        }
    }
}

fn normalize_api_key(value: &mut String) {
    *value = value.trim().to_string();
}

fn normalize_log_file(value: &mut Option<PathBuf>) {
    if value
        .as_ref()
        .is_some_and(|path| path.as_os_str().is_empty())
    {
        *value = None;
    }
}

fn deserialize_optional_mode<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<EffectiveOperationMode>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    raw.map(|value| match value.trim().to_ascii_lowercase().as_str() {
        "move" => Ok(EffectiveOperationMode::Move),
        "copy" => Ok(EffectiveOperationMode::Copy),
        "hardlink" => Ok(EffectiveOperationMode::HardLink),
        "symlink" => Ok(EffectiveOperationMode::SymLink),
        other => Err(serde::de::Error::custom(format!(
            "invalid default_mode '{other}', expected one of: move, copy, hardlink, symlink"
        ))),
    })
    .transpose()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub general: GeneralConfig,
    pub tmdb: TmdbConfig,
    pub media_extensions: MediaExtensions,
    pub non_media: NonMediaConfig,
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

        normalize_log_file(&mut cfg.general.log_file);

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
            Commands::Scan(_) | Commands::Doctor(_) => {}
        }

        normalize_api_key(&mut cfg.tmdb.api_key);
        normalize_extensions(&mut cfg.media_extensions.video);
        normalize_extensions(&mut cfg.media_extensions.subtitle);
        normalize_extensions(&mut cfg.media_extensions.audio);

        if cfg.tmdb.api_key.is_empty() {
            if let Ok(env_key) = std::env::var("TMDB_API_KEY") {
                cfg.tmdb.api_key = env_key;
                normalize_api_key(&mut cfg.tmdb.api_key);
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
    pub default_mode: EffectiveOperationMode,
    pub auto_confirm: bool,
    pub clean_empty_dirs: bool,
    pub conflict_mode: ConflictMode,
    pub log_file: Option<PathBuf>,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            default_mode: EffectiveOperationMode::Move,
            auto_confirm: false,
            clean_empty_dirs: false,
            conflict_mode: ConflictMode::Skip,
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
        if let Some(v) = incoming.conflict_mode {
            self.conflict_mode = v;
        }
        if let Some(v) = incoming.log_file {
            self.log_file = Some(v.into());
            normalize_log_file(&mut self.log_file);
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConflictMode {
    Skip,
    Overwrite,
    Abort,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TmdbConfig {
    pub api_key: String,
}

impl TmdbConfig {
    fn merge(&mut self, incoming: PartialTmdbConfig) {
        if let Some(mut v) = incoming.api_key {
            normalize_api_key(&mut v);
            self.api_key = v;
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
            subtitle: vec![
                ".srt", ".ass", ".ssa", ".sub", ".idx", ".vtt", ".sup", ".smi",
            ]
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
        if let Some(mut v) = incoming.video {
            normalize_extensions(&mut v);
            self.video = v;
        }
        if let Some(mut v) = incoming.subtitle {
            normalize_extensions(&mut v);
            self.subtitle = v;
        }
        if let Some(mut v) = incoming.audio {
            normalize_extensions(&mut v);
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
    media_extensions: Option<PartialMediaExtensions>,
    non_media: Option<PartialNonMediaConfig>,
}

#[derive(Debug, Deserialize)]
struct PartialGeneralConfig {
    #[serde(default, deserialize_with = "deserialize_optional_mode")]
    default_mode: Option<EffectiveOperationMode>,
    auto_confirm: Option<bool>,
    clean_empty_dirs: Option<bool>,
    conflict_mode: Option<ConflictMode>,
    #[serde(default)]
    log_file: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PartialTmdbConfig {
    api_key: Option<String>,
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

pub fn global_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("organize").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::{
        load_file, ConflictMode, EffectiveOperationMode, GeneralConfig, MediaExtensions,
        PartialGeneralConfig,
    };
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn general_merge_applies_conflict_mode_override() {
        let mut general = GeneralConfig::default();
        assert_eq!(general.conflict_mode, ConflictMode::Skip);

        general.merge(PartialGeneralConfig {
            default_mode: None,
            auto_confirm: None,
            clean_empty_dirs: None,
            conflict_mode: Some(ConflictMode::Abort),
            log_file: None,
        });

        assert_eq!(general.conflict_mode, ConflictMode::Abort);
    }

    #[test]
    fn general_merge_ignores_absent_fields() {
        let mut general = GeneralConfig {
            default_mode: EffectiveOperationMode::Copy,
            auto_confirm: true,
            ..Default::default()
        };

        general.merge(PartialGeneralConfig {
            default_mode: None,
            auto_confirm: None,
            clean_empty_dirs: None,
            conflict_mode: None,
            log_file: None,
        });

        assert_eq!(general.default_mode, EffectiveOperationMode::Copy);
        assert!(general.auto_confirm);
        assert_eq!(general.conflict_mode, ConflictMode::Skip);
    }

    #[test]
    fn media_extensions_merge_normalizes_values() {
        let mut exts = MediaExtensions::default();
        exts.merge(super::PartialMediaExtensions {
            video: Some(vec![" MKV ".to_string(), "mp4".to_string()]),
            subtitle: None,
            audio: Some(vec![" FlAc ".to_string()]),
        });

        assert_eq!(exts.video, vec![".mkv", ".mp4"]);
        assert_eq!(exts.audio, vec![".flac"]);
    }

    #[test]
    fn load_file_rejects_invalid_default_mode() {
        let dir = tempdir().expect("create tempdir");
        let path = dir.path().join("invalid.toml");
        fs::write(&path, "[general]\ndefault_mode = \"teleport\"\n").expect("write invalid config");

        load_file(&path).expect_err("invalid default_mode should fail parsing");
    }
}
