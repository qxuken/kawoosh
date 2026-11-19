//! Only a portable Kawoosh release owns the entire installation directory.
use std::fs::{self, File};
use std::path::{Component, Path};

use anyhow::{Context, Result, ensure};
use flate2::read::GzDecoder;
use semver::Version;

use super::unix::Staged;

const MARKER: &str = ".kawoosh-release";

pub(super) fn install_dir(exe: &Path) -> Option<&Path> {
    let bin = exe.parent()?;
    let prefix = bin.parent()?;
    if exe.file_name()? != "kawoosh"
        || bin.file_name()? != "bin"
        || prefix.file_name()? != "Kawoosh"
    {
        return None;
    }
    let marker = fs::read_to_string(prefix.join(MARKER)).ok()?;
    (Version::parse(marker.trim()).ok()? == Version::parse(env!("CARGO_PKG_VERSION")).ok()?)
        .then_some(prefix)
}

pub(super) fn stage(app: &Path, version: Version, payload: &Path) -> Result<Staged> {
    let staged = Staged::new(app, version)?;
    let bundle = staged.bundle();
    unpack(payload, bundle.parent().context("No staging directory")?)?;
    let marker = fs::read_to_string(bundle.join(MARKER))
        .context("The archive is missing its release marker")?;
    ensure!(
        marker.trim() == staged.version.to_string(),
        "The archive version does not match the release tag"
    );
    ensure!(
        bundle.join("bin/kawoosh").is_file() && bundle.join("bin/kawoosh-edit").is_file(),
        "The release archive is missing its binaries"
    );
    Ok(staged)
}

fn unpack(payload: &Path, destination: &Path) -> Result<()> {
    let mut archive = tar::Archive::new(GzDecoder::new(File::open(payload)?));
    let mut remaining = 1024 * 1024 * 1024_u64;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let header = entry.header();
        ensure!(
            header.entry_type().is_file() || header.entry_type().is_dir(),
            "Unexpected entry type in release archive"
        );
        remaining = remaining
            .checked_sub(entry.size())
            .context("Release archive is too large")?;
        let path = entry.path()?;
        let mut components = path.components();
        ensure!(
            components.next() == Some(Component::Normal("Kawoosh".as_ref())),
            "Unexpected release archive root"
        );
        ensure!(
            (match components.next() {
                None => header.entry_type().is_dir(),
                Some(Component::Normal(root)) => root == "bin" || root == "share" || root == MARKER,
                _ => false,
            }) && components.all(|part| matches!(part, Component::Normal(_))),
            "Unexpected path in release archive"
        );
        ensure!(
            entry.unpack_in(destination)?,
            "Cannot extract release archive entry"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn stages_a_portable_archive_and_checks_its_version() {
        let work = tempfile::tempdir().unwrap();
        let app = work.path().join("Kawoosh");
        let source = work.path().join("source").join("Kawoosh");
        fs::create_dir_all(source.join("bin")).unwrap();
        fs::write(source.join(MARKER), "0.0.2\n").unwrap();
        for name in ["kawoosh", "kawoosh-edit"] {
            let path = source.join("bin").join(name);
            fs::write(&path, "example").unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let payload = work.path().join("release.tar.gz");
        let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
            File::create(&payload).unwrap(),
            flate2::Compression::fast(),
        ));
        archive.append_dir_all("Kawoosh", source).unwrap();
        archive.into_inner().unwrap().finish().unwrap();

        assert!(stage(&app, Version::new(0, 0, 3), &payload).is_err());
        let staged = stage(&app, Version::new(0, 0, 2), &payload).unwrap();
        let exe = staged.bundle().join("bin/kawoosh");
        assert!(exe.metadata().unwrap().permissions().mode() & 0o111 != 0);
        assert!(!app.exists());
    }

    #[test]
    fn rejects_links_in_the_archive() {
        let work = tempfile::tempdir().unwrap();
        let file = work.path().join("update.tar.gz");
        let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
            File::create(&file).unwrap(),
            flate2::Compression::fast(),
        ));
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_mode(0o755);
        header.set_size(0);
        archive
            .append_link(&mut header, "Kawoosh/bin", "/tmp")
            .unwrap();
        archive.into_inner().unwrap().finish().unwrap();
        assert!(unpack(&file, work.path()).is_err());
        assert!(!work.path().join("Kawoosh/bin").exists());
    }
}
