//! Verify and extract a notarized Developer ID disk image.
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, ensure};
use semver::Version;

use super::unix::Staged;
use kawoosh_systems::spawn;

const APP_ID: &str = "dev.qxuken.kawoosh";
const DMG_ID: &str = "dev.qxuken.kawoosh.dmg";

pub(super) fn install_dir(exe: &Path) -> Option<&Path> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    (exe.file_name()? == "kawoosh"
        && macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && app.extension()? == "app")
        .then_some(app)
}

fn run(command: &mut Command) -> Result<Vec<u8>> {
    let program = command.get_program().to_string_lossy().into_owned();
    spawn::output(command)
        .map_err(anyhow::Error::from)
        .and_then(super::release::checked)
        .with_context(|| program)
}

pub(super) fn installed_team(app: &Path) -> Result<String> {
    let output = spawn::output(
        Command::new("/usr/bin/codesign")
            .args(["-dv", "--verbose=4"])
            .arg(app),
    )?;
    let details = String::from_utf8_lossy(&output.stderr);
    let team = details
        .lines()
        .find_map(|line| line.strip_prefix("TeamIdentifier="))
        .filter(|team| {
            team.len() == 10
                && team
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        })
        .context("Install the Developer ID–signed Kawoosh.app to use GitHub updates")?;
    verify_signature(app, APP_ID, team)?;
    Ok(team.to_owned())
}

fn verify_signature(path: &Path, identifier: &str, team: &str) -> Result<()> {
    let requirement = format!(
        "anchor apple generic and identifier \"{identifier}\" and certificate leaf[subject.OU] = \"{team}\" and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists"
    );
    run(Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict", "-R", &requirement])
        .arg(path))?;
    Ok(())
}

pub(super) fn stage(app: &Path, team: &str, version: Version, dmg: &Path) -> Result<Staged> {
    let work = tempfile::tempdir()?;
    let staged = Staged::new(app, version)?;
    verify_signature(dmg, DMG_ID, team)?;
    run(Command::new("/usr/sbin/spctl")
        .args([
            "--assess",
            "--type",
            "open",
            "--context",
            "context:primary-signature",
            "--verbose=2",
        ])
        .arg(dmg))?;

    let mount = work.path().join("mount");
    fs::create_dir(&mount)?;
    run(Command::new("/usr/bin/hdiutil")
        .args([
            "attach",
            "-readonly",
            "-nobrowse",
            "-noautoopen",
            "-mountpoint",
        ])
        .arg(&mount)
        .arg(dmg))?;
    let bundle = staged.bundle();
    let copied = run(Command::new("/usr/bin/ditto")
        .arg(mount.join("Kawoosh.app"))
        .arg(&bundle));
    if let Err(error) = run(Command::new("/usr/bin/hdiutil")
        .args(["detach", "-force"])
        .arg(&mount))
    {
        let _ = work.keep();
        return Err(error).context("Cannot detach the update disk image");
    }
    copied?;
    verify_signature(&bundle, APP_ID, team)?;
    run(Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(&bundle))?;
    let bundle_version = run(Command::new("/usr/bin/plutil")
        .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
        .arg(bundle.join("Contents/Info.plist")))?;
    ensure!(
        String::from_utf8_lossy(&bundle_version).trim() == staged.version.to_string(),
        "The bundle version does not match the release tag"
    );
    Ok(staged)
}
