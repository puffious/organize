use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use walkdir::WalkDir;

use crate::config::MediaExtensions;

#[derive(Debug, Clone, Copy)]
pub enum NonMediaPolicy {
    Keep,
    Ignore,
}

#[derive(Debug, Clone)]
pub struct ScannedFile {
    pub path: PathBuf,
    pub file_name: String,
    pub parent_name: String,
    pub extension: String,
}

#[derive(Debug, Clone, Default)]
pub struct ScanResult {
    pub video_files: Vec<ScannedFile>,
    pub subtitle_files: Vec<ScannedFile>,
    pub audio_files: Vec<ScannedFile>,
    pub other_files: Vec<ScannedFile>,
}

pub fn scan_source(source: &Path, exts: &MediaExtensions) -> Result<ScanResult> {
    if !source.exists() {
        anyhow::bail!("source path does not exist: {}", source.display());
    }

    let mut out = ScanResult::default();

    for entry in WalkDir::new(source)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path().to_path_buf();
        let file_name = entry.file_name().to_string_lossy().to_string();
        let parent_name = entry
            .path()
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let extension = lower_ext(&path);
        let item = ScannedFile {
            path,
            file_name,
            parent_name,
            extension: extension.clone(),
        };

        if exts.video.iter().any(|e| e.eq_ignore_ascii_case(&extension)) {
            out.video_files.push(item);
        } else if exts
            .subtitle
            .iter()
            .any(|e| e.eq_ignore_ascii_case(&extension))
        {
            out.subtitle_files.push(item);
        } else if exts.audio.iter().any(|e| e.eq_ignore_ascii_case(&extension)) {
            out.audio_files.push(item);
        } else {
            out.other_files.push(item);
        }
    }

    Ok(out)
}

pub fn clean_empty_dirs(source: &Path) -> Result<()> {
    let mut dirs = WalkDir::new(source)
        .contents_first(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_dir())
        .map(|e| e.path().to_path_buf())
        .collect::<Vec<_>>();

    dirs.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for dir in dirs {
        let is_empty = std::fs::read_dir(&dir)
            .with_context(|| format!("failed reading {}", dir.display()))?
            .next()
            .is_none();
        if is_empty {
            let _ = std::fs::remove_dir(&dir);
        }
    }
    Ok(())
}

fn lower_ext(path: &Path) -> String {
    path.extension()
        .map(|e| format!(".{}", e.to_string_lossy().to_ascii_lowercase()))
        .unwrap_or_default()
}
