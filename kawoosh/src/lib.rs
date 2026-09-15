//! kawoosh: a multiplexed terminal that is also an editor, on kui.
//!
//! The library half exists so tests can drive the same `App` headless
//! through a kui `Core` (docs/design/kui.md, Decision 8). `main.rs` is the
//! window.

pub mod app;
pub mod palette;
pub mod rows;

pub use app::Kawoosh;
pub use palette::Pal;
