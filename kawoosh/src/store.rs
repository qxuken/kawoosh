use std::path::{Path, PathBuf};

use gpui::{AppContext, Context, Entity, SharedString};
use text_buffer::Buffer as TextBuffer;

pub struct Store {
    pub cwd: PathBuf,
    pub documents: Vec<Entity<DocumentBuffer>>,
}

impl Store {
    pub fn new(cwd: Option<PathBuf>) -> Self {
        Self {
            cwd: cwd.unwrap_or_default(),
            documents: Vec::new(),
        }
    }

    pub fn new_directory(&mut self, path: PathBuf, cx: &mut Context<Self>) -> AnyBuffer {
        AnyBuffer::Directory(cx.new(|_| DirectoryBuffer::scan(path)))
    }

    pub fn open_document(&mut self, path: PathBuf, cx: &mut Context<Self>) -> AnyBuffer {
        if let Some(doc) = self
            .documents
            .iter()
            .find(|doc| doc.read(cx).path.as_deref() == Some(path.as_path()))
        {
            return AnyBuffer::Document(doc.clone());
        }

        let doc = cx.new(|_| DocumentBuffer::load(path));
        self.documents.push(doc.clone());
        AnyBuffer::Document(doc)
    }
}

pub enum AnyBuffer {
    Document(Entity<DocumentBuffer>),
    Directory(Entity<DirectoryBuffer>),
    #[allow(dead_code)]
    Command(Entity<CommandBuffer>),
    #[allow(dead_code)]
    Terminal(Entity<TerminalBuffer>),
}

pub struct DocumentBuffer {
    pub path: Option<PathBuf>,
    pub text: TextBuffer,
    pub dirty: bool,
}

impl DocumentBuffer {
    fn load(path: PathBuf) -> Self {
        let mut text = TextBuffer::new();
        if let Ok(bytes) = std::fs::read(&path) {
            text.set_text(&bytes);
        }
        Self {
            path: Some(path),
            text,
            dirty: false,
        }
    }

    pub fn title(&self) -> SharedString {
        let name = self
            .path
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".to_string());
        if self.dirty {
            format!("{name}*").into()
        } else {
            name.into()
        }
    }
}

pub struct DirEntry {
    pub name: SharedString,
    pub path: PathBuf,
    pub is_dir: bool,
}

pub struct DirectoryBuffer {
    pub path: PathBuf,
    pub entries: Vec<DirEntry>,
}

impl DirectoryBuffer {
    fn scan(path: PathBuf) -> Self {
        let mut entries: Vec<DirEntry> = std::fs::read_dir(&path)
            .map(|reader| {
                reader
                    .filter_map(Result::ok)
                    .map(|entry| DirEntry {
                        name: entry.file_name().to_string_lossy().into_owned().into(),
                        is_dir: entry.file_type().is_ok_and(|kind| kind.is_dir()),
                        path: entry.path(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));

        Self { path, entries }
    }

    pub fn title(&self) -> SharedString {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned())
            .into()
    }
}

pub struct CommandBuffer {
    pub title: SharedString,
}

impl CommandBuffer {
    #[allow(dead_code)]
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
        }
    }
}

pub struct TerminalBuffer {
    pub title: SharedString,
}

impl TerminalBuffer {
    #[allow(dead_code)]
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
        }
    }
}
