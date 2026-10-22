//! A diagram as a `.kdraw` file (docs/design/diagrams.md): boxes, free
//! text, arrows between boxes and groups round them, each a line, on a
//! grid of whole units. The parser and the writer, the geometry the
//! canvas draws and hits by, the edits a gesture makes on the text, and
//! the export to Excalidraw's JSON — no drawing here; the pane is
//! kawoosh's `canvas.rs`.

pub mod edit;
pub mod excalidraw;
pub mod geom;
pub mod model;
pub mod parse;
pub mod write;

pub use edit::Change;
pub use geom::{Pt, Rect, Wire};
pub use model::{
    Color, Doc, Edge, Entry, Group, Heads, Item, LabelAlign, Node, Problem, Route, Shape, Text,
};
pub use parse::parse;

/// What a unit is in logical pixels at zoom 1, and in the export.
pub const UNIT: f32 = 20.0;
