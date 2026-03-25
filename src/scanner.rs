use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::warn;
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

    for entry in WalkDir::new(source).follow_links(false) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                warn!("skipping unreadable path under {}: {}", source.display(), err);
                continue;
            }
        };

        if !entry.file_type().is_file() {
            continue;
        }

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

        if exts
            .video
            .iter()
            .any(|e| e.eq_ignore_ascii_case(&extension))
        {
            out.video_files.push(item);
        } else if exts
            .subtitle
            .iter()
            .any(|e| e.eq_ignore_ascii_case(&extension))
        {
            out.subtitle_files.push(item);
        } else if exts
            .audio
            .iter()
            .any(|e| e.eq_ignore_ascii_case(&extension))
        {
            out.audio_files.push(item);
        } else {
            out.other_files.push(item);
        }
    }

    Ok(out)
}

pub fn clean_empty_dirs(source: &Path) -> Result<()> {
    let mut dirs = Vec::new();
    for entry in WalkDir::new(source).contents_first(true) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                warn!(
                    "skipping unreadable path while cleaning under {}: {}",
                    source.display(),
                    err
                );
                continue;
            }
        };

        if entry.file_type().is_dir() {
            dirs.push(entry.path().to_path_buf());
        }
    }

    dirs.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for dir in dirs {
        let is_empty = std::fs::read_dir(&dir)
            .with_context(|| format!("failed reading {}", dir.display()))?
            .next()
            .is_none();
        if is_empty {
            if let Err(err) = std::fs::remove_dir(&dir) {
                warn!("failed to remove empty directory {}: {}", dir.display(), err);
            }
        }
    }
    Ok(())
}

fn lower_ext(path: &Path) -> String {
    path.extension()
        .map(|e| format!(".{}", e.to_string_lossy().to_ascii_lowercase()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MediaExtensions;
    use std::fs;
    use tempfile::tempdir;

    fn write_file(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        fs::write(path, b"test").expect("write test file");
    }

    #[test]
    fn scan_source_classifies_files_by_extension() {
        let dir = tempdir().expect("create tempdir");
        let root = dir.path();

        write_file(&root.join("show/episode01.MKV"));
        write_file(&root.join("show/episode01.srt"));
        write_file(&root.join("show/episode01.FLAC"));
        write_file(&root.join("show/poster.jpg"));

        let exts = MediaExtensions::default();
        let result = scan_source(root, &exts).expect("scan source");

        assert_eq!(result.video_files.len(), 1);
        assert_eq!(result.subtitle_files.len(), 1);
        assert_eq!(result.audio_files.len(), 1);
        assert_eq!(result.other_files.len(), 1);
    }

    #[test]
    fn clean_empty_dirs_removes_empty_only() {
        let dir = tempdir().expect("create tempdir");
        let root = dir.path();
        let keep_dir = root.join("keep");
        let remove_dir = root.join("remove/nested");

        fs::create_dir_all(&keep_dir).expect("create keep dir");
        fs::create_dir_all(&remove_dir).expect("create remove dir");
        fs::write(keep_dir.join("data.txt"), b"x").expect("write keep file");

        clean_empty_dirs(root).expect("clean empty dirs");

        assert!(keep_dir.exists());
        assert!(!remove_dir.exists());
    }
}
