//! Global highlight definitions.
//!
//! A highlight is a *definition*, not an occurrence: document runs reference it
//! by [`HighlightId`], so retheming a definition updates every use without
//! touching a single run tree.

use std::collections::HashMap;

use crate::HighlightId;

/// Packed `0xAARRGGBB`. `core` deliberately does not depend on a UI colour
/// type; the renderer converts.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Rgba(pub u32);

/// UI-neutral style properties.
///
/// Every field is optional, and that is the point: a selection sets only `bg`,
/// a diagnostic sets only `underline`, syntax sets only `fg`. With mandatory
/// fields an overlapping layer has no way to say "leave this alone" and every
/// overlap either clobbers or needs a sentinel value.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HighlightStyle {
    pub fg: Option<Rgba>,
    pub bg: Option<Rgba>,
    pub underline: Option<Rgba>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub strikethrough: Option<bool>,
}

impl HighlightStyle {
    /// Layer `over` on top of `self`, field by field. Fields `over` leaves
    /// unset keep `self`'s value.
    pub fn compose(self, over: Self) -> Self {
        Self {
            fg: over.fg.or(self.fg),
            bg: over.bg.or(self.bg),
            underline: over.underline.or(self.underline),
            bold: over.bold.or(self.bold),
            italic: over.italic.or(self.italic),
            strikethrough: over.strikethrough.or(self.strikethrough),
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

bitflags::bitflags! {
    /// Orthogonal range properties.
    ///
    /// The [`Self::CONSTRAINTS`] subset is what the *edit* and *movement* paths
    /// consult; it is mirrored into a flattened index so those paths never walk
    /// the styling layers. The rest is read by the renderer.
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
    pub struct HighlightFlags: u8 {
        /// Text in this range cannot be modified.
        const READONLY = 1 << 0;
        /// The range is indivisible: a caret may not rest inside it and an edit
        /// may not cover it partially.
        const ATOMIC = 1 << 1;
        /// Selection skips over this range.
        const NO_SELECT = 1 << 2;
        /// The renderer omits this range.
        const HIDDEN = 1 << 3;

        /// Flags mirrored into the constraint index.
        const CONSTRAINTS = Self::READONLY.bits() | Self::ATOMIC.bits() | Self::NO_SELECT.bits();
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum HighlightKind {
    #[default]
    Item,
    Group(HighlightGroupKind),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HighlightGroupKind {
    Movement,
    Semantic,
    Presentation,
    Custom,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MetadataValue {
    Bool(bool),
    Integer(i64),
    Text(String),
}

/// The global, reusable description attached to a metadata run.
///
/// `style` and `properties` use a small data vocabulary rather than UI types so
/// that a renderer, a language service, and a future out-of-process plugin can
/// agree on property names without `core` depending on any of them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Highlight {
    /// Inherit unset style fields and flags from another definition.
    pub parent: Option<HighlightId>,
    pub kind: HighlightKind,
    pub style: HighlightStyle,
    pub flags: HighlightFlags,
    pub properties: HashMap<String, MetadataValue>,
}

impl Highlight {
    pub fn styled(style: HighlightStyle) -> Self {
        Self {
            style,
            ..Self::default()
        }
    }

    pub fn with_flags(mut self, flags: HighlightFlags) -> Self {
        self.flags = flags;
        self
    }

    pub fn with_parent(mut self, parent: HighlightId) -> Self {
        self.parent = Some(parent);
        self
    }
}

/// A definition plus its flattened inheritance.
///
/// Chains are resolved when a highlight is created (or when
/// [`Core::rebuild_styles`](crate::Core::rebuild_styles) is called after a
/// theme edit) rather than per chunk per frame.
#[derive(Clone, Debug)]
pub(crate) struct HighlightEntry {
    pub def: Highlight,
    pub resolved_style: HighlightStyle,
    pub resolved_flags: HighlightFlags,
}
