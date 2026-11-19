//! Release discovery and download shared by the desktop packages.
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Output};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use anyhow::{Context, Result, ensure};
use kawoosh_editor::{Layer, Settings};
use kawoosh_systems::spawn;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[cfg(target_os = "linux")]
use super::linux as platform;
#[cfg(target_os = "macos")]
use super::macos as platform;
#[cfg(unix)]
use super::unix::Staged;
#[cfg(windows)]
use super::windows as platform;
use crate::Kawoosh;
use crate::notify::{Level, Note, Show};

#[derive(Default)]
pub(super) struct ReleaseUpdate {
    job: Option<(bool, Receiver<Result<Outcome>>)>,
    #[cfg(unix)]
    pub(super) staged: Option<Staged>,
}

enum Outcome {
    Current,
    Available(Version),
    #[cfg(unix)]
    Staged(Staged),
    #[cfg(windows)]
    Ready,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    digest: Option<String>,
    browser_download_url: String,
}

impl Kawoosh {
    /// Startup checks for metadata. Downloading requires an explicit command.
    pub fn check_release(&mut self, download: bool) {
        let url = release_url(&self.ed.settings);
        if url.is_empty() {
            if download {
                self.ed.message = "Release updates are disabled in your settings".into();
            }
            return;
        }
        if self.update.waiting().is_some() {
            if download {
                self.ed.message = "A Kawoosh update is ready. Run :relaunch.".into();
            }
            return;
        }
        if self.update.release.job.is_some() {
            if download {
                self.ed.message = "A Kawoosh update check is already in progress".into();
            }
            return;
        }
        let Some((app, suffix)) = self
            .update
            .exe
            .as_deref()
            .and_then(platform::install_dir)
            .zip(asset_suffix())
        else {
            if download {
                self.ed.message = "Install a Kawoosh release package to use :update".into();
            }
            return;
        };
        let app = app.to_owned();
        let url = url.to_owned();
        let (send, receive) = mpsc::channel();
        let wake = self.wake.named("update");
        std::thread::spawn(move || {
            let _ = send.send(check(&app, &url, suffix, download));
            wake.wake();
        });
        self.update.release.job = Some((download, receive));
        if download {
            self.ed.message = "Checking for a Kawoosh update…".into();
        }
    }

    pub(super) fn sync_release(&mut self) {
        let Some((download, receiver)) = &self.update.release.job else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(anyhow::anyhow!("update check stopped")),
        };
        let download = *download;
        self.update.release.job = None;
        match result {
            #[cfg(unix)]
            Ok(Outcome::Staged(staged)) => {
                self.update.release.staged = Some(staged);
                if let Some(id) = self.update.toast.take() {
                    self.notes.dismiss(id);
                }
                self.update_seen();
            }
            #[cfg(windows)]
            Ok(Outcome::Ready) => self.update_seen(),
            Ok(Outcome::Available(version)) => {
                self.notify_with(
                    Note::new(Level::Info, format!("Kawoosh {version} is available"))
                        .source("update")
                        .show(Show::Toast)
                        .action("Download", "update"),
                );
            }
            Ok(Outcome::Current) if download => {
                self.ed.message = "No newer Kawoosh release is available".into();
            }
            Err(error) if download => {
                self.notify(Level::Error, format!("Kawoosh update failed. {error:#}"));
            }
            _ => {}
        }
    }
}

fn release_url(settings: &Settings) -> &str {
    [Layer::User, Layer::Default]
        .into_iter()
        .find_map(|layer| settings.layer_value(layer, "update.url"))
        .and_then(|setting| setting.as_str())
        .unwrap_or_default()
        .trim()
}

fn asset_suffix() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("macos-arm64.dmg"),
        ("windows", "x86_64") => Some("windows-x64-setup.exe"),
        ("linux", "x86_64") => Some("linux-x64.tar.gz"),
        ("linux", "aarch64") => Some("linux-arm64.tar.gz"),
        _ => None,
    }
}

fn check(app: &Path, url: &str, suffix: &str, download: bool) -> Result<Outcome> {
    require_https(url)?;
    #[cfg(target_os = "macos")]
    let team = platform::installed_team(app)?;
    let current = Version::parse(env!("CARGO_PKG_VERSION"))?;
    let response = spawn::output(curl(30)?.args([
        "-H",
        "Accept: application/vnd.github+json",
        "--write-out",
        "\n%{http_code}",
        "--url",
        url,
    ]))?;
    if !response.status.success() && response.stdout == b"\n404" {
        return Ok(Outcome::Current);
    }
    let response = checked(response)?;
    let body = response
        .strip_suffix(b"\n200")
        .context("Unexpected release metadata response")?;
    let release: Release = serde_json::from_slice(body)?;
    let Some((version, asset)) = newer_release(&release, &current, suffix)? else {
        return Ok(Outcome::Current);
    };
    if !download {
        return Ok(Outcome::Available(version));
    }

    let work = tempfile::tempdir()?;
    let payload = work.path().join(&asset.name);
    download_asset(asset, &payload)?;
    #[cfg(target_os = "macos")]
    return platform::stage(app, &team, version, &payload).map(Outcome::Staged);
    #[cfg(target_os = "linux")]
    return platform::stage(app, version, &payload).map(Outcome::Staged);
    #[cfg(windows)]
    {
        platform::stage(app, &payload)?;
        Ok(Outcome::Ready)
    }
}

