use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

pub mod profile;
pub mod settings;
pub use settings::Settings;

const CONFIG_FILE: &str = "config.json";
const APP_DIR: &str = "audiok";
const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;

fn dir_from_env(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(|base| PathBuf::from(base).join(APP_DIR))
}

fn config_path() -> PathBuf {
    match dir_from_env("LOCALAPPDATA").or_else(|| dir_from_env("APPDATA")) {
        Some(dir) => dir.join(CONFIG_FILE),
        None => PathBuf::from(CONFIG_FILE),
    }
}

pub fn log_path() -> PathBuf {
    config_path().with_file_name("audiok.log")
}

pub fn previous_log_path() -> PathBuf {
    config_path().with_file_name("audiok.previous.log")
}

pub fn background_dir() -> PathBuf {
    config_path()
        .parent()
        .map(|parent| parent.join("backgrounds"))
        .unwrap_or_else(|| PathBuf::from("backgrounds"))
}

pub fn background_path(file: &str) -> Option<PathBuf> {
    let path = Path::new(file);
    if file.is_empty() || path.file_name().and_then(|name| name.to_str()) != Some(file) {
        return None;
    }
    Some(background_dir().join(path))
}

fn legacy_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(roaming) = dir_from_env("APPDATA") {
        paths.push(roaming.join(CONFIG_FILE));
    }
    paths.push(PathBuf::from(CONFIG_FILE));
    paths
}

fn load_from(path: &Path) -> Option<Settings> {
    let file = fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_CONFIG_BYTES {
        return None;
    }
    let mut contents = String::new();
    file.take(MAX_CONFIG_BYTES + 1)
        .read_to_string(&mut contents)
        .ok()?;
    if contents.len() as u64 > MAX_CONFIG_BYTES {
        return None;
    }
    let mut settings: Settings = serde_json::from_str(&contents).ok()?;
    settings.sanitize();
    Some(settings)
}

fn save_to(path: &Path, settings: &Settings) {
    let Ok(json) = serde_json::to_string_pretty(settings) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, &json).is_err() {
        let _ = fs::write(path, json);
        return;
    }
    if fs::rename(&tmp, path).is_err() {
        let _ = fs::remove_file(path);
        if fs::rename(&tmp, path).is_err() {
            let _ = fs::remove_file(&tmp);
            let _ = fs::write(path, json);
        }
    }
}

pub fn load_settings() -> Settings {
    let path = config_path();
    if let Some(settings) = load_from(&path) {
        return settings;
    }
    for legacy in legacy_paths() {
        if legacy != path {
            if let Some(settings) = load_from(&legacy) {
                save_to(&path, &settings);
                return settings;
            }
        }
    }
    Settings::default()
}

pub fn save_settings(settings: &Settings) {
    let mut settings = settings.clone();
    settings.sanitize();
    save_to(&config_path(), &settings);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_path_accepts_only_plain_file_names() {
        assert!(background_path("bg.png").is_some());
        assert!(background_path("../bg.png").is_none());
        assert!(background_path("sub/bg.png").is_none());
        assert!(background_path("").is_none());
    }
}
