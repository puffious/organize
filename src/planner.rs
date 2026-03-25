use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::config::EffectiveOperationMode;
use crate::parser::MediaInfo;
use crate::scanner::{NonMediaPolicy, ScanResult};

#[derive(Debug, Clone, Copy)]
pub enum OperationKind {
    Move,
    Copy,
    HardLink,
    SymLink,
}

impl From<EffectiveOperationMode> for OperationKind {
    fn from(value: EffectiveOperationMode) -> Self {
        match value {
            EffectiveOperationMode::Move => Self::Move,
            EffectiveOperationMode::Copy => Self::Copy,
            EffectiveOperationMode::HardLink => Self::HardLink,
            EffectiveOperationMode::SymLink => Self::SymLink,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Operation {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub kind: OperationKind,
}

#[derive(Debug, Clone)]
pub struct UnparseableItem {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    ExistingFile,
    ExistingDirectory,
    ParentPathIsFile,
}

#[derive(Debug, Clone)]
pub struct ConflictItem {
    pub path: PathBuf,
    pub kind: ConflictKind,
    pub blocked_by: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub operations: Vec<Operation>,
    pub conflicts: Vec<PathBuf>,
    pub conflict_details: Vec<ConflictItem>,
    pub unparseable: Vec<UnparseableItem>,
}

pub fn build_show_plan(
    scan: &ScanResult,
    parsed: &[MediaInfo],
    destination_root: &Path,
    forced_title: Option<String>,
    forced_year: Option<u16>,
    mode: EffectiveOperationMode,
    non_media_mode: NonMediaPolicy,
) -> Result<Plan> {
    let mut plan = Plan::default();
    let mut matched_dirs = HashSet::new();

    for info in parsed {
        let Some(src) = info.full_path.clone() else {
            continue;
        };

        let title = forced_title.clone().or_else(|| info.title.clone());
        let year = forced_year.or(info.year);
        let season = info.season;
        let episode = info.episode;

        let (Some(title), Some(season), Some(_episode)) = (title, season, episode) else {
            plan.unparseable.push(UnparseableItem {
                path: src,
                reason: "missing title, season, or episode".to_string(),
            });
            continue;
        };

        let show_folder = if let Some(y) = year {
            format!("{} ({})", title, y)
        } else {
            title
        };
        let season_folder = format!("Season {:02}", season);
        let file_name = info.original_filename.clone();
        let dest = destination_root
            .join(show_folder)
            .join(season_folder)
            .join(file_name);

        record_conflict(&mut plan, &dest);

        if let Some(parent) = src.parent() {
            matched_dirs.insert(parent.to_path_buf());
        }

        plan.operations.push(Operation {
            source: src,
            destination: dest,
            kind: mode.into(),
        });
    }

    if let NonMediaPolicy::Keep = non_media_mode {
        attach_non_media(scan, &mut plan, mode, Some(&matched_dirs), None);
    }

    Ok(plan)
}

pub fn build_movie_plan(
    scan: &ScanResult,
    parsed: &[MediaInfo],
    destination_root: &Path,
    forced_title: Option<String>,
    forced_year: Option<u16>,
    mode: EffectiveOperationMode,
    non_media_mode: NonMediaPolicy,
) -> Result<Plan> {
    let mut plan = Plan::default();
    let mut default_target_folder: Option<PathBuf> = None;

    for info in parsed {
        let Some(src) = info.full_path.clone() else {
            continue;
        };

        let title = forced_title.clone().or_else(|| info.title.clone());
        let year = forced_year.or(info.year);

        let Some(title) = title else {
            plan.unparseable.push(UnparseableItem {
                path: src,
                reason: "missing title".to_string(),
            });
            continue;
        };

        let folder_name = if let Some(y) = year {
            format!("{} ({})", title, y)
        } else {
            title
        };
        let target_folder = destination_root.join(folder_name);
        if default_target_folder.is_none() {
            default_target_folder = Some(target_folder.clone());
        }
        let dest = target_folder.join(&info.original_filename);

        record_conflict(&mut plan, &dest);
        plan.operations.push(Operation {
            source: src,
            destination: dest,
            kind: mode.into(),
        });
    }

    if let NonMediaPolicy::Keep = non_media_mode {
        attach_non_media(scan, &mut plan, mode, None, default_target_folder.as_deref());
    }

    Ok(plan)
}

fn attach_non_media(
    scan: &ScanResult,
    plan: &mut Plan,
    mode: EffectiveOperationMode,
    matched_dirs: Option<&HashSet<PathBuf>>,
    fallback: Option<&Path>,
) {
    let (dir_to_dest, video_key_to_dest) = build_destination_maps(plan);

    for item in scan.subtitle_files.iter().chain(scan.audio_files.iter()) {
        if let Some(dest_parent) = resolve_dest_parent(
            &item.path,
            matched_dirs,
            &dir_to_dest,
            &video_key_to_dest,
            fallback,
            true,
        ) {
            plan.operations.push(Operation {
                source: item.path.clone(),
                destination: dest_parent.join(&item.file_name),
                kind: mode.into(),
            });
        }
    }

    for item in &scan.other_files {
        if let Some(dest_parent) = resolve_dest_parent(
            &item.path,
            matched_dirs,
            &dir_to_dest,
            &video_key_to_dest,
            fallback,
            false,
        ) {
            plan.operations.push(Operation {
                source: item.path.clone(),
                destination: dest_parent.join(&item.file_name),
                kind: mode.into(),
            });
        }
    }
}

fn build_destination_maps(
    plan: &Plan,
) -> (
    HashMap<PathBuf, PathBuf>,
    HashMap<(PathBuf, String), PathBuf>,
) {
    let mut dir_to_dest = HashMap::<PathBuf, PathBuf>::new();
    let mut video_key_to_dest = HashMap::<(PathBuf, String), PathBuf>::new();

    for op in &plan.operations {
        if let (Some(src_parent), Some(dst_parent)) = (op.source.parent(), op.destination.parent()) {
            dir_to_dest
                .entry(src_parent.to_path_buf())
                .or_insert_with(|| dst_parent.to_path_buf());

            if let Some(stem) = lower_stem(&op.source) {
                video_key_to_dest.insert((src_parent.to_path_buf(), stem), dst_parent.to_path_buf());
            }
        }
    }

    (dir_to_dest, video_key_to_dest)
}

fn resolve_dest_parent<'a>(
    item_path: &Path,
    matched_dirs: Option<&HashSet<PathBuf>>,
    dir_to_dest: &'a HashMap<PathBuf, PathBuf>,
    video_key_to_dest: &'a HashMap<(PathBuf, String), PathBuf>,
    fallback: Option<&'a Path>,
    match_by_stem: bool,
) -> Option<&'a Path> {
    let parent = item_path.parent()?;
    let parent_buf = parent.to_path_buf();

    let is_allowed = match matched_dirs {
        Some(dirs) => dirs.contains(&parent_buf),
        None => true,
    };

    if is_allowed {
        if match_by_stem {
            if let Some(stem) = lower_stem(item_path) {
                if let Some(dest_parent) = video_key_to_dest.get(&(parent_buf.clone(), stem)) {
                    return Some(dest_parent.as_path());
                }
            }
        }

        if let Some(dest_parent) = dir_to_dest.get(&parent_buf) {
            return Some(dest_parent.as_path());
        }
    }

    fallback
}
fn lower_stem(path: &Path) -> Option<String> {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
}

fn record_conflict(plan: &mut Plan, destination: &Path) {
    if let Some(blocker) = parent_path_file_blocker(destination) {
        plan.conflicts.push(destination.to_path_buf());
        plan.conflict_details.push(ConflictItem {
            path: destination.to_path_buf(),
            kind: ConflictKind::ParentPathIsFile,
            blocked_by: Some(blocker),
        });
        return;
    }

    if !destination.exists() {
        return;
    }

    let kind = if destination.is_dir() {
        ConflictKind::ExistingDirectory
    } else {
        ConflictKind::ExistingFile
    };

    plan.conflicts.push(destination.to_path_buf());
    plan.conflict_details.push(ConflictItem {
        path: destination.to_path_buf(),
        kind,
        blocked_by: None,
    });
}

fn parent_path_file_blocker(destination: &Path) -> Option<PathBuf> {
    let mut current = destination.parent();
    while let Some(path) = current {
        if path.exists() {
            if path.is_file() {
                return Some(path.to_path_buf());
            }
            return None;
        }
        current = path.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::EffectiveOperationMode;
    use crate::parser::MediaInfo;
    use crate::scanner::{ScanResult, ScannedFile};
    use tempfile::tempdir;

    fn scanned(path: PathBuf, file_name: &str, parent_name: &str, extension: &str) -> ScannedFile {
        ScannedFile {
            path,
            file_name: file_name.to_string(),
            parent_name: parent_name.to_string(),
            extension: extension.to_string(),
        }
    }

    fn parsed(
        path: PathBuf,
        file_name: &str,
        title: Option<&str>,
        year: Option<u16>,
        season: Option<u16>,
        episode: Option<u16>,
    ) -> MediaInfo {
        MediaInfo {
            title: title.map(str::to_string),
            year,
            season,
            episode,
            extension: ".mkv".to_string(),
            original_filename: file_name.to_string(),
            full_path: Some(path),
        }
    }

    #[test]
    fn show_plan_routes_to_season_and_attaches_non_media() {
        let source_parent = PathBuf::from("/tmp/source/Show.S01");
        let video_path = source_parent.join("Show.S01E01.mkv");
        let subtitle_path = source_parent.join("Show.S01E01.srt");
        let other_path = source_parent.join("poster.jpg");

        let scan = ScanResult {
            video_files: vec![scanned(
                video_path.clone(),
                "Show.S01E01.mkv",
                "Show.S01",
                ".mkv",
            )],
            subtitle_files: vec![scanned(
                subtitle_path.clone(),
                "Show.S01E01.srt",
                "Show.S01",
                ".srt",
            )],
            audio_files: vec![],
            other_files: vec![scanned(
                other_path.clone(),
                "poster.jpg",
                "Show.S01",
                ".jpg",
            )],
        };

        let parsed = vec![parsed(
            video_path,
            "Show.S01E01.mkv",
            Some("Show"),
            Some(2022),
            Some(1),
            Some(1),
        )];
        let dest = PathBuf::from("/tmp/dest");

        let plan = build_show_plan(
            &scan,
            &parsed,
            &dest,
            None,
            None,
            EffectiveOperationMode::Move,
            NonMediaPolicy::Keep,
        )
        .expect("plan should build");

        assert_eq!(plan.operations.len(), 3);
        assert!(plan.unparseable.is_empty());
        assert!(plan.operations.iter().any(|op| op
            .destination
            .ends_with("Show (2022)/Season 01/Show.S01E01.mkv")));
        assert!(plan.operations.iter().any(|op| op.source == subtitle_path));
        assert!(plan.operations.iter().any(|op| op.source == other_path));
    }

    #[test]
    fn show_plan_marks_missing_episode_as_unparseable() {
        let source_parent = PathBuf::from("/tmp/source/Show.S01");
        let video_path = source_parent.join("Show.S01.only.mkv");
        let scan = ScanResult {
            video_files: vec![scanned(
                video_path.clone(),
                "Show.S01.only.mkv",
                "Show.S01",
                ".mkv",
            )],
            subtitle_files: vec![],
            audio_files: vec![],
            other_files: vec![],
        };

        let parsed = vec![parsed(
            video_path,
            "Show.S01.only.mkv",
            Some("Show"),
            Some(2022),
            Some(1),
            None,
        )];
        let plan = build_show_plan(
            &scan,
            &parsed,
            Path::new("/tmp/dest"),
            None,
            None,
            EffectiveOperationMode::Move,
            NonMediaPolicy::Keep,
        )
        .expect("plan should build");

        assert_eq!(plan.operations.len(), 0);
        assert_eq!(plan.unparseable.len(), 1);
    }

    #[test]
    fn movie_plan_uses_fallback_folder_for_unmatched_non_media() {
        let video_parent = PathBuf::from("/tmp/source/movie");
        let video_path = video_parent.join("Movie.2023.mkv");
        let subtitle_path = video_parent.join("Movie.2023.srt");
        let orphan_other = PathBuf::from("/tmp/source/extras/readme.txt");

        let scan = ScanResult {
            video_files: vec![scanned(
                video_path.clone(),
                "Movie.2023.mkv",
                "movie",
                ".mkv",
            )],
            subtitle_files: vec![scanned(subtitle_path, "Movie.2023.srt", "movie", ".srt")],
            audio_files: vec![],
            other_files: vec![scanned(
                orphan_other.clone(),
                "readme.txt",
                "extras",
                ".txt",
            )],
        };

        let parsed = vec![parsed(
            video_path,
            "Movie.2023.mkv",
            Some("Movie"),
            Some(2023),
            None,
            None,
        )];
        let dest_root = PathBuf::from("/tmp/dest");

        let plan = build_movie_plan(
            &scan,
            &parsed,
            &dest_root,
            None,
            None,
            EffectiveOperationMode::Copy,
            NonMediaPolicy::Keep,
        )
        .expect("plan should build");

        assert_eq!(plan.operations.len(), 3);
        let fallback_dest = dest_root.join("Movie (2023)").join("readme.txt");
        assert!(plan
            .operations
            .iter()
            .any(|op| op.destination == fallback_dest && op.source == orphan_other));
    }

    #[test]
    fn show_plan_pairs_subtitle_to_matching_season_by_stem() {
        let source_parent = PathBuf::from("/tmp/source/Show.Complete");
        let s1_video = source_parent.join("Show.S01E01.mkv");
        let s2_video = source_parent.join("Show.S02E01.mkv");
        let s2_sub = source_parent.join("Show.S02E01.srt");

        let scan = ScanResult {
            video_files: vec![
                scanned(s1_video.clone(), "Show.S01E01.mkv", "Show.Complete", ".mkv"),
                scanned(s2_video.clone(), "Show.S02E01.mkv", "Show.Complete", ".mkv"),
            ],
            subtitle_files: vec![scanned(
                s2_sub.clone(),
                "Show.S02E01.srt",
                "Show.Complete",
                ".srt",
            )],
            audio_files: vec![],
            other_files: vec![],
        };

        let parsed = vec![
            parsed(
                s1_video,
                "Show.S01E01.mkv",
                Some("Show"),
                Some(2022),
                Some(1),
                Some(1),
            ),
            parsed(
                s2_video,
                "Show.S02E01.mkv",
                Some("Show"),
                Some(2022),
                Some(2),
                Some(1),
            ),
        ];

        let plan = build_show_plan(
            &scan,
            &parsed,
            Path::new("/tmp/dest"),
            None,
            None,
            EffectiveOperationMode::Move,
            NonMediaPolicy::Keep,
        )
        .expect("plan should build");

        assert!(plan.operations.iter().any(|op| op.source == s2_sub
            && op
                .destination
                .ends_with("Show (2022)/Season 02/Show.S02E01.srt")));
    }

    #[test]
    fn movie_plan_pairs_subtitle_with_matching_movie_by_stem() {
        let source_parent = PathBuf::from("/tmp/source/mixed");
        let m1_video = source_parent.join("Movie.One.2021.mkv");
        let m2_video = source_parent.join("Movie.Two.2022.mkv");
        let m2_sub = source_parent.join("Movie.Two.2022.srt");

        let scan = ScanResult {
            video_files: vec![
                scanned(m1_video.clone(), "Movie.One.2021.mkv", "mixed", ".mkv"),
                scanned(m2_video.clone(), "Movie.Two.2022.mkv", "mixed", ".mkv"),
            ],
            subtitle_files: vec![scanned(
                m2_sub.clone(),
                "Movie.Two.2022.srt",
                "mixed",
                ".srt",
            )],
            audio_files: vec![],
            other_files: vec![],
        };

        let parsed = vec![
            parsed(
                m1_video,
                "Movie.One.2021.mkv",
                Some("Movie One"),
                Some(2021),
                None,
                None,
            ),
            parsed(
                m2_video,
                "Movie.Two.2022.mkv",
                Some("Movie Two"),
                Some(2022),
                None,
                None,
            ),
        ];

        let plan = build_movie_plan(
            &scan,
            &parsed,
            Path::new("/tmp/dest"),
            None,
            None,
            EffectiveOperationMode::Move,
            NonMediaPolicy::Keep,
        )
        .expect("plan should build");

        assert!(plan.operations.iter().any(|op| op.source == m2_sub
            && op
                .destination
                .ends_with("Movie Two (2022)/Movie.Two.2022.srt")));
    }

    #[test]
    fn show_plan_records_existing_file_conflict_kind() {
        let dir = tempdir().expect("create tempdir");
        let source_parent = dir.path().join("source");
        std::fs::create_dir_all(&source_parent).expect("create source dir");
        let video_path = source_parent.join("Show.S01E01.mkv");
        std::fs::write(&video_path, b"video").expect("write source video");

        let dest_root = dir.path().join("dest");
        let conflict_path = dest_root
            .join("Show (2022)")
            .join("Season 01")
            .join("Show.S01E01.mkv");
        std::fs::create_dir_all(conflict_path.parent().expect("conflict parent"))
            .expect("create conflict parent");
        std::fs::write(&conflict_path, b"existing").expect("write conflict file");

        let scan = ScanResult {
            video_files: vec![scanned(
                video_path.clone(),
                "Show.S01E01.mkv",
                "source",
                ".mkv",
            )],
            subtitle_files: vec![],
            audio_files: vec![],
            other_files: vec![],
        };

        let parsed = vec![parsed(
            video_path,
            "Show.S01E01.mkv",
            Some("Show"),
            Some(2022),
            Some(1),
            Some(1),
        )];

        let plan = build_show_plan(
            &scan,
            &parsed,
            &dest_root,
            None,
            None,
            EffectiveOperationMode::Move,
            NonMediaPolicy::Keep,
        )
        .expect("plan should build");

        assert_eq!(plan.conflicts.len(), 1);
        assert_eq!(plan.conflict_details.len(), 1);
        assert_eq!(plan.conflict_details[0].kind, ConflictKind::ExistingFile);
    }

    #[test]
    fn show_plan_records_parent_path_file_conflict_kind() {
        let dir = tempdir().expect("create tempdir");
        let source_parent = dir.path().join("source");
        std::fs::create_dir_all(&source_parent).expect("create source dir");
        let video_path = source_parent.join("Show.S01E01.mkv");
        std::fs::write(&video_path, b"video").expect("write source video");

        let dest_root = dir.path().join("dest");
        std::fs::create_dir_all(&dest_root).expect("create dest root");
        let blocked_parent = dest_root.join("Show (2022)");
        std::fs::write(&blocked_parent, b"not a directory").expect("write blocker file");

        let scan = ScanResult {
            video_files: vec![scanned(
                video_path.clone(),
                "Show.S01E01.mkv",
                "source",
                ".mkv",
            )],
            subtitle_files: vec![],
            audio_files: vec![],
            other_files: vec![],
        };

        let parsed = vec![parsed(
            video_path,
            "Show.S01E01.mkv",
            Some("Show"),
            Some(2022),
            Some(1),
            Some(1),
        )];

        let plan = build_show_plan(
            &scan,
            &parsed,
            &dest_root,
            None,
            None,
            EffectiveOperationMode::Move,
            NonMediaPolicy::Keep,
        )
        .expect("plan should build");

        assert_eq!(plan.conflict_details.len(), 1);
        assert_eq!(
            plan.conflict_details[0].kind,
            ConflictKind::ParentPathIsFile
        );
        assert_eq!(
            plan.conflict_details[0].blocked_by.as_deref(),
            Some(blocked_parent.as_path())
        );
    }
}
