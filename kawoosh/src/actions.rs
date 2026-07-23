use std::path::PathBuf;

use gpui::{Action, actions};
use schemars::JsonSchema;
use serde::Deserialize;

actions!(kawoosh, [Quit, TabNew, TabNext, TabPrev]);

#[derive(Clone, PartialEq, Deserialize, JsonSchema, Action)]
#[action(namespace = kawoosh)]
pub struct TabSelect {
    pub idx: usize,
}

#[derive(Clone, PartialEq, Deserialize, JsonSchema, Action)]
#[action(namespace = kawoosh)]
pub struct TabClose {
    pub idx: Option<usize>,
}

/// Open a path in the selected tab: directories become a file browser,
/// files become an editor. This is the content-kind switch trigger.
#[derive(Clone, PartialEq, Deserialize, JsonSchema, Action)]
#[action(namespace = kawoosh)]
pub struct OpenPath {
    pub path: PathBuf,
}
