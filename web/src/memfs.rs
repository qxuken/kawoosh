//! The page's disk: a file system held in memory, which every local path
//! goes to (`kawoosh_doc::fs::set_local_disk`). It starts as the demo's
//! tree and keeps what is written to it until the page is closed.

use std::collections::BTreeMap;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use kawoosh_doc::fs::{Entry, Fs, Stat};

enum Node {
    File { bytes: Vec<u8>, modified: u64 },
    Dir,
}

/// Paths are absolute and normalized: `/`, `/kawoosh`, `/kawoosh/a.rs`.
pub struct MemFs {
    nodes: Mutex<BTreeMap<PathBuf, Node>>,
    home: PathBuf,
}

fn not_found(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("{}: no such file or directory", path.display()),
    )
}

/// Seconds since the epoch, by the page's clock.
fn now() -> u64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl MemFs {
    pub fn new(home: PathBuf) -> Self {
        let fs = Self {
            nodes: Mutex::new(BTreeMap::from([(PathBuf::from("/"), Node::Dir)])),
            home: home.clone(),
        };
        fs.mkdirs(&home);
        fs
    }

    /// `path` as a key: `~` the home, `.` and `..` resolved, absolute.
    fn key(&self, path: &Path) -> PathBuf {
        let path = match path.strip_prefix("~") {
            Ok(rest) => self.home.join(rest),
            Err(_) => path.to_path_buf(),
        };
        let mut out = PathBuf::from("/");
        for c in path.components() {
            match c {
                Component::Normal(n) => out.push(n),
                Component::ParentDir => {
                    out.pop();
                }
                Component::RootDir | Component::CurDir | Component::Prefix(_) => {}
            }
        }
        out
    }

    fn mkdirs(&self, dir: &Path) {
        let dir = self.key(dir);
        let mut nodes = self.nodes.lock().unwrap();
        for d in dir.ancestors() {
            nodes.entry(d.to_path_buf()).or_insert(Node::Dir);
        }
    }

    /// A file of the demo's tree, its directories made.
    pub fn seed(&self, path: &str, bytes: &[u8]) {
        let key = self.key(Path::new(path));
        if let Some(dir) = key.parent() {
            self.mkdirs(dir);
        }
        self.nodes.lock().unwrap().insert(
            key,
            Node::File {
                bytes: bytes.to_vec(),
                modified: now(),
            },
        );
    }
}

impl Fs for MemFs {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        match self.nodes.lock().unwrap().get(&self.key(path)) {
            Some(Node::File { bytes, .. }) => Ok(bytes.clone()),
            Some(Node::Dir) => Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                format!("{}: is a directory", path.display()),
            )),
            None => Err(not_found(path)),
        }
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let key = self.key(path);
        let mut nodes = self.nodes.lock().unwrap();
        if !key
            .parent()
            .is_some_and(|d| matches!(nodes.get(d), Some(Node::Dir)))
        {
            return Err(not_found(path));
        }
        if matches!(nodes.get(&key), Some(Node::Dir)) {
            return Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                format!("{}: is a directory", path.display()),
            ));
        }
        nodes.insert(
            key,
            Node::File {
                bytes: bytes.to_vec(),
                modified: now(),
            },
        );
        Ok(())
    }

    fn stat(&self, path: &Path) -> io::Result<Stat> {
        match self.nodes.lock().unwrap().get(&self.key(path)) {
            Some(Node::File { bytes, modified }) => Ok(Stat {
                is_dir: false,
                is_file: true,
                is_symlink: false,
                size: bytes.len() as u64,
                modified: Some(*modified),
            }),
            Some(Node::Dir) => Ok(Stat {
                is_dir: true,
                is_file: false,
                is_symlink: false,
                size: 0,
                modified: None,
            }),
            None => Err(not_found(path)),
        }
    }

    fn list(&self, dir: &Path) -> io::Result<Vec<Entry>> {
        let key = self.key(dir);
        let nodes = self.nodes.lock().unwrap();
        match nodes.get(&key) {
            Some(Node::Dir) => {}
            Some(Node::File { .. }) => {
                return Err(io::Error::new(
                    io::ErrorKind::NotADirectory,
                    format!("{}: not a directory", dir.display()),
                ));
            }
            None => return Err(not_found(dir)),
        }
        Ok(nodes
            .iter()
            .filter(|(p, _)| p.parent() == Some(key.as_path()))
            .map(|(p, node)| {
                let name = p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                match node {
                    Node::File { bytes, modified } => Entry {
                        name,
                        is_dir: false,
                        is_symlink: false,
                        size: bytes.len() as u64,
                        modified: Some(*modified),
                    },
                    Node::Dir => Entry {
                        name,
                        is_dir: true,
                        is_symlink: false,
                        size: 0,
                        modified: None,
                    },
                }
            })
            .collect())
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let (from, to) = (self.key(from), self.key(to));
        if let Some(dir) = to.parent() {
            self.mkdirs(dir);
        }
        let mut nodes = self.nodes.lock().unwrap();
        if !nodes.contains_key(&from) {
            return Err(not_found(&from));
        }
        // The path and everything under it.
        let moved: Vec<PathBuf> = nodes
            .keys()
            .filter(|p| p.starts_with(&from))
            .cloned()
            .collect();
        for p in moved {
            if let Some(node) = nodes.remove(&p) {
                let rest = p.strip_prefix(&from).unwrap_or(Path::new(""));
                nodes.insert(to.join(rest), node);
            }
        }
        Ok(())
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        let key = self.key(path);
        let mut nodes = self.nodes.lock().unwrap();
        if !nodes.contains_key(&key) {
            return Err(not_found(path));
        }
        nodes.retain(|p, _| !p.starts_with(&key));
        Ok(())
    }

    fn create(&self, path: &Path, is_dir: bool) -> io::Result<()> {
        let key = self.key(path);
        if is_dir {
            self.mkdirs(&key);
            return Ok(());
        }
        if self.nodes.lock().unwrap().contains_key(&key) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{}: already exists", path.display()),
            ));
        }
        self.write(&key, b"")
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        let key = self.key(path);
        if self.nodes.lock().unwrap().contains_key(&key) {
            Ok(key)
        } else {
            Err(not_found(path))
        }
    }
}
