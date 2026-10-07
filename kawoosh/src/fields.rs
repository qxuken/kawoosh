//! What a Lua view draws through the engine (docs/design/lua-boundary.md
//! Decisions 8 and 9): its fields, key caps, legends and the way to a
//! legend. Each is a `fill { name = "engine/…", params = { kind, … } }`
//! in the view's tree, and [`EngineDraw`] — a kui extension of the
//! app's, loaded by the Lua host the first frame it draws ([`LuaHost`])
//! — fills it with what the app draws its own with:
//!
//! - `field`: the line `Kawoosh::field_line` draws the prompt with — tabs
//!   and escapes drawn, every selection, a caret per selection, a block
//!   or a bar on kui's blink, scrolled sideways under the caret;
//! - `keys`: a notation's caps (`icons::keys`);
//! - `legend`: a legend's items, wrapped between items
//!   (`icons::legend_items`);
//! - `toggle`: the way to a legend and back, `⌥/ keys` (`legends::toggle`).
//!
//! The view draws none of these itself; kui's slots carry them (ADR 0014,
//! an extension hosting extensions), and a click on a field or a toggle
//! is a reply to the view that declared it. Whether a legend is whole is
//! still the view's to say (`ctx.legend` gives nil while it is compact).
//!
//! The app gathers what the extension reads — each field's scene from the
//! engine, the palette, the icons — before a Lua pane or a header is
//! drawn (`Kawoosh::publish_drawing`) into [`Drawing`]: the extension
//! draws without the app.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use kui_native::{Align, Color, Extension, NodeSpec, Slot, TextStyle, Ui, UiEvent, Value};

use crate::icons::{CAP, KeyStyle};
use crate::look::Face;
use crate::panes::{FieldScene, draw_field};

/// The namespace the extension fills under: `engine/KIND:…`.
pub const NAMESPACE: &str = "engine";

/// What the extension draws from: the chrome's face and the palette as
/// the frame has them, the icons, and each Lua field's scene by its full
/// name (`lua:VIEW/NAME`) with whether the keyboard is on it.
#[derive(Default)]
pub struct Drawing {
    pub face: Face,
    pub pal: crate::Pal,
    pub icons: crate::icons::Shared,
    pub fields: HashMap<String, (FieldScene, bool)>,
}

pub type Shared = Rc<RefCell<Drawing>>;

/// The `engine` extension: every slot under its namespace one thing to
/// draw, by its params' `kind`.
pub struct EngineDraw {
    drawing: Shared,
    slots: Vec<String>,
}

