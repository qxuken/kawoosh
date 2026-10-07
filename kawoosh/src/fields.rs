//! A Lua view's field drawn by the engine (docs/design/lua-boundary.md
//! Decision 8): `ctx.field { … }` is a `fill { name = "field/…" }` in the
//! view's tree, and [`FieldDraw`] — a kui extension of the app's, loaded
//! by the Lua host the first frame it draws ([`LuaHost`]) — fills it with
//! the line `Kawoosh::field_line` draws for the app's own fields: tabs
//! and escapes drawn, every selection, a caret per selection, a block or
//! a bar on kui's blink, scrolled sideways under the caret. The view
//! draws no field of its own; kui's slots carry it (ADR 0014, an
//! extension hosting extensions), and a click on the field is a reply to
//! the view that declared it.
//!
//! The app gathers each field's scene from the engine before a Lua pane
//! is drawn (`Kawoosh::publish_field_scenes`) into [`Scenes`], which the
//! extension reads while the view is drawn: it draws without the app.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use kui_native::{Align, Extension, NodeSpec, Slot, Ui, UiEvent, Value};

use crate::look::Face;
use crate::panes::{FieldScene, draw_field};

/// The namespace the field extension fills under: `field/NAME`.
pub const NAMESPACE: &str = "field";

/// What the extension draws from: the chrome's face, and each Lua
/// field's scene by its full name (`lua:VIEW/NAME`) with whether the
/// keyboard is on it.
#[derive(Default)]
pub struct FieldScenes {
    pub face: Face,
    pub fields: HashMap<String, (FieldScene, bool)>,
}

pub type Scenes = Rc<RefCell<FieldScenes>>;

/// The `field` extension: every slot under its namespace a field, its
/// params `{ field, size, placeholder, focused }` from the view.
pub struct FieldDraw {
    scenes: Scenes,
    slots: Vec<String>,
}

impl FieldDraw {
    pub fn new(scenes: Scenes) -> Self {
        FieldDraw {
            scenes,
            slots: vec![kui_native::ANY_SLOT.to_string()],
        }
    }
}

fn param<'a>(params: &'a Value, key: &str) -> Option<&'a Value> {
    match params {
        Value::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
        _ => None,
    }
}

impl Extension for FieldDraw {
    fn name(&self) -> &str {
        NAMESPACE
    }

    fn slots(&self) -> &[String] {
        &self.slots
    }

    fn view(&mut self, slot: &Slot<'_>, ui: &mut Ui<'_>) -> Result<(), String> {
        let p = slot.params;
        let Some(Value::Str(full)) = param(p, "field") else {
            return Err(format!("{}: a field slot names no `field`", slot.name));
        };
        let scenes = self.scenes.borrow();
        let base = scenes.face;
        let size = match param(p, "size") {
            Some(Value::Float(s)) => *s as f32,
            Some(Value::Int(s)) => *s as f32,
            _ => base.size,
        };
        // The line as tall as the field's row, which is the text's size
        // and three either side, as the Lua field drew it.
        let face = Face {
            size,
            line_height: size + 6.0,
            ..base
        };
        // The pane has the keys, and not the command line over it: one
        // caret on the screen.
        let pane_keyed = !matches!(param(p, "focused"), Some(Value::Bool(false)));
        let click = Value::map([
            ("kind", Value::from("field")),
            ("field", Value::from(full.as_str())),
        ]);
        // Not open yet (asked for this frame): its row, empty, so
        // nothing moves when it is.
        let scene = scenes.fields.get(full.as_str());
        let keyed = scene.is_some_and(|(_, keys)| *keys) && pane_keyed;
        // Keyed by the field, as it was drawn in Lua: what finds its row.
        // The room its row gives (`"grow"`), so many pixels, or — by
        // default — as wide as its line (else its placeholder) and a
        // cell for the block caret past its end, as the Lua field was.
        let row = NodeSpec::row();
        let row = match param(p, "width") {
            Some(Value::Str(w)) if w == "grow" => row.grow_width(),
            Some(Value::Float(w)) => row.width(*w as f32),
            Some(Value::Int(w)) => row.width(*w as f32),
            _ => {
                let style = crate::rows::mono(face, &crate::Pal::default());
                let shown = match (scene, param(p, "placeholder")) {
                    (Some((s, _)), _) if !s.text.is_empty() => s.text.as_str(),
                    (_, Some(Value::Str(ph))) => ph.as_str(),
                    _ => "",
                };
                let w = ui.measure_text(shown, &style, None).width
                    + ui.measure_text(" ", &style, None).width;
                row.width(w)
            }
        };
        ui.with_keyed(
            &format!("field:{full}"),
            row.height(size + 6.0)
                .cross_align(Align::Center)
                .on_click(click),
            |ui| {
                let Some((scene, _)) = scene else { return };
                if scene.text.is_empty()
                    && !keyed
                    && let Some(Value::Str(placeholder)) = param(p, "placeholder")
                {
                    let style = crate::rows::mono(face, &scene.pal).color(scene.pal.dim);
                    // Clipped as the line is, where a narrow field cuts it.
                    ui.with_keyed(
                        "field",
                        NodeSpec::row()
                            .grow_width()
                            .clip()
                            .cross_align(Align::Center),
                        |ui| ui.text(placeholder, style),
                    );
                    return;
                }
                draw_field(ui, scene, keyed, face);
            },
        );
        Ok(())
    }

    /// A click on a field: the reply its view hears, `{ kind = "field",
    /// field = }`, as it heard its own field's before.
    fn on_event(&mut self, ev: &UiEvent) -> Vec<Value> {
        match param(&ev.payload, "kind") {
            Some(Value::Str(k)) if k == "field" => vec![ev.payload.clone()],
            _ => Vec::new(),
        }
    }
}

/// The Lua runtime's kui extension, and the field extension it loads
/// the first frame it draws: kui's way for a guest to bring one of its
/// own (`Ui::add_extension`), so whoever registers `lua` — the app, the
/// test harness, a test's drive — has the fields with it.
pub struct LuaHost {
    lua: kui_lua::LuaExtension,
    field: Option<FieldDraw>,
}

impl LuaHost {
    pub fn new(lua: kui_lua::LuaExtension, scenes: Scenes) -> Self {
        LuaHost {
            lua,
            field: Some(FieldDraw::new(scenes)),
        }
    }
}

impl Extension for LuaHost {
    fn name(&self) -> &str {
        self.lua.name()
    }

    fn slots(&self) -> &[String] {
        self.lua.slots()
    }

    fn view(&mut self, slot: &Slot<'_>, ui: &mut Ui<'_>) -> Result<(), String> {
        if let Some(field) = self.field.take()
            && let Err(e) = ui.add_extension(NAMESPACE, Box::new(field))
        {
            log::error!("the field extension: {e}");
        }
        self.lua.view(slot, ui)
    }

    fn on_event(&mut self, ev: &UiEvent) -> Vec<Value> {
        self.lua.on_event(ev)
    }
}
