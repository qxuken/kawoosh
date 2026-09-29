//! kawoosh: a multiplexed terminal that is also an editor, on kui.
//!
//! The library half exists so tests can drive the same `App` headless
//! through a kui `Core` (docs/design/kui.md, Decision 8). `main.rs` is the
//! window.

/// A terminal's `$EDITOR`: the small binary of that name
/// (`src/bin/edit.rs`, `app::shipped_editor`), or this one linked
/// under that name (`app::editor_link`), which then answers as
/// `kawoosh edit --wait`.
pub const EDITOR_SHIM: &str = "kawoosh-edit";

pub mod app;
pub mod breadcrumbs;
pub mod chrome;
pub mod cmdline;
pub mod commands;
pub mod compile;
pub mod confirm;
pub mod deduce;
pub mod devtab;
pub mod diff;
pub mod disk;
pub mod dock;
pub mod domains;
pub mod du;
pub mod editorconfig;
pub mod fonts;
pub mod format;
pub mod graph;
pub mod harness;
pub mod help;
pub mod history;
pub mod inspector;
pub mod languages;
pub mod launcher;
pub mod layout;
pub mod links;
pub mod listing;
pub mod lists;
pub mod logger;
pub mod look;
pub mod lsp;
pub mod lsp_logs;
pub mod lsp_rules;
pub mod markdown;
pub mod marks;
pub mod memory;
pub mod moments;
pub mod multis;
pub mod nodes;
pub mod notify;
pub mod palette;
pub mod panes;
pub mod perf;
pub mod plugins;
pub mod rows;
pub mod scripting;
pub mod scroll_probe;
pub mod secrets;
pub mod session;
pub mod settings;
pub mod term_images;
pub mod terminals;
pub mod theme_check;
pub mod themes;
pub mod trust;
pub mod types;
pub mod undo;
pub mod whichkey;
pub mod wrap;

pub use app::Kawoosh;
pub use palette::Pal;
