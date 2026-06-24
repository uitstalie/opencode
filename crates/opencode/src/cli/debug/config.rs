use clap::Subcommand;

#[derive(Subcommand)]
pub enum Cmd {
    /// Validate opencode.json configuration
    Validate,
    /// Show parsed configuration
    Show,
    /// Show config file paths
    Path,
}

pub fn run(cmd: Cmd) -> anyhow::Result<()> {
    match cmd {
        Cmd::Validate => {
            println!("Config validation: (not yet implemented)");
            println!("  Looking for opencode.json in current directory...");
            Ok(())
        }
        Cmd::Show => {
            println!("Config: (not yet implemented)");
            Ok(())
        }
        Cmd::Path => {
            let cwd = std::env::current_dir()?;
            println!("Project config:  {}/opencode.json", cwd.display());
            println!("                  {}/.opencode/opencode.jsonc", cwd.display());
            let home = dirs_fallback();
            println!("Global config:   {}/.config/opencode/opencode.json", home);
            Ok(())
        }
    }
}

fn dirs_fallback() -> String {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| "~".to_string())
}
