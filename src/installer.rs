//! Self-updater for macOS .app installs.
//!
//! Downloads `Spellbook-X.Y.Z.dmg` from the GitHub release, mounts it,
//! replaces `/Applications/Spellbook.app`, strips quarantine, and offers a
//! one-click restart. No external CLI dependency (brew/curl) at runtime.
//!
//! Other install paths (brew formula CLI, raw cargo install, Windows .exe,
//! Linux binary) are intentionally NOT auto-updated — they fall back to
//! "open release notes" in the UI.

use anyhow::{anyhow, bail, Context, Result};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    /// Inside /Applications/Spellbook.app/Contents/MacOS/spellbook
    MacApp,
}

/// Best-effort detection of how Spellbook was installed. Only macOS .app
/// is currently supported for auto-update.
pub fn detect_install_kind() -> Option<InstallKind> {
    let exe = std::env::current_exe().ok()?;
    let s = exe.to_string_lossy();
    if s.contains("/Applications/Spellbook.app/Contents/MacOS/") {
        Some(InstallKind::MacApp)
    } else {
        None
    }
}

#[derive(Debug, Clone)]
pub enum InstallState {
    Idle,
    Downloading { received: u64, total: u64 },
    Installing(String), // status text
    Done,
    Failed(String),
}

#[derive(Debug)]
enum Msg {
    Progress { received: u64, total: u64 },
    Status(String),
    Done,
    Failed(String),
}

pub struct Installer {
    state: InstallState,
    rx: Option<mpsc::Receiver<Msg>>,
}

impl Default for Installer {
    fn default() -> Self {
        Self::new()
    }
}

impl Installer {
    pub fn new() -> Self {
        Self {
            state: InstallState::Idle,
            rx: None,
        }
    }

    pub fn state(&self) -> &InstallState {
        &self.state
    }

    pub fn busy(&self) -> bool {
        matches!(
            self.state,
            InstallState::Downloading { .. } | InstallState::Installing(_)
        )
    }

    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.state = InstallState::Idle;
        self.rx = None;
    }

    /// Drain channel each frame.
    pub fn poll(&mut self) {
        let Some(rx) = self.rx.as_ref() else { return };
        loop {
            match rx.try_recv() {
                Ok(Msg::Progress { received, total }) => {
                    self.state = InstallState::Downloading { received, total };
                }
                Ok(Msg::Status(s)) => {
                    self.state = InstallState::Installing(s);
                }
                Ok(Msg::Done) => {
                    self.state = InstallState::Done;
                    self.rx = None;
                    return;
                }
                Ok(Msg::Failed(e)) => {
                    self.state = InstallState::Failed(e);
                    self.rx = None;
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.state =
                        InstallState::Failed("update worker disconnected".into());
                    self.rx = None;
                    return;
                }
            }
        }
    }

    /// Spawn the macOS install worker. Idempotent if already running.
    pub fn start_mac(&mut self, version: &str) {
        if self.rx.is_some() {
            return;
        }
        let v = version.to_string();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            if let Err(e) = run_mac_install(&v, &tx) {
                let _ = tx.send(Msg::Failed(e.to_string()));
            }
        });
        self.rx = Some(rx);
        self.state = InstallState::Downloading {
            received: 0,
            total: 0,
        };
    }
}

/// Relaunch /Applications/Spellbook.app and exit current process.
pub fn restart_mac_app() -> Result<()> {
    Command::new("open")
        .args(["-n", "/Applications/Spellbook.app"])
        .spawn()
        .context("open -n /Applications/Spellbook.app")?;
    // Give the new process a moment to come up before the OS reaps us.
    std::thread::sleep(Duration::from_millis(400));
    std::process::exit(0);
}

