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

#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub operations: Vec<Operation>,
    pub conflicts: Vec<PathBuf>,
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

        if dest.exists() {
            plan.conflicts.push(dest.clone());
        }

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
        attach_non_media(scan, &mut plan, mode, &matched_dirs);
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
    let mut parent_to_folder = HashMap::<PathBuf, PathBuf>::new();
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

        if dest.exists() {
            plan.conflicts.push(dest.clone());
        }

        if let Some(parent) = src.parent() {
            parent_to_folder.insert(parent.to_path_buf(), target_folder);
        }

        plan.operations.push(Operation {
            source: src,
            destination: dest,
            kind: mode.into(),
        });
    }

    if let NonMediaPolicy::Keep = non_media_mode {
        let fallback = default_target_folder.clone();
        for item in scan
            .subtitle_files
            .iter()
            .chain(scan.audio_files.iter())
            .chain(scan.other_files.iter())
        {
            if let Some(parent) = item.path.parent() {
                if let Some(target) = parent_to_folder.get(parent) {
                    plan.operations.push(Operation {
                        source: item.path.clone(),
                        destination: target.join(&item.file_name),
                        kind: mode.into(),
                    });
                    continue;
                }
            }
            if let Some(target) = &fallback {
                plan.operations.push(Operation {
                    source: item.path.clone(),
                    destination: target.join(&item.file_name),
                    kind: mode.into(),
                });
            }
        }
    }

    Ok(plan)
}

fn attach_non_media(
    scan: &ScanResult,
    plan: &mut Plan,
    mode: EffectiveOperationMode,
    matched_dirs: &HashSet<PathBuf>,
) {
    let mut dir_to_dest = HashMap::<PathBuf, PathBuf>::new();
    for op in &plan.operations {
        if let (Some(src_parent), Some(dst_parent)) = (op.source.parent(), op.destination.parent()) {
            dir_to_dest
                .entry(src_parent.to_path_buf())
                .or_insert_with(|| dst_parent.to_path_buf());
        }
    }

    for item in scan
        .subtitle_files
        .iter()
        .chain(scan.audio_files.iter())
        .chain(scan.other_files.iter())
    {
        if let Some(parent) = item.path.parent() {
            let parent_buf = parent.to_path_buf();
            if matched_dirs.contains(&parent_buf) {
                if let Some(dest_parent) = dir_to_dest.get(&parent_buf) {
                    plan.operations.push(Operation {
                        source: item.path.clone(),
                        destination: dest_parent.join(&item.file_name),
                        kind: mode.into(),
                    });
                }
            }
        }
    }
}
