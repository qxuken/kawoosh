//! The diagram as Excalidraw's JSON (docs/design/diagrams.md Decision
//! 6): its shapes, their labels bound to them, its arrows bound at both
//! ends, its groups as dashed frames. Sharp and monospace, so it opens
//! engineered rather than sketched; the same file exports the same JSON.

use serde_json::{Value, json};

use crate::UNIT;
use crate::geom::{Layout, Rect, group_rect, node_rect};
use crate::model::*;

/// Excalidraw's monospace family.
const MONO: i64 = 3;
const FONT: f32 = 16.0;
const LINE: f32 = 1.25;

/// A colour as Excalidraw's palette has it: the stroke, the fill.
fn colors(c: Option<Color>) -> (&'static str, &'static str) {
    match c {
        None => ("#1e1e1e", "transparent"),
        Some(Color::Red) => ("#e03131", "#ffc9c9"),
        Some(Color::Orange) => ("#e8590c", "#ffd8a8"),
        Some(Color::Yellow) => ("#f08c00", "#ffec99"),
        Some(Color::Green) => ("#2f9e44", "#b2f2bb"),
        Some(Color::Teal) => ("#0c8599", "#99e9f2"),
        Some(Color::Blue) => ("#1971c2", "#a5d8ff"),
        Some(Color::Purple) => ("#6741d9", "#d0bfff"),
        Some(Color::Gray) => ("#495057", "#e9ecef"),
    }
}

/// A number from a string, the same each time: an element's seed.
fn seed(s: &str) -> i64 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    (h & 0x7fff_ffff) as i64
}

fn px(u: f32) -> f64 {
    (u * UNIT) as f64
}

/// What every element carries.
fn base(id: &str, kind: &str, r: Rect, stroke: &str, bg: &str) -> serde_json::Map<String, Value> {
    let v = json!({
        "id": id,
        "type": kind,
        "x": px(r.x),
        "y": px(r.y),
        "width": px(r.w),
        "height": px(r.h),
        "angle": 0,
        "strokeColor": stroke,
        "backgroundColor": bg,
        "fillStyle": "solid",
        "strokeWidth": 2,
        "strokeStyle": "solid",
        "roughness": 0,
        "opacity": 100,
        "groupIds": [],
        "frameId": null,
        "roundness": null,
        "seed": seed(id),
        "version": 1,
        "versionNonce": seed(&format!("{id}#")),
        "isDeleted": false,
        "boundElements": [],
        "updated": 1,
        "link": null,
        "locked": false,
    });
    match v {
        Value::Object(m) => m,
        _ => unreachable!(),
    }
}

/// How big a label is in px, by its lines and their characters: what
/// Excalidraw measures again when it opens the file.
fn text_px(label: &str) -> (f32, f32) {
    let lines: Vec<&str> = label.split('\n').collect();
    let widest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f32;
    (widest * FONT * 0.6, lines.len() as f32 * FONT * LINE)
}

/// A text element: free at `at`, or bound to `container` and centred in
/// `around`.
fn text(id: &str, label: &str, color: &str, place: TextAt) -> Value {
    let (w, h) = text_px(label);
    let (x, y, container, align, valign) = match place {
        TextAt::Free(x, y) => (x, y, Value::Null, "left", "top"),
        TextAt::In(container, r, LabelAlign::Center) => {
            let c = r.center();
            (
                px(c.x) as f32 - w / 2.0,
                px(c.y) as f32 - h / 2.0,
                Value::String(container.to_string()),
                "center",
                "middle",
            )
        }
        // From the top, against a side: the canvas's half a unit in.
        TextAt::In(container, r, side) => {
            let pad = px(0.5) as f32;
            let x = match side {
                LabelAlign::Right => px(r.right()) as f32 - pad - w,
                _ => px(r.x) as f32 + pad,
            };
            (
                x,
                px(r.y) as f32 + pad,
                Value::String(container.to_string()),
                side.word(),
                "top",
            )
        }
    };
    let mut m = base(id, "text", Rect::default(), color, "transparent");
    m.insert("x".into(), json!(x));
    m.insert("y".into(), json!(y));
    m.insert("width".into(), json!(w));
    m.insert("height".into(), json!(h));
    m.insert("text".into(), json!(label));
    m.insert("originalText".into(), json!(label));
    m.insert("fontSize".into(), json!(FONT));
    m.insert("fontFamily".into(), json!(MONO));
    m.insert("textAlign".into(), json!(align));
    m.insert("verticalAlign".into(), json!(valign));
    m.insert("containerId".into(), container);
    m.insert("lineHeight".into(), json!(LINE));
    m.insert("autoResize".into(), json!(true));
    Value::Object(m)
}

