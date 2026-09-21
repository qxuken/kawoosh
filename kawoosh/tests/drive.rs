//! The headless harness is the crate's (`kawoosh::harness`, roadmap
//! step 8): the same one `kawoosh test` drives a Lua script with, under
//! the name the tests have always used.

#![allow(dead_code, unused_imports)]

pub use kawoosh::harness::Harness as Drive;
