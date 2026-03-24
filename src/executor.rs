use std::path::Path;

use anyhow::{Context, Result};
use tracing::warn;

use crate::planner::{Operation, OperationKind, Plan};

#[derive(Debug, Default)]
pub struct ExecutionResult {
    pub succeeded: usize,
    pub failed: usize,
    pub skipped: usize,
    pub failures: Vec<String>,
}

impl ExecutionResult {
    pub fn into_exit_result(self) -> Result<()> {
        if self.succeeded == 0 && self.failed > 0 {
            anyhow::bail!("complete failure: all operations failed");
        }
        if self.failed > 0 {
            anyhow::bail!("partial failure: {} operations failed", self.failed);
        }
        Ok(())
    }
}

pub fn execute_plan(plan: &Plan, overwrite: bool) -> Result<ExecutionResult> {
    let mut result = ExecutionResult::default();

    for op in &plan.operations {
        if op.destination.exists() && !overwrite {
            result.skipped += 1;
            continue;
        }

        if let Some(parent) = op.destination.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create destination dir {}", parent.display()))?;
        }

        if overwrite && op.destination.exists() {
            remove_existing(&op.destination)?;
        }

        match execute_operation(op) {
            Ok(()) => result.succeeded += 1,
            Err(err) => {
                result.failed += 1;
                let detail = format!(
                    "{} -> {}: {}",
                    op.source.display(),
                    op.destination.display(),
                    err
                );
                warn!("operation failed: {}", detail);
                result.failures.push(detail);
            }
        }
    }

    Ok(result)
}

fn execute_operation(op: &Operation) -> Result<()> {
    match op.kind {
        OperationKind::Move => execute_move(op),
        OperationKind::Copy => {
            std::fs::copy(&op.source, &op.destination)
                .with_context(|| format!("copy {} -> {} failed", op.source.display(), op.destination.display()))?;
            Ok(())
        }
        OperationKind::HardLink => {
            std::fs::hard_link(&op.source, &op.destination).with_context(|| {
                format!("hardlink {} -> {} failed", op.source.display(), op.destination.display())
            })?;
            Ok(())
        }
        OperationKind::SymLink => {
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(&op.source, &op.destination).with_context(|| {
                    format!("symlink {} -> {} failed", op.source.display(), op.destination.display())
                })?;
            }
            Ok(())
        }
    }
}

fn execute_move(op: &Operation) -> Result<()> {
    match std::fs::rename(&op.source, &op.destination) {
        Ok(_) => Ok(()),
        Err(_) => {
            std::fs::copy(&op.source, &op.destination).with_context(|| {
                format!("cross-device fallback copy {} -> {} failed", op.source.display(), op.destination.display())
            })?;
            std::fs::remove_file(&op.source)
                .with_context(|| format!("failed to remove original file {}", op.source.display()))?;
            Ok(())
        }
    }
}

