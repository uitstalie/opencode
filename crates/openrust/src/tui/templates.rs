//! Prompt templates for slash commands.

/// `/init` — project onboarding trigger.
///
/// Sends a short message asking the model to load the `project-onboarding`
/// skill and follow its instructions. User focus (if provided) is included.
pub(super) fn init_template(args: &str) -> String {
    let focus = if args.is_empty() {
        String::new()
    } else {
        format!("\n\nUser focus: {args}")
    };

    format!(
        "Initialize this project now. Load and follow the `project-onboarding` skill.{focus}"
    )
}