enum TextAt<'a> {
    Free(f32, f32),
    In(&'a str, Rect, LabelAlign),
}

fn bound(kind: &str, id: &str) -> Value {
    json!({ "type": kind, "id": id })
}

/// The whole file: `{"type": "excalidraw", "elements": …}`.
pub fn export(doc: &Doc) -> Value {
    let layout = Layout::of(doc, &node_rect, &|t| {
        let (w, h) = text_px(&t.label);
        (w / UNIT, h / UNIT)
    });
    let mut elements: Vec<Value> = Vec::new();
    // Groups behind everything, as the canvas draws them.
    for e in &doc.entries {
        if let Item::Group(g) = &e.item
            && let Some(r) = group_rect(doc, &g.id, &|t| {
                let (w, h) = text_px(&t.label);
                (w / UNIT, h / UNIT)
            })
        {
            let (stroke, _) = colors(g.color.or(Some(Color::Gray)));
            let id = format!("group-{}", g.id);
            let mut m = base(&id, "rectangle", r, stroke, "transparent");
            m.insert("strokeStyle".into(), json!("dashed"));
            m.insert("strokeWidth".into(), json!(1));
            m.insert("roundness".into(), json!({ "type": 3 }));
            elements.push(Value::Object(m));
            if !g.label.is_empty() {
                elements.push(text(
                    &format!("{id}-label"),
                    &g.label,
                    stroke,
                    TextAt::Free(px(r.x + 0.5) as f32, px(r.y + 0.4) as f32),
                ));
            }
        }
    }
    // Each box's arrows, for its `boundElements`.
    let arrows_of = |id: &str| -> Vec<Value> {
        doc.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(&e.item, Item::Edge(x) if x.from == id || x.to == id))
            .map(|(i, _)| bound("arrow", &format!("arrow-{i}")))
            .collect()
    };
    for (i, e) in doc.entries.iter().enumerate() {
        match &e.item {
            Item::Node(n) => {
                let r = node_rect(n);
                let (stroke, fill) = colors(n.color);
                let id = format!("box-{}", n.id);
                let kind = match n.shape {
                    Shape::Rect | Shape::Round | Shape::Cylinder => "rectangle",
                    Shape::Ellipse | Shape::Dot | Shape::Final => "ellipse",
                    Shape::Diamond => "diamond",
                };
                let fill = if n.shape == Shape::Dot { stroke } else { fill };
                let mut m = base(&id, kind, r, stroke, fill);
                if matches!(n.shape, Shape::Round | Shape::Cylinder) {
                    m.insert("roundness".into(), json!({ "type": 3 }));
                }
                if n.dashed {
                    m.insert("strokeStyle".into(), json!("dashed"));
                }
                let mut bound_to = arrows_of(&n.id);
                let label_id = format!("{id}-label");
                if !n.label.is_empty() {
                    bound_to.push(bound("text", &label_id));
                }
                m.insert("boundElements".into(), Value::Array(bound_to));
                elements.push(Value::Object(m));
                if n.shape == Shape::Final {
                    let inner = Rect::new(r.x + r.w * 0.25, r.y + r.h * 0.25, r.w * 0.5, r.h * 0.5);
                    elements.push(Value::Object(base(
                        &format!("{id}-core"),
                        "ellipse",
                        inner,
                        stroke,
                        stroke,
                    )));
                }
                if !n.label.is_empty() {
                    // A dot's or a final's label beside it: they are too
                    // small to hold one.
                    let at = if matches!(n.shape, Shape::Dot | Shape::Final) {
                        TextAt::Free(px(r.right() + 0.5) as f32, px(r.y) as f32)
                    } else {
                        TextAt::In(&id, r, n.align)
                    };
                    let mut t = text(&label_id, &n.label, stroke, at);
                    if matches!(n.shape, Shape::Dot | Shape::Final) {
                        t["containerId"] = Value::Null;
                    }
                    elements.push(t);
                }
            }
            Item::Text(t) => {
                let (stroke, _) = colors(t.color);
                elements.push(text(
                    &format!("text-{i}"),
                    &t.label,
                    stroke,
                    TextAt::Free(px(t.x as f32) as f32, px(t.y as f32) as f32),
                ));
            }
            Item::Edge(edge) => {
                let Some(w) = &layout.wires[i] else {
                    continue;
                };
                let (stroke, _) = colors(edge.color);
                let id = format!("arrow-{i}");
                let first = w.points[0];
                let (x0, y0) = (
                    w.points.iter().map(|p| p.x).fold(f32::MAX, f32::min),
                    w.points.iter().map(|p| p.y).fold(f32::MAX, f32::min),
                );
                let (x1, y1) = (
                    w.points.iter().map(|p| p.x).fold(f32::MIN, f32::max),
                    w.points.iter().map(|p| p.y).fold(f32::MIN, f32::max),
                );
                let mut m = base(
                    &id,
                    "arrow",
                    Rect::new(first.x, first.y, x1 - x0, y1 - y0),
                    stroke,
                    "transparent",
                );
                let points: Vec<Value> = w
                    .points
                    .iter()
                    .map(|p| json!([px(p.x - first.x), px(p.y - first.y)]))
                    .collect();
                m.insert("points".into(), Value::Array(points));
                if edge.dashed {
                    m.insert("strokeStyle".into(), json!("dashed"));
                }
                let head = |on: bool| if on { json!("arrow") } else { Value::Null };
                m.insert("startArrowhead".into(), head(edge.heads.at_start()));
                m.insert("endArrowhead".into(), head(edge.heads.at_end()));
                let binding = |box_id: &str| json!({ "elementId": format!("box-{box_id}"), "focus": 0, "gap": 1 });
                m.insert("startBinding".into(), binding(&edge.from));
                m.insert("endBinding".into(), binding(&edge.to));
                m.insert("lastCommittedPoint".into(), Value::Null);
                m.insert("elbowed".into(), json!(false));
                let label_id = format!("{id}-label");
                if !edge.label.is_empty() {
                    m.insert("boundElements".into(), json!([bound("text", &label_id)]));
                }
                elements.push(Value::Object(m));
                if !edge.label.is_empty() {
                    let r = Rect::new(w.mid.x, w.mid.y, 0.0, 0.0);
                    elements.push(text(
                        &label_id,
                        &edge.label,
                        stroke,
                        TextAt::In(&id, r, LabelAlign::Center),
                    ));
                }
            }
            Item::Group(_) => {}
        }
    }
    json!({
        "type": "excalidraw",
        "version": 2,
        "source": "kawoosh",
        "elements": elements,
        "appState": { "gridSize": UNIT, "viewBackgroundColor": "#ffffff" },
        "files": {},
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    #[test]
    fn a_diagram_exports_bound_and_the_same_each_time() {
        let doc = parse(
            "box api \"API\" at 0,0 size 8x3 shape round color blue\nbox db at 12,0 shape cylinder\narrow api -> db \"reads\" dashed\nbox s at 0,6 shape final\ngroup g \"G\" { api db }\n",
        );
        let v = export(&doc);
        assert_eq!(v, export(&doc), "the same JSON twice");
        let els = v["elements"].as_array().unwrap();
        let by = |id: &str| {
            els.iter()
                .find(|e| e["id"] == id)
                .unwrap_or_else(|| panic!("{id}"))
        };
        let api = by("box-api");
        assert_eq!(api["type"], "rectangle");
        assert_eq!(
            (api["x"].as_f64(), api["width"].as_f64()),
            (Some(0.0), Some(160.0))
        );
        assert_eq!(api["strokeColor"], "#1971c2");
        assert_eq!(api["roughness"], 0);
        let bound: Vec<&str> = api["boundElements"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["id"].as_str().unwrap())
            .collect();
        assert_eq!(bound, vec!["arrow-2", "box-api-label"]);
        let label = by("box-api-label");
        assert_eq!(
            (label["containerId"].as_str(), label["text"].as_str()),
            (Some("box-api"), Some("API"))
        );
        let arrow = by("arrow-2");
        assert_eq!(arrow["startBinding"]["elementId"], "box-api");
        assert_eq!(arrow["endBinding"]["elementId"], "box-db");
        assert_eq!(arrow["strokeStyle"], "dashed");
        assert_eq!(arrow["points"][0], json!([0.0, 0.0]));
        assert_eq!(arrow["endArrowhead"], "arrow");
        assert_eq!(arrow["startArrowhead"], Value::Null);
        assert_eq!(by("arrow-2-label")["containerId"], "arrow-2");
        assert_eq!(by("box-s")["type"], "ellipse");
        assert_eq!(by("box-s-core")["backgroundColor"], "#1e1e1e");
        assert_eq!(by("group-g")["strokeStyle"], "dashed");
        assert_eq!(by("group-g-label")["text"], "G");
        // Groups first, behind.
        assert_eq!(els[0]["id"], "group-g");
    }
}
