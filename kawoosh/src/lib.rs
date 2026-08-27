//! kawoosh application logic, UI-framework-free.
//!
//! Everything here runs headless: the SDL binary (`main.rs`) is a thin event
//! source + painter around [`App`], and tests drive [`App`] directly.

pub mod app;
pub mod paint;
pub mod text;
