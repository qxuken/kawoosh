//! kawoosh: a multiplexed terminal that is also an editor, on kui.
//!
//! The library half exists so tests can drive the same `App` headless
//! through a kui `Core` (docs/design/kui.md, Decision 8). `main.rs` is the
//! window.

pub mod app;
pub mod cmdline;
pub mod commands;
pub mod commands_pane;
pub mod compile;
pub mod confirm;
pub mod devtab;
pub mod diff;
pub mod graph;
pub mod history;
pub mod history_pane;
pub mod inspector;
pub mod languages;
pub mod layout;
pub mod logger;
pub mod lsp;
pub mod nodes;
pub mod notify;
pub mod palette;
pub mod panes;
pub mod perf;
pub mod plugins;
pub mod rows;
pub mod scripting;
pub mod session;
pub mod settings;
pub mod terminals;
pub mod undo;
pub mod whichkey;

pub use app::Kawoosh;
pub use palette::Pal;
