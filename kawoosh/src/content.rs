use std::path::PathBuf;

use gpui::{
    Action, App, Context, Entity, FocusHandle, Focusable, IntoElement, Render, ScrollHandle,
    SharedString, Window, colors::DefaultColors, div, prelude::*, rems,
};

use crate::OpenPath;
use crate::store::{AnyBuffer, CommandBuffer, DirectoryBuffer, DocumentBuffer, TerminalBuffer};

pub enum Content {
    FileBrowser(Entity<FileBrowserView>),
    Editor(Entity<EditorView>),
    Command(Entity<CommandView>),
    Terminal(Entity<TerminalView>),
}

impl Content {
    pub fn for_buffer(buffer: &AnyBuffer, cx: &mut App) -> Self {
        match buffer {
            AnyBuffer::Document(doc) => Self::Editor(cx.new(|cx| EditorView::new(doc.clone(), cx))),
            AnyBuffer::Directory(dir) => {
                Self::FileBrowser(cx.new(|cx| FileBrowserView::new(dir.clone(), cx)))
            }
            AnyBuffer::Command(cmd) => {
                Self::Command(cx.new(|cx| CommandView::new(cmd.clone(), cx)))
            }
            AnyBuffer::Terminal(term) => {
                Self::Terminal(cx.new(|cx| TerminalView::new(term.clone(), cx)))
            }
        }
    }

    pub fn title(&self, cx: &App) -> SharedString {
        match self {
            Self::FileBrowser(view) => view.read(cx).title(cx),
            Self::Editor(view) => view.read(cx).title(cx),
            Self::Command(view) => view.read(cx).title(cx),
            Self::Terminal(view) => view.read(cx).title(cx),
        }
    }
}

impl Focusable for Content {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self {
            Self::FileBrowser(view) => view.read(cx).focus_handle(cx),
            Self::Editor(view) => view.read(cx).focus_handle(cx),
            Self::Command(view) => view.read(cx).focus_handle(cx),
            Self::Terminal(view) => view.read(cx).focus_handle(cx),
        }
    }
}

impl Render for Content {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        match self {
            Self::FileBrowser(view) => view.clone().into_any_element(),
            Self::Editor(view) => view.clone().into_any_element(),
            Self::Command(view) => view.clone().into_any_element(),
            Self::Terminal(view) => view.clone().into_any_element(),
        }
    }
}

pub struct FileBrowserView {
    dir: Entity<DirectoryBuffer>,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
}

impl FileBrowserView {
    fn new(dir: Entity<DirectoryBuffer>, cx: &mut Context<Self>) -> Self {
        cx.observe(&dir, |_, _, cx| cx.notify()).detach();
        Self {
            dir,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
        }
    }

    fn title(&self, cx: &App) -> SharedString {
        self.dir.read(cx).title()
    }
}

impl Focusable for FileBrowserView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for FileBrowserView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.default_colors();
        let dir = self.dir.read(cx);
        let text_size = rems(1.);

        let entry_row = |id: usize, name: SharedString, path: PathBuf, is_dir: bool| {
            let label = if is_dir {
                format!("{name}/")
            } else {
                name.to_string()
            };
            div()
                .id(("dir-entry", id))
                .cursor_pointer()
                .px_1()
                .text_size(text_size)
                .line_height(text_size)
                .text_color(colors.text)
                .hover(|s| s.bg(colors.selected).text_color(colors.selected_text))
                .on_click(move |_event, window, cx| {
                    window.dispatch_action(OpenPath { path: path.clone() }.boxed_clone(), cx);
                })
                .child(label)
        };

        let mut rows = Vec::with_capacity(dir.entries.len() + 1);
        if let Some(parent) = dir.path.parent() {
            rows.push(entry_row(0, "..".into(), parent.to_path_buf(), true));
        }
        for (ix, entry) in dir.entries.iter().enumerate() {
            rows.push(entry_row(
                ix + 1,
                entry.name.clone(),
                entry.path.clone(),
                entry.is_dir,
            ));
        }

        div()
            .id("file-browser")
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .flex()
            .flex_col()
            .children(rows)
    }
}

pub struct EditorView {
    doc: Entity<DocumentBuffer>,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
}

impl EditorView {
    fn new(doc: Entity<DocumentBuffer>, cx: &mut Context<Self>) -> Self {
        cx.observe(&doc, |_, _, cx| cx.notify()).detach();
        Self {
            doc,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
        }
    }

    fn title(&self, cx: &App) -> SharedString {
        self.doc.read(cx).title()
    }
}

impl Focusable for EditorView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for EditorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.default_colors();
        let doc = self.doc.read(cx);
        let text_size = rems(1.);

        let lines = (0..doc.text.line_count()).map(|ix| {
            let line = doc.text.get_line(ix).unwrap_or_default();
            let line = String::from_utf8_lossy(&line).into_owned();
            div().id(("line", ix)).child(if line.is_empty() {
                " ".to_string()
            } else {
                line
            })
        });

        div()
            .id("editor")
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .flex()
            .flex_col()
            .px_1()
            .text_size(text_size)
            .line_height(text_size)
            .text_color(colors.text)
            .children(lines)
    }
}

/// Stub view.
pub struct CommandView {
    buffer: Entity<CommandBuffer>,
    focus_handle: FocusHandle,
}

impl CommandView {
    fn new(buffer: Entity<CommandBuffer>, cx: &mut Context<Self>) -> Self {
        Self {
            buffer,
            focus_handle: cx.focus_handle(),
        }
    }

    fn title(&self, cx: &App) -> SharedString {
        self.buffer.read(cx).title.clone()
    }
}

impl Focusable for CommandView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for CommandView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.default_colors();
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(colors.text)
            .child("Command (todo)")
    }
}

/// Stub view.
pub struct TerminalView {
    buffer: Entity<TerminalBuffer>,
    focus_handle: FocusHandle,
}

impl TerminalView {
    fn new(buffer: Entity<TerminalBuffer>, cx: &mut Context<Self>) -> Self {
        Self {
            buffer,
            focus_handle: cx.focus_handle(),
        }
    }

    fn title(&self, cx: &App) -> SharedString {
        self.buffer.read(cx).title.clone()
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.default_colors();
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(colors.text)
            .child("Terminal (todo)")
    }
}