impl EngineDraw {
    pub fn new(drawing: Shared) -> Self {
        EngineDraw {
            drawing,
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

fn number(params: &Value, key: &str) -> Option<f32> {
    match param(params, key) {
        Some(Value::Float(s)) => Some(*s as f32),
        Some(Value::Int(s)) => Some(*s as f32),
        _ => None,
    }
}

/// A colour a view passed: a theme's (`0xRRGGBBAA`) or `"#hex"`.
fn color(params: &Value, key: &str) -> Option<Color> {
    match param(params, key) {
        Some(Value::Int(n)) => Some(kui_native::schema::color_num(*n as u32)),
        Some(Value::Float(n)) => Some(kui_native::schema::color_num(*n as u32)),
        Some(Value::Str(s)) => kui_native::schema::color_hex_str(s).ok(),
        _ => None,
    }
}

/// `width`: the room its row gives (`"grow"`), so many pixels, `"fit"`,
/// or what the kind does without one.
fn width(row: NodeSpec, params: &Value, otherwise: impl FnOnce(NodeSpec) -> NodeSpec) -> NodeSpec {
    match param(params, "width") {
        Some(Value::Str(w)) if w == "grow" => row.grow_width(),
        Some(Value::Str(w)) if w == "fit" => row,
        Some(Value::Float(w)) => row.width(*w as f32),
        Some(Value::Int(w)) => row.width(*w as f32),
        _ => otherwise(row),
    }
}

impl EngineDraw {
    /// A view's caps' style: mono at `size` (the note size unless
    /// given), `color` (the muted one) and `border`, as the Lua caps were.
    fn key_style(d: &Drawing, p: &Value) -> KeyStyle {
        let size = number(p, "size").unwrap_or(d.face.size);
        KeyStyle::new(
            TextStyle::new(size)
                .mono()
                .color(color(p, "color").unwrap_or(d.pal.dim)),
            color(p, "border").unwrap_or(d.pal.border),
        )
    }

    fn field(d: &Drawing, p: &Value, ui: &mut Ui<'_>) -> Result<(), String> {
        let Some(Value::Str(full)) = param(p, "field") else {
            return Err("a field names no `field`".into());
        };
        let size = number(p, "size").unwrap_or(d.face.size);
        // The line as tall as the field's row, which is the text's size
        // and three either side, as the Lua field drew it.
        let face = Face {
            size,
            line_height: size + 6.0,
            ..d.face
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
        let scene = d.fields.get(full.as_str());
        let keyed = scene.is_some_and(|(_, keys)| *keys) && pane_keyed;
        // By default as wide as its line (else its placeholder) and a
        // cell for the block caret past its end, as the Lua field was.
        let row = width(NodeSpec::row(), p, |row| {
            let style = crate::rows::mono(face, &d.pal);
            let shown = match (scene, param(p, "placeholder")) {
                (Some((s, _)), _) if !s.text.is_empty() => s.text.as_str(),
                (_, Some(Value::Str(ph))) => ph.as_str(),
                _ => "",
            };
            let w = ui.measure_text(shown, &style, None).width
                + ui.measure_text(" ", &style, None).width;
            row.width(w)
        });
        // Keyed by the field, as it was drawn in Lua: what finds its row.
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

    fn keys(d: &Drawing, p: &Value, ui: &mut Ui<'_>) -> Result<(), String> {
        let Some(Value::Str(notation)) = param(p, "notation") else {
            return Err("keys name no `notation`".into());
        };
        crate::icons::keys(ui, &d.icons.borrow(), notation, &Self::key_style(d, p));
        Ok(())
    }

    fn legend(d: &Drawing, p: &Value, ui: &mut Ui<'_>) -> Result<(), String> {
        let Some(Value::List(list)) = param(p, "items") else {
            return Err("a legend has no `items`".into());
        };
        // Each item `{ keys = { notation, … }, words = }`.
        let mut items: Vec<(Vec<&str>, &str)> = Vec::new();
        for it in list {
            let keys = match param(it, "keys") {
                Some(Value::List(ks)) => ks
                    .iter()
                    .filter_map(|k| match k {
                        Value::Str(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            let words = match param(it, "words") {
                Some(Value::Str(w)) => w.as_str(),
                _ => "",
            };
            items.push((keys, words));
        }
        let items: Vec<(&[&str], &str)> = items.iter().map(|(k, w)| (k.as_slice(), *w)).collect();
        let size = number(p, "size").unwrap_or(d.face.size);
        let words = TextStyle::new(size).color(color(p, "word").unwrap_or(d.pal.faint));
        let style = Self::key_style(d, p);
        // Wrapped between its items, never inside one.
        let row = width(NodeSpec::row(), p, |row| row.grow_width());
        ui.with_keyed(
            "legend",
            row.gap(CAP.item_gap)
                .cross_gap(2.0)
                .wrap()
                .cross_align(Align::Center),
            |ui| crate::icons::legend_items(ui, &d.icons.borrow(), &items, &style, words),
        );
        Ok(())
    }

    fn toggle(d: &Drawing, p: &Value, ui: &mut Ui<'_>) -> Result<(), String> {
        let pane = number(p, "pane").unwrap_or(0.0) as crate::layout::PaneId;
        let full = matches!(param(p, "full"), Some(Value::Bool(true)));
        let size = number(p, "size").unwrap_or(d.face.size);
        let style = crate::legends::LegendStyle {
            keys: Self::key_style(d, p),
            words: TextStyle::new(size).color(color(p, "word").unwrap_or(d.pal.faint)),
            hover: d.pal.hover,
        };
        crate::legends::toggle(ui, &d.icons.borrow(), pane, full, &style, None);
        Ok(())
    }
}

impl Extension for EngineDraw {
    fn name(&self) -> &str {
        NAMESPACE
    }

    fn slots(&self) -> &[String] {
        &self.slots
    }

    fn view(&mut self, slot: &Slot<'_>, ui: &mut Ui<'_>) -> Result<(), String> {
        let d = self.drawing.borrow();
        let p = slot.params;
        let r = match param(p, "kind") {
            Some(Value::Str(k)) if k == "field" => Self::field(&d, p, ui),
            Some(Value::Str(k)) if k == "keys" => Self::keys(&d, p, ui),
            Some(Value::Str(k)) if k == "legend" => Self::legend(&d, p, ui),
            Some(Value::Str(k)) if k == "toggle" => Self::toggle(&d, p, ui),
            other => Err(format!("no kind {other:?} to draw")),
        };
        r.map_err(|e| format!("{}: {e}", slot.name))
    }

    /// A click on a field or a legend's toggle: the reply its view hears,
    /// `{ kind = "field", field = }` or `{ kind = "legend", pane = }`, as
    /// it heard its own before.
    fn on_event(&mut self, ev: &UiEvent) -> Vec<Value> {
        match param(&ev.payload, "kind") {
            Some(Value::Str(k)) if k == "field" || k == "legend" => vec![ev.payload.clone()],
            _ => Vec::new(),
        }
    }
}

/// The Lua runtime's kui extension, and the engine's drawing it loads the
/// first frame it draws: kui's way for a guest to bring one of its own
/// (`Ui::add_extension`), so whoever registers `lua` — the app, the test
/// harness, a test's drive — has fields, caps and legends with it.
pub struct LuaHost {
    lua: kui_lua::LuaExtension,
    engine: Option<EngineDraw>,
}

impl LuaHost {
    pub fn new(lua: kui_lua::LuaExtension, drawing: Shared) -> Self {
        LuaHost {
            lua,
            engine: Some(EngineDraw::new(drawing)),
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
        if let Some(engine) = self.engine.take()
            && let Err(e) = ui.add_extension(NAMESPACE, Box::new(engine))
        {
            log::error!("the engine's drawing for Lua views: {e}");
        }
        self.lua.view(slot, ui)
    }

    fn on_event(&mut self, ev: &UiEvent) -> Vec<Value> {
        self.lua.on_event(ev)
    }
}
