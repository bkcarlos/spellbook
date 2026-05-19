//! Update checker.
//!
//! On startup we ask GitHub for the latest release of bkcarlos/spellbook
//! and compare its tag to our compile-time `CARGO_PKG_VERSION`. Result
//! is cached for 24h on disk so we don't hit GitHub every launch.
//!
//! Everything is best-effort. Network failure / parse failure → silent.
//! No automatic download / install — Homebrew owns that.

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

const REPO: &str = "bkcarlos/spellbook";
const USER_AGENT: &str = concat!("spellbook/", env!("CARGO_PKG_VERSION"), " update-check");
const CACHE_TTL_HOURS: i64 = 24;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    checked_at: DateTime<Utc>,
    latest_tag: String,
    release_url: String,
}

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub latest_version: String,
    pub release_url: String,
}

pub struct UpdateChecker {
    rx: Option<mpsc::Receiver<Result<CacheEntry>>>,
    info: Option<UpdateInfo>,
    failed_reason: Option<String>,
    last_check: Option<DateTime<Utc>>,
}

impl Default for UpdateChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl UpdateChecker {
    pub fn new() -> Self {
        let mut s = Self {
            rx: None,
            info: None,
            failed_reason: None,
            last_check: None,
        };
        // Apply cache immediately if fresh
        if let Some(cache) = load_cache() {
            s.last_check = Some(cache.checked_at);
            s.apply(&cache);
            if cache_is_fresh(&cache) {
                return s;
            }
        }
        // Otherwise spawn a background fetch
        s.kick_off_check();
        s
    }

    /// Force a fresh check (used by "Check now" button).
    pub fn kick_off_check(&mut self) {
        if self.rx.is_some() {
            return; // already in flight
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = fetch_latest();
            let _ = tx.send(result);
        });
        self.rx = Some(rx);
    }

    /// Drain pending result. Call every frame.
    pub fn poll(&mut self) {
        let Some(rx) = self.rx.as_ref() else { return };
        match rx.try_recv() {
            Ok(Ok(entry)) => {
                self.last_check = Some(entry.checked_at);
                self.apply(&entry);
                let _ = save_cache(&entry);
                self.failed_reason = None;
                self.rx = None;
            }
            Ok(Err(e)) => {
                self.failed_reason = Some(e.to_string());
                self.rx = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.failed_reason = Some("worker died".into());
                self.rx = None;
            }
        }
    }

    pub fn current_version(&self) -> &'static str {
        CURRENT_VERSION
    }

    pub fn update_info(&self) -> Option<&UpdateInfo> {
        self.info.as_ref()
    }

    pub fn checking(&self) -> bool {
        self.rx.is_some()
    }

    pub fn last_check(&self) -> Option<DateTime<Utc>> {
        self.last_check
    }

    pub fn failed_reason(&self) -> Option<&str> {
        self.failed_reason.as_deref()
    }

    /// True iff the latest release is strictly newer than current.
    pub fn has_update(&self) -> bool {
        let Some(info) = &self.info else { return false };
        compare_semver(&info.latest_version, CURRENT_VERSION) == std::cmp::Ordering::Greater
    }

    fn apply(&mut self, entry: &CacheEntry) {
        let latest = strip_v_prefix(&entry.latest_tag);
        self.info = Some(UpdateInfo {
            latest_version: latest.to_string(),
            release_url: entry.release_url.clone(),
        });
    }
}

fn fetch_latest() -> Result<CacheEntry> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .build();
    let resp = agent
        .get(&url)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| anyhow!("github api: {e}"))?;
    let json: serde_json::Value = resp.into_json().context("parse github response")?;
    let tag = json
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("no tag_name in response"))?;
    let html = json
        .get("html_url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("no html_url in response"))?;
    Ok(CacheEntry {
        checked_at: Utc::now(),
        latest_tag: tag.to_string(),
        release_url: html.to_string(),
    })
}

fn cache_path() -> Option<PathBuf> {
    crate::db::data_dir().ok().map(|d| d.join("update_cache.json"))
}

fn load_cache() -> Option<CacheEntry> {
    let path = cache_path()?;
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn save_cache(entry: &CacheEntry) -> Result<()> {
    let path = cache_path().ok_or_else(|| anyhow!("no cache path"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&path, serde_json::to_vec_pretty(entry)?)?;
    Ok(())
}

fn cache_is_fresh(entry: &CacheEntry) -> bool {
    let age = Utc::now().signed_duration_since(entry.checked_at);
    age.num_hours() < CACHE_TTL_HOURS
}

fn strip_v_prefix(tag: &str) -> &str {
    tag.strip_prefix('v').unwrap_or(tag)
}

/// Compare two "X.Y.Z" version strings numerically. Missing parts are
/// treated as 0; non-numeric components compare lexicographically. Robust
/// enough for the simple semver tags we use.
pub fn compare_semver(a: &str, b: &str) -> std::cmp::Ordering {
    let pa: Vec<u64> = a
        .split('.')
        .map(|x| x.parse::<u64>().unwrap_or(0))
        .collect();
    let pb: Vec<u64> = b
        .split('.')
        .map(|x| x.parse::<u64>().unwrap_or(0))
        .collect();
    for i in 0..pa.len().max(pb.len()) {
        let x = pa.get(i).copied().unwrap_or(0);
        let y = pb.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_compare_basic() {
        use std::cmp::Ordering::*;
        assert_eq!(compare_semver("0.1.2", "0.1.3"), Less);
        assert_eq!(compare_semver("0.1.3", "0.1.2"), Greater);
        assert_eq!(compare_semver("0.1.3", "0.1.3"), Equal);
        assert_eq!(compare_semver("1.0.0", "0.9.9"), Greater);
        assert_eq!(compare_semver("0.10.0", "0.9.0"), Greater);
    }

    #[test]
    fn semver_compare_uneven_lengths() {
        use std::cmp::Ordering::*;
        assert_eq!(compare_semver("1.0", "1.0.0"), Equal);
        assert_eq!(compare_semver("1.0", "1.0.1"), Less);
        assert_eq!(compare_semver("1", "1.0.0"), Equal);
    }

    #[test]
    fn semver_compare_nonnumeric_treated_as_zero() {
        use std::cmp::Ordering::*;
        // weird tags shouldn't panic
        assert_eq!(compare_semver("foo.bar", "0.0.0"), Equal);
        assert_eq!(compare_semver("0.1.0-rc1", "0.1.0"), Equal); // pre-release suffix ignored
    }

    #[test]
    fn strip_v_prefix_works() {
        assert_eq!(strip_v_prefix("v0.1.3"), "0.1.3");
        assert_eq!(strip_v_prefix("0.1.3"), "0.1.3");
    }

    #[test]
    fn cache_freshness() {
        let fresh = CacheEntry {
            checked_at: Utc::now() - chrono::Duration::hours(1),
            latest_tag: "v0.1.0".into(),
            release_url: "x".into(),
        };
        let stale = CacheEntry {
            checked_at: Utc::now() - chrono::Duration::hours(48),
            latest_tag: "v0.1.0".into(),
            release_url: "x".into(),
        };
        assert!(cache_is_fresh(&fresh));
        assert!(!cache_is_fresh(&stale));
    }
}
