use anyhow::Result;
use dialoguer::{Confirm, Input};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShowGroupPrompt {
    pub title: Option<String>,
    pub parent_path: String,
    pub file_count: usize,
    pub missing_season: bool,
    pub missing_episode: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShowGroupResolution {
    pub season: Option<u16>,
    pub episode: Option<u16>,
}

pub fn confirm_execute() -> Result<bool> {
    let answer = Confirm::new()
        .with_prompt("Proceed with these operations?")
        .default(true)
        .interact()?;
    Ok(answer)
}

pub fn ask_for_year(title: Option<&str>) -> Result<Option<u16>> {
    let prompt = match title {
        Some(t) if !t.trim().is_empty() => {
            format!(
                "Year not found for \"{}\". Enter year (or leave blank to skip)",
                t
            )
        }
        _ => "Year not found. Enter year (or leave blank to skip)".to_string(),
    };

    let text: String = Input::new()
        .allow_empty(true)
        .with_prompt(prompt)
        .interact_text()?;
    if text.trim().is_empty() {
        return Ok(None);
    }
    Ok(text.trim().parse::<u16>().ok())
}

pub fn ask_for_show_group_metadata(
    context: &ShowGroupPrompt,
) -> Result<Option<ShowGroupResolution>> {
    let target = context
        .title
        .as_deref()
        .filter(|title| !title.trim().is_empty())
        .map(|title| format!("\"{}\"", title))
        .unwrap_or_else(|| "this group".to_string());
    let file_label = if context.file_count == 1 {
        "file"
    } else {
        "files"
    };
    println!(
        "Missing show metadata for {} in {} ({} {}). Leave blank to skip this group.",
        target,
        display_parent_path(&context.parent_path).display(),
        context.file_count,
        file_label
    );

    let mut resolution = ShowGroupResolution::default();

    if context.missing_season {
        let prompt = if context.missing_episode {
            "Season number"
        } else {
            "Season number (leave blank to skip this group)"
        };
        let Some(season) = ask_for_number(prompt)? else {
            return Ok(None);
        };
        resolution.season = Some(season);
    }

    if context.missing_episode {
        let prompt = if context.missing_season {
            "Episode number"
        } else {
            "Episode number (leave blank to skip this group)"
        };
        let Some(episode) = ask_for_number(prompt)? else {
            return Ok(None);
        };
        resolution.episode = Some(episode);
    }

    Ok(Some(resolution))
}

fn ask_for_number(prompt: &str) -> Result<Option<u16>> {
    let value: String = Input::new()
        .allow_empty(true)
        .with_prompt(prompt)
        .interact_text()?;
    if value.trim().is_empty() {
        return Ok(None);
    }
    Ok(value.trim().parse::<u16>().ok())
}

fn display_parent_path(path: &str) -> &Path {
    Path::new(path)
}
