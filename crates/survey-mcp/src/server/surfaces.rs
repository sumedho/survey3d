//! Surfaces (TINs) built from the drawing: create, change, rebuild, delete.

use serde_json::{Value, json};
use survey_model::{Document, Entity, EntityId, SurfaceDefinition, SurfaceId, SurfaceStyle};

use super::settings::parse_color;
use super::{Outcome, Server, ToolResult, flag, merge, required_str, str_arg};

impl Server {
    pub(super) fn create_surface(&mut self, args: &Value) -> Outcome {
        let name = required_str(args, "name")?.to_string();
        let definition = definition(self.doc(), args, SurfaceDefinition::default())?;
        let style = style(args, SurfaceStyle::default())?;
        let id = self.session.build_surface(&name, definition, style)?;
        Ok(ToolResult::json(&self.surface_summary(id)))
    }

    pub(super) fn set_surface(&mut self, args: &Value) -> Outcome {
        let id = self.surface_id(args)?;
        let surface = self.doc().surface(id).ok_or("The surface has gone")?;
        let fields = [
            "point_layers",
            "point_codes",
            "breakline_layers",
            "breakline_codes",
            "boundary",
            "holes",
            "max_edge",
        ];
        let definition = if fields.iter().any(|f| args.get(f).is_some()) {
            let current = surface.definition.clone().ok_or(
                "This surface was imported, not built from the drawing, so it has no definition to change",
            )?;
            Some(definition(self.doc(), args, current)?)
        } else {
            None
        };
        let style = match args.get("style") {
            Some(_) => Some(style(args, surface.style)?),
            None => None,
        };
        let name = str_arg(args, "rename").map(str::to_string);
        self.session
            .change_surface(id, name, definition, style, flag(args, "rebuild"))?;
        Ok(ToolResult::json(&self.surface_summary(id)))
    }

    pub(super) fn delete_surface(&mut self, args: &Value) -> Outcome {
        let id = self.surface_id(args)?;
        let name = self.doc().surface(id).map(|s| s.name.clone()).unwrap_or_default();
        self.session.delete_surface(id);
        Ok(ToolResult::text(format!("Deleted surface {name}")))
    }

    /// The surface named by `surface`: its id or its name.
    pub(super) fn surface_id(&self, args: &Value) -> Result<SurfaceId, String> {
        surface_arg(self.doc(), args.get("surface"), "surface")
    }

    fn surface_summary(&self, id: SurfaceId) -> Value {
        let doc = self.doc();
        let Some(s) = doc.surface(id) else {
            return json!({});
        };
        let warnings = s.definition.as_ref().map(|d| d.warnings(doc)).unwrap_or_default();
        let z = s.tin.vertices.iter().map(|v| v.z);
        let (low, high) = z.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), z| {
            (lo.min(z), hi.max(z))
        });
        json!({
            "id": id.0,
            "name": s.name,
            "points": s.tin.vertices.len(),
            "triangles": s.tin.triangles.len(),
            "heights": (!s.tin.vertices.is_empty()).then_some([low, high]),
            "definition": s.definition,
            "style": style_json(&s.style),
            "warnings": warnings,
        })
    }
}

/// `current` with the definition fields given in `args`. Boundary and holes
/// are closed polylines, by id or name (`null` clears the boundary).
fn definition(doc: &Document, args: &Value, mut def: SurfaceDefinition) -> Result<SurfaceDefinition, String> {
    let strings = |key: &str| -> Result<Option<Vec<String>>, String> {
        match args.get(key) {
            None => Ok(None),
            Some(Value::Array(a)) => a
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| format!("{key}: strings"))
                })
                .collect::<Result<_, _>>()
                .map(Some),
            Some(_) => Err(format!("{key}: a list of strings")),
        }
    };
    if let Some(v) = strings("point_layers")? {
        def.point_layers = v;
    }
    if let Some(v) = strings("point_codes")? {
        def.point_codes = v;
    }
    if let Some(v) = strings("breakline_layers")? {
        def.breakline_layers = v;
    }
    if let Some(v) = strings("breakline_codes")? {
        def.breakline_codes = v;
    }
    match args.get("boundary") {
        None => {}
        Some(Value::Null) => def.boundary = None,
        Some(v) => def.boundary = Some(closed_polyline(doc, v, "boundary")?),
    }
    if let Some(holes) = args.get("holes") {
        def.holes = holes
            .as_array()
            .ok_or("holes: a list of polylines")?
            .iter()
            .map(|h| closed_polyline(doc, h, "holes"))
            .collect::<Result<_, _>>()?;
    }
    match args.get("max_edge") {
        None => {}
        Some(Value::Null) => def.max_edge = None,
        Some(v) => {
            let edge = v
                .as_f64()
                .filter(|e| *e > 0.0)
                .ok_or("max_edge: metres, more than 0")?;
            def.max_edge = Some(edge);
        }
    }
    Ok(def)
}

/// A closed polyline by id or name.
fn closed_polyline(doc: &Document, v: &Value, key: &str) -> Result<EntityId, String> {
    let found = match v {
        Value::Number(n) => n
            .as_i64()
            .map(EntityId)
            .and_then(|id| Some((id, doc.entity(id)?))),
        Value::String(name) => doc
            .entities()
            .find(|(_, e)| matches!(e, Entity::Polyline(l) if l.name == *name)),
        _ => return Err(format!("{key}: a polyline id or name")),
    };
    match found {
        Some((id, Entity::Polyline(l))) if l.closed => Ok(id),
        Some((id, Entity::Polyline(_))) => Err(format!("{key}: polyline {} is not closed", id.0)),
        _ => Err(format!("{key}: no polyline {v}")),
    }
}

/// `current` with the `style` patch in `args` merged in (colour as #rrggbb).
fn style(args: &Value, current: SurfaceStyle) -> Result<SurfaceStyle, String> {
    let Some(patch) = args.get("style") else {
        return Ok(current);
    };
    let mut patch = patch.clone();
    if let Some(color) = patch.get("color").and_then(Value::as_str) {
        let c = parse_color(color).map_err(|e| format!("style.{e}"))?;
        patch["color"] = json!(c.0);
    }
    let mut value = serde_json::to_value(current).map_err(|e| e.to_string())?;
    merge(&mut value, patch);
    let style: SurfaceStyle = serde_json::from_value(value).map_err(|e| format!("style: {e}"))?;
    if style.contours.interval <= 0.0 {
        return Err("style.contours.interval: metres, more than 0".into());
    }
    Ok(style)
}

/// A style for reading: the colour as #rrggbb.
pub(super) fn style_json(style: &SurfaceStyle) -> Value {
    let mut v = serde_json::to_value(style).unwrap_or_default();
    let [r, g, b, _] = style.color.0;
    v["color"] = json!(format!("#{r:02x}{g:02x}{b:02x}"));
    v
}

/// A surface by id or name.
pub(super) fn surface_arg(doc: &Document, v: Option<&Value>, key: &str) -> Result<SurfaceId, String> {
    match v {
        Some(Value::Number(n)) => n
            .as_i64()
            .map(SurfaceId)
            .filter(|id| doc.surface(*id).is_some())
            .ok_or_else(|| format!("{key}: no surface with id {n}")),
        Some(Value::String(name)) => doc
            .surface_by_name(name)
            .map(|(id, _)| id)
            .ok_or_else(|| format!("{key}: no surface called {name}")),
        _ => Err(format!("{key}: a surface's id or name")),
    }
}
