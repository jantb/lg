//! `terrarium clear` — removing everything terrarium configured for a project.

use super::prompts::confirm;

pub(super) fn run(yes: bool) -> anyhow::Result<()> {
    let project_root = std::env::current_dir()?;
    let confirmed = yes
        || confirm(
            &format!(
                "Remove all terrarium settings for {}?",
                project_root.display()
            ),
            false,
        );
    if !confirmed {
        println!("terrarium: nothing cleared");
        return Ok(());
    }

    let removed = crate::app::clear_project(&project_root)?;
    if removed.is_empty() {
        println!("terrarium: nothing to clear for this project");
    } else {
        for item in &removed {
            println!("terrarium: removed {item}");
        }
    }
    Ok(())
}
