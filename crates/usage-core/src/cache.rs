use crate::usage::UsageSnapshot;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Konnte App-Datenverzeichnis nicht bestimmen")]
    NoAppDir,
    #[error("Dateizugriff fehlgeschlagen: {0}")]
    Io(#[from] std::io::Error),
    #[error("Ungueltiges JSON: {0}")]
    Parse(#[from] serde_json::Error),
}

/// Verzeichnis fuer Settings/Cache-Dateien der App (z.B. Windows:
/// %APPDATA%\claude-usage-widget, Linux: ~/.config/claude-usage-widget).
pub fn app_data_dir() -> Result<PathBuf, StoreError> {
    let base = dirs::config_dir().ok_or(StoreError::NoAppDir)?;
    Ok(base.join("claude-usage-widget"))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub theme: Theme,
    pub accent_color: String,
    pub compact_layout: bool,
    pub always_on_top: bool,
    pub opacity: f32,
    pub poll_interval_secs: u64,
    pub window_x: Option<i32>,
    pub window_y: Option<i32>,
    pub autostart: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    System,
    Dark,
    Light,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            accent_color: "#d97757".to_string(),
            compact_layout: false,
            always_on_top: true,
            opacity: 0.96,
            // Jeder Poll kostet minimal Kontingent (siehe usage.rs) - 5 Minuten
            // ist ein vernuenftiger Default zwischen Aktualitaet und Verbrauch.
            poll_interval_secs: 300,
            window_x: None,
            window_y: None,
            autostart: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct UsageCache {
    pub last_snapshot: Option<UsageSnapshot>,
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> Result<T, StoreError> {
    if !path.exists() {
        return Ok(T::default());
    }
    let raw = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), StoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string_pretty(value)?;
    std::fs::write(path, raw)?;
    Ok(())
}

pub fn load_settings(dir: &Path) -> Result<Settings, StoreError> {
    read_json(&dir.join("settings.json"))
}

pub fn save_settings(dir: &Path, settings: &Settings) -> Result<(), StoreError> {
    write_json(&dir.join("settings.json"), settings)
}

pub fn load_usage_cache(dir: &Path) -> Result<UsageCache, StoreError> {
    read_json(&dir.join("usage_cache.json"))
}

pub fn save_usage_cache(dir: &Path, cache: &UsageCache) -> Result<(), StoreError> {
    write_json(&dir.join("usage_cache.json"), cache)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::RateWindow;

    fn tmp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("uc-cache-test-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn defaults_when_no_settings_file_exists() {
        let dir = tmp_dir("defaults");
        let settings = load_settings(&dir).unwrap();
        assert_eq!(settings, Settings::default());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn round_trips_settings() {
        let dir = tmp_dir("roundtrip");
        let mut settings = Settings::default();
        settings.theme = Theme::Dark;
        settings.poll_interval_secs = 600;
        settings.window_x = Some(120);

        save_settings(&dir, &settings).unwrap();
        let loaded = load_settings(&dir).unwrap();
        assert_eq!(loaded, settings);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn round_trips_usage_cache() {
        let dir = tmp_dir("usagecache");
        let cache = UsageCache {
            last_snapshot: Some(UsageSnapshot {
                five_hour: Some(RateWindow {
                    status: Some("allowed".into()),
                    reset_unix: Some(123),
                    utilization: Some(0.42),
                }),
                seven_day: None,
                overage: None,
                representative_claim: Some("five_hour".into()),
                fetched_at_unix: 100,
            }),
        };
        save_usage_cache(&dir, &cache).unwrap();
        let loaded = load_usage_cache(&dir).unwrap();
        assert_eq!(loaded, cache);
        std::fs::remove_dir_all(&dir).ok();
    }
}
