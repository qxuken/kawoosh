//! What the nodes kawoosh draws say when an event reaches them: one
//! typed message per thing that can be pressed, dragged, scrolled or laid
//! out, declared on its node (`.on_click(Msg::Tab { index })`) and read
//! back beside the event (`ev.split::<Msg>()`, kui ADR 0047). The window's
//! own events arrive as the root's (ADR 0048). `Kawoosh::on_ui_event`
//! matches the pair, so a message two handlers share — a terminal's on
//! its keys, clicks, focus and wheel — is told apart by the event, never
//! by its order in a chain.
//!
//! On the wire each is `{kind, ...fields}`, the kind the variant's name in
//! snake_case (`TabClose` is `tab_close`). Lua sees two of them: the
//! engine's legend toggle (`legend`, `pane`) and a field's own message,
//! which is not one of these (`fields.rs`). A Lua view's messages are its
//! own and never read as these.

use kui_native::Message;

use crate::layout::PaneId;

#[derive(Message, Clone, Debug, PartialEq)]
pub enum Msg {
    // ---------------------------------------------------- the window's own
    /// The window back in front or sent behind (`on_focus`).
    Window,
    /// The machine's fonts changed (`on_fonts`).
    Fonts,
    /// Documents the OS asked the app to open (`on_open`).
    Open,
    /// The modifier keys held (`on_modifiers`).
    Modifiers,
    /// The wheel with ⌘ or Ctrl held, anywhere: the font's size.
    Zoom,
    /// A row chosen from one of kawoosh's menus: the bar's, or one opened
    /// over a node that declares it.
    Menu,

    // ---------------------------------------------------- a pane's frame
    /// Where a pane was laid out.
    Layout { pane: PaneId },
    /// A pane's title bar: a click focuses it.
    Title { pane: PaneId },
    /// A right-click a pane answers with its menu: the title bar's
    /// (`title`), or a terminal's grid.
    PaneMenu { pane: PaneId, title: bool },
    /// A title bar dragged: the pane moved.
    PaneDrag { pane: PaneId },
    /// A pane's close button.
    PaneClose { pane: PaneId },
    /// A divider: `path` names the split (`gap{i}` a strip's column,
    /// `d:` the dock's), `vw` the viewport's width for a strip's last.
    Split {
        path: String,
        dir: Along,
        vw: Option<f32>,
    },
    /// A legend's `⌥/ keys`: its pane's legend whole, or compact again.
    Legend { pane: PaneId },

    // ---------------------------------------------------- an editor pane
    /// The editor's sink: its keys, drags and wheel.
    Editor { pane: PaneId },
    /// The secondary button on the editor: its menu.
    EditButton { pane: PaneId },
    /// The editor's lines, laid out for the caret's row (no one reads it).
    Lines,
    /// A rendered markdown row, laid out (no one reads it).
    MarkdownRow,
    /// A hanging float beside a row, laid out (no one reads it).
    Hang,
    /// A breadcrumb in the title bar: its symbol's place.
    Crumb {
        pane: PaneId,
        line: i64,
        character: i64,
    },
    /// The completion's documentation panel, laid out (no one reads it).
    CompletionPanel,

    // ---------------------------------------------------- a terminal pane
    /// The terminal's sink: its keys, focus, wheel, and the ⌘-click.
    Term { pane: PaneId },
    /// The terminal's grid, laid out (no one reads it).
    TermGrid,
    /// A drag a program reporting the mouse hears.
    TermMouse { pane: PaneId },
    /// The middle button, and every button while a program reports.
    TermButton { pane: PaneId },
    /// The scrollbar's thumb.
    TermBar { pane: PaneId },
    /// The badge on a terminal scrolled away: back to the prompt.
    TermBottom { pane: PaneId },

    // ---------------------------------------------------- a diagram pane
    /// The canvas: its keys, drags and wheel.
    Canvas { pane: PaneId },
    /// Where the canvas was laid out.
    CanvasLayout { pane: PaneId },
    /// The middle and secondary buttons on the canvas.
    CanvasButton { pane: PaneId },
    /// The toolbar, which a drag does not move.
    CanvasBar,
    /// A toolbar button: its command line, on its pane.
    CanvasTool { pane: PaneId, run: String },

    // ---------------------------------------------------- a Lua pane
    /// The column a Lua view or a buffer's header draws in.
    LuaPane { pane: PaneId },
    /// Where a buffer's header was laid out.
    HeaderLayout { pane: PaneId },

    // ---------------------------------------------------- the chrome
    /// The title bar's cwd: listed.
    Cwd,
    /// A tab in the strip.
    Tab { index: usize },
    /// A tab dragged along the strip.
    TabDrag { index: usize },
    /// A tab's close button.
    TabClose { index: usize },
    /// A title-bar or status-line block's command line.
    Chrome { run: String },

    // ---------------------------------------------------- the floats
    /// A toast: hovered, or its body clicked.
    Toast { id: u64 },
    /// One of a toast's actions.
    ToastAction { id: u64, action: usize },
    /// The confirm's modal and sink: dismissed, or its keys.
    Confirm,
    /// One of the confirm's buttons.
    ConfirmAnswer { action: usize },

    // ---------------------------------------------------- the dev tabs
    /// The syntax tab's toggle for anonymous nodes.
    SyntaxAnonymous,
    /// A syntax tab row: its node's range selected.
    SyntaxSelect { start: usize, end: usize },
    /// A syntax tab row's fold.
    SyntaxFold { id: usize },
    /// The memory pane: its sink and its strip.
    Memory { pane: PaneId },
    /// A memory scope chip.
    MemoryScope { pane: PaneId, scope: String },
    /// A memory view chip.
    MemoryView { pane: PaneId, view: String },
    /// The memory pane's filter.
    MemoryFilter { pane: PaneId },
    /// A memory row.
    MemoryRow { pane: PaneId, row: usize },
    /// The undo pane: its sink.
    Undo { pane: PaneId },
    /// An undo row: its state.
    UndoState { pane: PaneId, state: usize },
}

impl Msg {
    /// The pane a message names, for a key on its sink: a terminal's
    /// releases and lone modifiers go to its program.
    pub fn pane(&self) -> Option<PaneId> {
        match *self {
            Msg::Term { pane }
            | Msg::Editor { pane }
            | Msg::Canvas { pane }
            | Msg::LuaPane { pane }
            | Msg::Memory { pane }
            | Msg::Undo { pane } => Some(pane),
            _ => None,
        }
    }
}

/// A divider's direction: `h` between columns, `v` between rows.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
#[message(string)]
pub enum Along {
    H,
    V,
}