fn newer_release<'a>(
    release: &'a Release,
    current: &Version,
    suffix: &str,
) -> Result<Option<(Version, &'a Asset)>> {
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let version = Version::parse(
        release
            .tag_name
            .strip_prefix('v')
            .context("Release tag must start with v")?,
    )?;
    if !version.pre.is_empty() || !version.cmp_precedence(current).is_gt() {
        return Ok(None);
    }
    let name = format!("Kawoosh-{version}-{suffix}");
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .with_context(|| format!("The latest release has no {suffix} asset"))?;
    Ok(Some((version, asset)))
}

fn require_https(url: &str) -> Result<()> {
    let authority = url
        .strip_prefix("https://")
        .and_then(|rest| rest.split(['/', '?', '#']).next());
    ensure!(
        authority.is_some_and(|host| !host.is_empty() && !host.contains('@'))
            && !url
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control() || byte == b'\\'),
        "Release URLs must use HTTPS without embedded credentials"
    );
    Ok(())
}

fn curl(timeout: u32) -> Result<Command> {
    #[cfg(unix)]
    let mut cmd = Command::new("/usr/bin/curl");
    #[cfg(windows)]
    let mut cmd = Command::new(
        std::path::PathBuf::from(
            std::env::var_os("SystemRoot").context("Windows system directory is missing")?,
        )
        .join("System32/curl.exe"),
    );
    #[cfg(target_os = "macos")]
    cmd.env("CURL_SSL_BACKEND", "securetransport");
    cmd.args([
        "-q",
        "-fsSL",
        "--globoff",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--connect-timeout",
        "20",
        "--max-time",
        &timeout.to_string(),
    ]);
    Ok(cmd)
}

pub(super) fn checked(output: Output) -> Result<Vec<u8>> {
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        let message = message.trim();
        ensure!(!message.is_empty(), "Command failed with {}", output.status);
        anyhow::bail!("{message}");
    }
    Ok(output.stdout)
}

fn download_asset(asset: &Asset, destination: &Path) -> Result<()> {
    require_https(&asset.browser_download_url)?;
    let digest = asset
        .digest
        .as_deref()
        .and_then(|s| s.strip_prefix("sha256:"))
        .filter(|s| s.len() == 64 && s.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .context("The release metadata did not supply a valid SHA-256 digest")?;
    checked(spawn::output(
        curl(600)?
            .args(["--max-filesize", "1073741824", "-o"])
            .arg(destination)
            .arg("--url")
            .arg(&asset.browser_download_url),
    )?)?;
    let mut hash = Sha256::new();
    let mut file = BufReader::new(File::open(destination)?);
    loop {
        let bytes = file.fill_buf()?;
        if bytes.is_empty() {
            break;
        }
        hash.update(bytes);
        let len = bytes.len();
        file.consume(len);
    }
    let actual: String = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    ensure!(
        actual.eq_ignore_ascii_case(digest),
        "Download checksum does not match the release"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kawoosh_editor::Setting;

    #[test]
    fn only_personal_settings_can_change_the_feed() {
        let mut settings = Settings::default();
        let default = release_url(&settings).to_owned();
        assert!(default.starts_with("https://api.github.com/"));
        for layer in [Layer::Project, Layer::Session] {
            settings.set(
                layer,
                "update.url",
                Setting::Str("https://other.example".into()),
            );
            assert_eq!(release_url(&settings), default);
        }
        settings.set(
            Layer::User,
            "update.url",
            Setting::Str(" https://mirror.example/latest ".into()),
        );
        assert_eq!(release_url(&settings), "https://mirror.example/latest");
        settings.set(Layer::User, "update.url", Setting::Str(String::new()));
        assert!(release_url(&settings).is_empty());
    }

    #[test]
    fn selects_each_platform_asset_from_the_configured_feed() {
        let current = Version::parse("0.0.9").unwrap();
        let mut release = Release {
            tag_name: "v0.0.10".into(),
            draft: false,
            prerelease: false,
            assets: vec![],
        };
        for suffix in [
            "macos-arm64.dmg",
            "windows-x64-setup.exe",
            "linux-x64.tar.gz",
            "linux-arm64.tar.gz",
        ] {
            let name = format!("Kawoosh-0.0.10-{suffix}");
            let url = format!("https://mirror.example/assets/{name}");
            release.assets.push(Asset {
                name,
                digest: None,
                browser_download_url: url.clone(),
            });
            let (version, asset) = newer_release(&release, &current, suffix).unwrap().unwrap();
            assert_eq!(version.to_string(), "0.0.10");
            assert_eq!(asset.browser_download_url, url);
        }
        release.prerelease = true;
        assert!(
            newer_release(&release, &current, "linux-x64.tar.gz")
                .unwrap()
                .is_none()
        );
        release.prerelease = false;
        release.tag_name = "v0.0.9+another-build".into();
        assert!(
            newer_release(&release, &current, "linux-x64.tar.gz")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn release_urls_require_https() {
        assert!(require_https("https://mirror.example/latest.json").is_ok());
        for url in [
            "http://mirror.example",
            "file:///tmp/latest.json",
            "https://user@mirror.example",
            "https:///latest",
            "https://a.example\\b",
        ] {
            assert!(require_https(url).is_err(), "{url}");
        }
    }
}
