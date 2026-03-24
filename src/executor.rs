use std::path::Path;

use anyhow::{Context, Result};

use crate::planner::{Operation, OperationKind, Plan};

#[derive(Debug, Default)]
pub struct ExecutionResult {
    pub succeeded: usize,
    pub failed: usize,
    pub skipped: usize,
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
            Err(_) => result.failed += 1,
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
