//! Platform-aware filesystem layout for OpenRust.

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformKind {
    Windows,
    Fedora,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct PlatformPaths {
    pub kind: PlatformKind,
    pub home: PathBuf,
    pub config: PlatformScope,
    pub data: PlatformScope,
    pub cache: PlatformScope,
}

#[derive(Debug, Clone)]
pub struct PlatformScope {
    pub dir: PathBuf,
}

impl PlatformPaths {
    pub fn detect() -> Self {
        let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
        let kind = detect_kind();
        let (config_dir, data_dir, cache_dir) = match kind {
            PlatformKind::Windows => {
                let appdata = std::env::var_os("APPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join("AppData").join("Roaming"));
                let local = std::env::var_os("LOCALAPPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join("AppData").join("Local"));
                (
                    appdata.join("openrust"),
                    local.join("openrust"),
                    local.join("openrust").join("cache"),
                )
            }
            PlatformKind::Fedora | PlatformKind::Unknown => (
                home.join(".config").join("openrust"),
                home.join(".local").join("share").join("openrust"),
                home.join(".cache").join("openrust"),
            ),
        };

        Self {
            kind,
            home,
            config: PlatformScope { dir: config_dir },
            data: PlatformScope { dir: data_dir },
            cache: PlatformScope { dir: cache_dir },
        }
    }

    pub fn global_config_path(&self) -> PathBuf {
        self.config.dir.join("config.json")
    }

    pub fn credentials_path(&self) -> PathBuf {
        self.config.dir.join("credentials.enc")
    }

    pub fn undo_dir(&self) -> PathBuf {
        self.cache.dir.join("undo")
    }

    pub fn sessions_db_path(&self) -> PathBuf {
        self.data.dir.join("sessions.db")
    }

    pub fn config_dir(&self) -> &PathBuf {
        &self.config.dir
    }

    pub fn data_dir(&self) -> &PathBuf {
        &self.data.dir
    }

    pub fn cache_dir(&self) -> &PathBuf {
        &self.cache.dir
    }
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}

pub fn detect_kind() -> PlatformKind {
    if cfg!(windows) {
        return PlatformKind::Windows;
    }

    let os_release = std::fs::read_to_string("/etc/os-release")
        .unwrap_or_default()
        .to_lowercase();
    if os_release.contains("fedora") {
        return PlatformKind::Fedora;
    }

    PlatformKind::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_paths_include_required_entries() {
        let paths = PlatformPaths::detect();
        assert!(paths.global_config_path().ends_with("config.json"));
        assert!(paths.credentials_path().ends_with("credentials.enc"));
        assert!(paths.config_dir().ends_with("openrust"));
        assert!(paths.data_dir().ends_with("openrust"));
        assert!(paths.cache_dir().ends_with("cache"));
        assert!(paths.undo_dir().ends_with("undo"));
        assert!(paths.sessions_db_path().ends_with("sessions.db"));
    }

    #[test]
    fn home_dir_uses_current_process_environment() {
        assert!(home_dir().is_some());
    }
}
