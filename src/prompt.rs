use anyhow::Result;
use dialoguer::{Confirm, Input};

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

pub fn ask_for_season() -> Result<Option<u16>> {
    let season: String = Input::new()
        .allow_empty(true)
        .with_prompt("Season number")
        .interact_text()?;
    if season.trim().is_empty() {
        return Ok(None);
    }
    Ok(season.trim().parse::<u16>().ok())
}

pub fn ask_for_episode() -> Result<Option<u16>> {
    let episode: String = Input::new()
        .allow_empty(true)
        .with_prompt("Episode number")
        .interact_text()?;
    if episode.trim().is_empty() {
        return Ok(None);
    }
    Ok(episode.trim().parse::<u16>().ok())
}

