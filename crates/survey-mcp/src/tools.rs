//! Tool names, descriptions and input schemas (what `tools/list` returns).

use serde_json::{Value, json};

/// Filter shared by `query` and `select`.
fn filter_schema() -> Value {
    json!({
        "type": "object",
        "description": "All given criteria must match.",
        "properties": {
            "ids": { "type": "array", "items": { "type": "integer" }, "description": "Entity ids." },
            "names": { "type": "array", "items": { "type": "string" }, "description": "Exact point or polyline names." },
            "code": { "type": "string", "description": "Feature code (case-insensitive)." },
            "layer": { "type": "string", "description": "Layer name (case-insensitive)." },
            "within": {
                "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4,
                "description": "Plan box [min E, min N, max E, max N] in metres; objects that touch it."
            }
        },
        "additionalProperties": false
    })
}

fn record_kinds() -> Value {
    json!(["sheet", "parcel", "alignment", "traverse"])
}

pub fn definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "project_info",
            "title": "Project summary",
            "description": "The open project: file, name, coordinate system, conventions (units, angle and bearing formats, axis names), counts, layers, surfaces, point clouds, records (sheets, parcels, alignments, traverses, field data), selection, active tool and its prompt, and what undo/redo would do. Call this first.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true }
        }),
        json!({
            "name": "run_commands",
            "title": "Type on the command line",
            "description": "Types each line into the command line, as a user would, and returns the command-line output, the active tool's prompt afterwards, and any reports produced. A line is a tool or command name ('help' lists them all), a point (E,N[,Z]; @dE,dN[,dZ] relative; @distance<bearing polar; or a point name), a value or option the prompt asks for (e.g. 'c' to close a polyline), or '' for Enter (finish). Examples: [\"pl\", \"1000,2000\", \"1010,2000\", \"@10<90\", \"c\"]; [\"inverse\", \"101\", \"102\"]; [\"undo\"]. Commands such as delete, join, explode, list, average, mkparcel and linework act on the selection (see select).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "lines": { "type": "array", "items": { "type": "string" }, "minItems": 1 }
                },
                "required": ["lines"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "query",
            "title": "Query the project",
            "description": "Lists objects as JSON. points: id, name, e, n, z (null when the height is unknown), code, description, layer. polylines: id, name, code, layer, closed, vertices [[e,n,z]], bulges (arcs, if any), plan length, area (closed). layers, surfaces, clouds: summaries. sheet/parcel/alignment/traverse/field_data: the full records with their ids (edit them with set_record). selection: the selected objects. audit: the most recent audit trail entries, newest first. Paged with offset/limit.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["points", "polylines", "layers", "surfaces", "clouds", "sheet", "parcel", "alignment", "traverse", "field_data", "selection", "audit"]
                    },
                    "filter": filter_schema(),
                    "offset": { "type": "integer", "minimum": 0, "default": 0 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 5000, "default": 200 }
                },
                "required": ["kind"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        }),
        json!({
            "name": "select",
            "title": "Set the selection",
            "description": "Changes the selection, which commands such as delete, join, list, average, mkparcel, mkalign and linework act on. Objects on hidden or locked layers are not selected.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "mode": { "type": "string", "enum": ["set", "add", "remove", "clear"], "default": "set" },
                    "kind": { "type": "string", "enum": ["points", "polylines", "all"], "default": "all" },
                    "filter": filter_schema()
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "add_points",
            "title": "Add points",
            "description": "Adds points in one undoable step. Unnamed points take the next free numbers. Omit z for a point whose height is unknown.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "points": {
                        "type": "array", "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string" },
                                "e": { "type": "number" }, "n": { "type": "number" }, "z": { "type": "number" },
                                "code": { "type": "string" }, "description": { "type": "string" }
                            },
                            "required": ["e", "n"],
                            "additionalProperties": false
                        }
                    },
                    "layer": { "type": "string", "description": "Layer name; created if missing. Default: the current layer." }
                },
                "required": ["points"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "add_polyline",
            "title": "Add a polyline",
            "description": "Adds a polyline through the given vertices, each [e, n] or [e, n, z] in metres or the name of an existing point. bulges (optional, one per segment) make arcs: tan(sweep/4), positive anticlockwise.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "vertices": {
                        "type": "array", "minItems": 2,
                        "items": { "oneOf": [
                            { "type": "string" },
                            { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 3 }
                        ] }
                    },
                    "closed": { "type": "boolean", "default": false },
                    "bulges": { "type": "array", "items": { "type": "number" } },
                    "name": { "type": "string" },
                    "code": { "type": "string" },
                    "layer": { "type": "string" }
                },
                "required": ["vertices"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "set_record",
            "title": "Create or edit a record",
            "description": "Creates a record (no id) or changes one (id given) — a plan sheet, parcel, alignment or saved traverse. When editing, only the fields given change (nested objects merge), e.g. {\"kind\":\"sheet\",\"id\":4,\"record\":{\"scale\":250,\"title\":{\"title\":\"Detail survey\"}}}. Read records with query to see their fields. One undoable step.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": record_kinds() },
                    "id": { "type": "integer" },
                    "record": { "type": "object" }
                },
                "required": ["kind", "record"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "delete_record",
            "title": "Delete a record",
            "description": "Removes a sheet, parcel, alignment, saved traverse or imported field data (the drawing objects stay). One undoable step.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": ["sheet", "parcel", "alignment", "traverse", "field_data"] },
                    "id": { "type": "integer" }
                },
                "required": ["kind", "id"],
                "additionalProperties": false
            },
            "annotations": { "destructiveHint": true }
        }),
        json!({
            "name": "import_file",
            "title": "Import a file",
            "description": "Reads a file into the project (one undoable step). Formats by extension unless given: csv/txt/pts points (column layout as letters, default PENZC: P name, E, N, Z height, C code, D description, - skip), dxf drawing, xml LandXML surfaces, gsi/jxl raw instrument data (Leica GSI, Trimble JobXML; kept for the network adjustment), las (uncompressed) or xyz point cloud.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "format": { "type": "string", "enum": ["csv", "dxf", "landxml", "raw", "cloud"] },
                    "layout": { "type": "string", "description": "CSV column letters, e.g. PNEZD." },
                    "layer": { "type": "string", "description": "CSV: layer for the points (created if missing)." }
                },
                "required": ["path"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "export_file",
            "title": "Export a file",
            "description": "Writes an export: dxf (drawing, symbols sized for a sheet's scale), landxml (surfaces), parcels (parcels as LandXML), gpkg (GeoPackage), csv (points; layout letters, default PENZCD), pdf or svg (a plan sheet at true scale; sheet_id, else an overview of the whole drawing). Refuses to replace an existing file unless overwrite is true.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "format": { "type": "string", "enum": ["dxf", "landxml", "parcels", "gpkg", "csv", "pdf", "svg"] },
                    "path": { "type": "string" },
                    "sheet_id": { "type": "integer" },
                    "layout": { "type": "string" },
                    "overwrite": { "type": "boolean", "default": false }
                },
                "required": ["format", "path"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "render",
            "title": "Render a plan",
            "description": "A PNG of a plan sheet as it would print (title block, grid, north arrow, scale bar, legend): the sheet with sheet_id, else a north-up A3 overview of the whole drawing. Use it to check work visually.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "sheet_id": { "type": "integer" },
                    "width": { "type": "integer", "minimum": 200, "maximum": 4000, "default": 1600, "description": "Image width in pixels." }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        }),
        json!({
            "name": "new_project",
            "title": "New project",
            "description": "Starts a new, unsaved project. Refuses when the open project has unsaved changes unless discard_changes is true.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "conventions": { "type": "string", "description": "Conventions preset name (see project_info for the choices); default: metric." },
                    "discard_changes": { "type": "boolean", "default": false }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "open_project",
            "title": "Open a project",
            "description": "Opens a .s3d project file. Refuses when the open project has unsaved changes unless discard_changes is true.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "discard_changes": { "type": "boolean", "default": false }
                },
                "required": ["path"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "save_project",
            "title": "Save the project",
            "description": "Saves the project (and its audit trail) to its file, or to path (save as). Refuses to replace a different existing file unless overwrite is true.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "overwrite": { "type": "boolean", "default": false }
                },
                "additionalProperties": false
            }
        }),
    ]
}

pub fn exists(name: &str) -> bool {
    definitions().iter().any(|d| d["name"] == name)
}
