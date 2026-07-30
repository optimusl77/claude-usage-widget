use crate::usage::{RateWindow, UsageSnapshot};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("could not determine the app data directory")]
    NoAppDir,
    #[error("file access failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid JSON: {0}")]
    Parse(#[from] serde_json::Error),
}

/// Directory for the app's settings/cache files (e.g. Windows:
/// %APPDATA%\claude-usage-widget, Linux: ~/.config/claude-usage-widget).
pub fn app_data_dir() -> Result<PathBuf, StoreError> {
    let base = dirs::config_dir().ok_or(StoreError::NoAppDir)?;
    Ok(base.join("claude-usage-widget"))
}

fn default_widget_scale() -> f32 {
    1.0
}

fn default_true() -> bool {
    true
}

/// Utilization thresholds (0.0-1.0) at which a bar's automatic severity
/// color switches to the next step. Only used when `Settings::bar_color` is
/// `None` (automatic coloring).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SeverityThresholds {
    pub warning: f32,
    pub serious: f32,
    pub critical: f32,
}

impl Default for SeverityThresholds {
    fn default() -> Self {
        Self { warning: 0.6, serious: 0.8, critical: 0.95 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub theme: Theme,
    pub accent_color: String,
    /// Show an extrapolated "full at" estimate under each bar, based on
    /// recent usage history. Off by default since it needs some history to
    /// become meaningful.
    #[serde(default)]
    pub show_estimated_time: bool,
    pub always_on_top: bool,
    pub opacity: f32,
    pub poll_interval_secs: u64,
    pub window_x: Option<i32>,
    pub window_y: Option<i32>,
    pub autostart: bool,
    /// Fixed color used for all usage bars, overriding the automatic
    /// good/warning/serious/critical severity coloring. `None` means
    /// automatic.
    #[serde(default)]
    pub bar_color: Option<String>,
    /// Scales the widget window size (and its content) relative to the
    /// base size. 1.0 is the default size.
    #[serde(default = "default_widget_scale")]
    pub widget_scale: f32,
    #[serde(default)]
    pub severity_thresholds: SeverityThresholds,
    /// Whether to show the reset/estimate countdown lines under the weekly
    /// bar too. The session (5h) bar's countdown always shows.
    #[serde(default = "default_true")]
    pub show_week_reset: bool,
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
            show_estimated_time: false,
            always_on_top: true,
            opacity: 0.96,
            // Every poll costs a small sliver of quota (see usage.rs) - 5
            // minutes is a reasonable default between freshness and cost.
            poll_interval_secs: 300,
            window_x: None,
            window_y: None,
            autostart: false,
            bar_color: None,
            widget_scale: 1.0,
            severity_thresholds: SeverityThresholds::default(),
            show_week_reset: true,
        }
    }
}

/// How many past snapshots to keep for the "estimated time to full" feature.
/// At the default 5-minute poll interval this covers close to 7 days, which
/// matches the longest rate-limit window we track.
const MAX_HISTORY: usize = 2000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct UsageCache {
    pub last_snapshot: Option<UsageSnapshot>,
    #[serde(default)]
    pub history: Vec<UsageSnapshot>,
}

impl UsageCache {
    /// Records a freshly fetched snapshot as both the latest snapshot and a
    /// new history point, pruning the oldest entries beyond `MAX_HISTORY`.
    pub fn record(&mut self, snapshot: UsageSnapshot) {
        self.last_snapshot = Some(snapshot.clone());
        self.history.push(snapshot);
        if self.history.len() > MAX_HISTORY {
            let excess = self.history.len() - MAX_HISTORY;
            self.history.drain(0..excess);
        }
    }

    /// Extracts `(timestamp, utilization, reset_unix)` triples for a single
    /// rate-limit window across history, for feeding into
    /// `estimate::estimate_full_at`. Points missing utilization or
    /// reset_unix are skipped.
    pub fn samples_for<'a>(
        &'a self,
        selector: impl Fn(&'a UsageSnapshot) -> Option<&'a RateWindow>,
    ) -> Vec<(i64, f64, i64)> {
        self.history
            .iter()
            .filter_map(|snap| {
                let window = selector(snap)?;
                Some((snap.fetched_at_unix, window.utilization?, window.reset_unix?))
            })
            .collect()
    }
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

    fn tmp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("uc-cache-test-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_snapshot(fetched_at_unix: i64, utilization: f64, reset_unix: i64) -> UsageSnapshot {
        UsageSnapshot {
            five_hour: Some(RateWindow {
                status: Some("allowed".into()),
                reset_unix: Some(reset_unix),
                utilization: Some(utilization),
                estimated_full_unix: None,
            }),
            seven_day: None,
            overage: None,
            representative_claim: Some("five_hour".into()),
            fetched_at_unix,
        }
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
        settings.show_estimated_time = true;
        settings.bar_color = Some("#2a78d6".to_string());
        settings.widget_scale = 1.25;
        settings.severity_thresholds = SeverityThresholds { warning: 0.5, serious: 0.75, critical: 0.9 };
        settings.show_week_reset = false;

        save_settings(&dir, &settings).unwrap();
        let loaded = load_settings(&dir).unwrap();
        assert_eq!(loaded, settings);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn loads_settings_file_from_before_thresholds_and_week_reset_existed() {
        // Shape of a real settings.json written by v0.1.3, before
        // severityThresholds/showWeekReset were added.
        let dir = tmp_dir("old-format");
        let old_json = r##"{
            "theme": "dark",
            "accentColor": "#d97757",
            "showEstimatedTime": true,
            "alwaysOnTop": true,
            "opacity": 0.9,
            "pollIntervalSecs": 300,
            "windowX": 10,
            "windowY": 20,
            "autostart": false,
            "barColor": null,
            "widgetScale": 1.0
        }"##;
        std::fs::write(dir.join("settings.json"), old_json).unwrap();

        let loaded = load_settings(&dir).unwrap();
        assert_eq!(loaded.theme, Theme::Dark);
        assert_eq!(loaded.show_estimated_time, true);
        assert_eq!(loaded.severity_thresholds, SeverityThresholds::default());
        assert_eq!(loaded.show_week_reset, true);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn round_trips_usage_cache() {
        let dir = tmp_dir("usagecache");
        let mut cache = UsageCache::default();
        cache.record(sample_snapshot(100, 0.42, 123));

        save_usage_cache(&dir, &cache).unwrap();
        let loaded = load_usage_cache(&dir).unwrap();
        assert_eq!(loaded, cache);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn record_prunes_history_beyond_the_cap() {
        let mut cache = UsageCache::default();
        for i in 0..(MAX_HISTORY + 10) {
            cache.record(sample_snapshot(i as i64, 0.1, 1000));
        }
        assert_eq!(cache.history.len(), MAX_HISTORY);
        // The oldest 10 entries should have been dropped, so the first
        // remaining entry is fetched_at_unix=10.
        assert_eq!(cache.history.first().unwrap().fetched_at_unix, 10);
    }

    #[test]
    fn samples_for_extracts_matching_window_triples() {
        let mut cache = UsageCache::default();
        cache.record(sample_snapshot(0, 0.1, 1000));
        cache.record(sample_snapshot(600, 0.3, 1000));

        let samples = cache.samples_for(|s| s.five_hour.as_ref());
        assert_eq!(samples, vec![(0, 0.1, 1000), (600, 0.3, 1000)]);

        let empty = cache.samples_for(|s| s.seven_day.as_ref());
        assert!(empty.is_empty());
    }
}