fn run_mac_install(version: &str, tx: &mpsc::Sender<Msg>) -> Result<()> {
    let url = format!(
        "https://github.com/bkcarlos/spellbook/releases/download/v{}/Spellbook-{}.dmg",
        version, version
    );
    let sha_url = format!("{}.sha256", url);

    let _ = tx.send(Msg::Status("获取校验和…".into()));
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .build();
    let expected_sha: Option<String> = agent
        .get(&sha_url)
        .call()
        .ok()
        .and_then(|r| r.into_string().ok())
        .map(|s| s.trim().to_lowercase());

    let _ = tx.send(Msg::Status("下载 DMG…".into()));
    let resp = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(300))
        .build()
        .get(&url)
        .call()
        .map_err(|e| anyhow!("download: {e}"))?;
    let total: u64 = resp
        .header("Content-Length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let tmp_dir = std::env::temp_dir().join("spellbook-update");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir)?;
    let dmg_path = tmp_dir.join(format!("Spellbook-{}.dmg", version));

    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(&dmg_path)?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut received: u64 = 0;
    let mut last_emit: u64 = 0;
    loop {
        let n = reader.read(&mut buf).context("read response")?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
        received += n as u64;
        // throttle UI updates to every ~64 KB (or every read if total unknown)
        if received - last_emit >= 64 * 1024 {
            let _ = tx.send(Msg::Progress {
                received,
                total: total.max(received),
            });
            last_emit = received;
        }
    }
    drop(file);
    let _ = tx.send(Msg::Progress {
        received,
        total: received,
    });

    if let Some(expected) = expected_sha {
        let actual = format!("{:x}", hasher.finalize());
        if !actual.eq_ignore_ascii_case(&expected) {
            bail!("校验和不匹配 (expected {}, got {})", expected, actual);
        }
    }

    let _ = tx.send(Msg::Status("挂载 DMG…".into()));
    let mount_dir = tmp_dir.join("mount");
    std::fs::create_dir_all(&mount_dir)?;
    let out = Command::new("hdiutil")
        .args(["attach", "-nobrowse", "-mountpoint"])
        .arg(&mount_dir)
        .arg(&dmg_path)
        .output()
        .context("hdiutil attach")?;
    if !out.status.success() {
        bail!("hdiutil attach failed: {}", String::from_utf8_lossy(&out.stderr));
    }

    let src_app = mount_dir.join("Spellbook.app");
    if !src_app.exists() {
        let _ = Command::new("hdiutil").arg("detach").arg(&mount_dir).output();
        bail!("DMG 里没有 Spellbook.app");
    }
    let dst_new: PathBuf = PathBuf::from("/Applications/Spellbook.app.new");
    let dst_app: &Path = Path::new("/Applications/Spellbook.app");

    let _ = tx.send(Msg::Status("复制新版到 /Applications…".into()));
    // clean any stale .new from a prior aborted run
    let _ = std::fs::remove_dir_all(&dst_new);
    let out = Command::new("cp")
        .arg("-R")
        .arg(&src_app)
        .arg(&dst_new)
        .output()
        .context("cp -R")?;
    let _ = Command::new("hdiutil").arg("detach").arg(&mount_dir).output();
    if !out.status.success() {
        let _ = std::fs::remove_dir_all(&dst_new);
        bail!(
            "复制失败 (可能权限不足): {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let _ = tx.send(Msg::Status("替换旧版…".into()));
    if dst_app.exists() {
        let out = Command::new("rm").arg("-rf").arg(dst_app).output()?;
        if !out.status.success() {
            let _ = std::fs::remove_dir_all(&dst_new);
            bail!("rm old: {}", String::from_utf8_lossy(&out.stderr));
        }
    }
    let out = Command::new("mv").arg(&dst_new).arg(dst_app).output()?;
    if !out.status.success() {
        bail!("mv: {}", String::from_utf8_lossy(&out.stderr));
    }

    // Strip the quarantine attr that macOS sets on .app payloads pulled out
    // of a downloaded DMG, so the next launch doesn't show a Gatekeeper
    // dialog. Same as the cask postflight.
    let _ = Command::new("xattr")
        .args(["-dr", "com.apple.quarantine"])
        .arg(dst_app)
        .output();

    // Cleanup
    let _ = std::fs::remove_file(&dmg_path);

    let _ = tx.send(Msg::Done);
    Ok(())
}