fn remove_existing(path: &Path) -> Result<()> {
    if path.is_file() {
        std::fs::remove_file(path)
            .with_context(|| format!("failed removing existing destination {}", path.display()))?;
    } else if path.is_dir() {
        std::fs::remove_dir_all(path)
            .with_context(|| format!("failed removing existing destination dir {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::{Operation, OperationKind, Plan};
    use std::fs;
    use tempfile::tempdir;

    fn write_file(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, body).expect("write test file");
    }

    #[test]
    fn copy_operation_copies_file() {
        let dir = tempdir().expect("create tempdir");
        let src = dir.path().join("src/a.txt");
        let dst = dir.path().join("dst/a.txt");
        write_file(&src, "alpha");

        let plan = Plan {
            operations: vec![Operation {
                source: src.clone(),
                destination: dst.clone(),
                kind: OperationKind::Copy,
            }],
            conflicts: vec![],
            conflict_details: vec![],
            unparseable: vec![],
        };

        let result = execute_plan(&plan, false).expect("execute plan");
        assert_eq!(result.succeeded, 1);
        assert!(result.failures.is_empty());
        assert_eq!(fs::read_to_string(dst).expect("read destination"), "alpha");
        assert_eq!(fs::read_to_string(src).expect("read source"), "alpha");
    }

    #[test]
    fn move_operation_moves_file() {
        let dir = tempdir().expect("create tempdir");
        let src = dir.path().join("src/b.txt");
        let dst = dir.path().join("dst/b.txt");
        write_file(&src, "beta");

        let plan = Plan {
            operations: vec![Operation {
                source: src.clone(),
                destination: dst.clone(),
                kind: OperationKind::Move,
            }],
            conflicts: vec![],
            conflict_details: vec![],
            unparseable: vec![],
        };

        let result = execute_plan(&plan, false).expect("execute plan");
        assert_eq!(result.succeeded, 1);
        assert!(result.failures.is_empty());
        assert!(!src.exists());
        assert_eq!(fs::read_to_string(dst).expect("read destination"), "beta");
    }

    #[test]
    fn existing_destination_is_skipped_when_overwrite_disabled() {
        let dir = tempdir().expect("create tempdir");
        let src = dir.path().join("src/c.txt");
        let dst = dir.path().join("dst/c.txt");
        write_file(&src, "new");
        write_file(&dst, "old");

        let plan = Plan {
            operations: vec![Operation {
                source: src.clone(),
                destination: dst.clone(),
                kind: OperationKind::Copy,
            }],
            conflicts: vec![],
            conflict_details: vec![],
            unparseable: vec![],
        };

        let result = execute_plan(&plan, false).expect("execute plan");
        assert_eq!(result.skipped, 1);
        assert_eq!(result.succeeded, 0);
        assert!(result.failures.is_empty());
        assert_eq!(fs::read_to_string(dst).expect("read destination"), "old");
        assert_eq!(fs::read_to_string(src).expect("read source"), "new");
    }

    #[test]
    fn overwrite_replaces_existing_destination() {
        let dir = tempdir().expect("create tempdir");
        let src = dir.path().join("src/d.txt");
        let dst = dir.path().join("dst/d.txt");
        write_file(&src, "fresh");
        write_file(&dst, "stale");

        let plan = Plan {
            operations: vec![Operation {
                source: src,
                destination: dst.clone(),
                kind: OperationKind::Copy,
            }],
            conflicts: vec![],
            conflict_details: vec![],
            unparseable: vec![],
        };

        let result = execute_plan(&plan, true).expect("execute plan");
        assert_eq!(result.succeeded, 1);
        assert!(result.failures.is_empty());
        assert_eq!(fs::read_to_string(dst).expect("read destination"), "fresh");
    }

    #[test]
    fn hardlink_operation_creates_linked_file() {
        let dir = tempdir().expect("create tempdir");
        let src = dir.path().join("src/e.txt");
        let dst = dir.path().join("dst/e.txt");
        write_file(&src, "echo");

        let plan = Plan {
            operations: vec![Operation {
                source: src,
                destination: dst.clone(),
                kind: OperationKind::HardLink,
            }],
            conflicts: vec![],
            conflict_details: vec![],
            unparseable: vec![],
        };

        let result = execute_plan(&plan, false).expect("execute plan");
        assert_eq!(result.succeeded, 1);
        assert!(result.failures.is_empty());
        assert_eq!(fs::read_to_string(dst).expect("read destination"), "echo");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_operation_creates_symlinked_file() {
        let dir = tempdir().expect("create tempdir");
        let src = dir.path().join("src/f.txt");
        let dst = dir.path().join("dst/f.txt");
        write_file(&src, "foxtrot");

        let plan = Plan {
            operations: vec![Operation {
                source: src.clone(),
                destination: dst.clone(),
                kind: OperationKind::SymLink,
            }],
            conflicts: vec![],
            conflict_details: vec![],
            unparseable: vec![],
        };

        let result = execute_plan(&plan, false).expect("execute plan");
        assert_eq!(result.succeeded, 1);
        assert!(result.failures.is_empty());
        let meta = fs::symlink_metadata(&dst).expect("symlink metadata");
        assert!(meta.file_type().is_symlink());
        assert_eq!(fs::read_to_string(dst).expect("read destination"), "foxtrot");
    }

    #[test]
    fn failed_operation_collects_failure_detail() {
        let dir = tempdir().expect("create tempdir");
        let missing_src = dir.path().join("src/missing.txt");
        let dst = dir.path().join("dst/missing.txt");

        let plan = Plan {
            operations: vec![Operation {
                source: missing_src,
                destination: dst,
                kind: OperationKind::Copy,
            }],
            conflicts: vec![],
            conflict_details: vec![],
            unparseable: vec![],
        };

        let result = execute_plan(&plan, false).expect("execute plan should return aggregated result");
        assert_eq!(result.failed, 1);
        assert_eq!(result.succeeded, 0);
        assert_eq!(result.failures.len(), 1);
        assert!(result.failures[0].contains("failed"));
    }
}
