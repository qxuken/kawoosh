//! The systems (docs/design/mvp.md, Decision 6): threads behind message
//! channels, no async runtime. Each system is understandable, testable, and
//! replaceable from its message enum alone.

pub mod ts;
