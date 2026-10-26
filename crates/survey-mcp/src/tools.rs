//! Tool names, descriptions and input schemas (what `tools/list` returns).

use serde_json::{Value, json};
use survey_core::style::{LineStyle, Symbol};
use survey_model::TextAlign;

fn symbol_keys() -> Vec<&'static str> {
    Symbol::ALL.iter().map(|s| s.key()).collect()
}

fn line_style_keys() -> Vec<&'static str> {
    LineStyle::ALL.iter().map(|s| s.key()).collect()
}

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

fn surface_style_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "visible": { "type": "boolean" },
            "shaded": { "type": "boolean" },
            "elevation_colors": { "type": "boolean" },
            "triangles": { "type": "boolean" },
            "color": { "type": "string" },
            "isopach": { "type": "boolean" },
            "contours": {
                "type": "object",
                "properties": {
                    "show": { "type": "boolean" },
                    "interval": { "type": "number", "exclusiveMinimum": 0 },
                    "major_every": { "type": "integer", "minimum": 1 },
                    "labels": { "type": "boolean" }
                },
                "additionalProperties": false
            }
        },
        "additionalProperties": false
    })
}

/// Properties for creating a surface: its name, definition and style.
fn surface_properties() -> Value {
    json!({
        "name": { "type": "string" },
        "point_layers": { "type": "array", "items": { "type": "string" } },
        "point_codes": { "type": "array", "items": { "type": "string" } },
        "breakline_layers": { "type": "array", "items": { "type": "string" } },
        "breakline_codes": { "type": "array", "items": { "type": "string" } },
        "boundary": { "type": ["integer", "string"] },
        "holes": { "type": "array", "items": { "type": ["integer", "string"] } },
        "max_edge": { "type": "number", "exclusiveMinimum": 0 },
        "style": surface_style_schema()
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
            "description": "Types each line into the command line, as a user would, and returns the command-line output, the active tool's prompt afterwards, and any reports produced. A line is a tool or command name ('help' lists them all), a point (E,N[,Z]; @dE,dN[,dZ] relative; @distance<bearing polar; or a point name), a value or option the prompt asks for (e.g. 'c' to close a polyline), or '' for Enter (finish). Examples: [\"pl\", \"1000,2000\", \"1010,2000\", \"@10<90\", \"c\"]; a lot by bearing & distance, closed with its misclosure reported and made a parcel: [\"bd\", \"1000,2000\", \"0 25.00\", \"90 40.00\", \"180 25.00\", \"270 40.00\", \"c\", \"Lot 1\"]; a curved leg tangent to the last, radius 10 turning right through 90°: \"r=10 d=90 r\"; 'j' closes with a computed closing leg; [\"inverse\", \"101\", \"102\"]; [\"undo\"]. Commands such as delete, join, explode, list, average, mkparcel and linework act on the selection (see select).",
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
            "description": "Lists objects as JSON. points: id, name, e, n, z (null when the height is unknown), code, description, layer. polylines: id, name, code, layer, closed, vertices ([e,n,z], or [e,n] for a plan-only line without heights), bulges (arcs, if any), plan length, area (closed). texts: id, text, e, n, height_mm, rotation, align, layer. dimensions: id, kind, from, to, line (the dimension line's ends), length, shows (the text drawn), text (its own text), layer. feature_codes: the code library (filter.names picks codes). layers, surfaces, clouds: summaries. sheet/parcel/alignment/traverse/field_data: the full records with their ids (edit them with set_record). selection: the selected objects. audit: the most recent audit trail entries, newest first. Paged with offset/limit.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["points", "polylines", "texts", "dimensions", "layers", "feature_codes", "surfaces", "clouds", "sheet", "parcel", "alignment", "traverse", "field_data", "selection", "audit"]
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
                    "kind": { "type": "string", "enum": ["points", "polylines", "texts", "dimensions", "all"], "default": "all" },
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
                    "layer": { "type": "string", "description": "Layer name; created if missing. Default: the layer of each point's feature code (created in the code's colour), else the current layer." }
                },
                "required": ["points"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "add_polyline",
            "title": "Add a polyline",
            "description": "Adds a polyline through the given vertices, each [e, n, z] or [e, n] in metres or the name of an existing point. Give heights for every vertex, or for none: a line without heights is plan-only (heights unknown; left out of surfaces). bulges (optional, one per segment) make arcs: tan(sweep/4), positive anticlockwise.",
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
            "description": "Creates a record (no id) or changes one (id given) — a plan sheet, parcel, alignment or saved traverse. When editing, only the fields given change (nested objects merge), e.g. {\"kind\":\"sheet\",\"id\":4,\"record\":{\"scale\":250,\"title\":{\"title\":\"Detail survey\"}}}. For several records at once pass items: [{\"id\":7,\"record\":{\"plan\":\"DP 1234\"}}, …] instead of id/record. Read records with query to see their fields. A sheet's content.legend_position is auto (the corner with least drawing under it), top_left, top_right, bottom_left or bottom_right. One undoable step.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": record_kinds() },
                    "id": { "type": "integer" },
                    "record": { "type": "object" },
                    "items": {
                        "type": "array", "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": { "id": { "type": "integer" }, "record": { "type": "object" } },
                            "required": ["record"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["kind"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "edit_entities",
            "title": "Edit objects",
            "description": "Changes fields of the points, polylines, texts and dimensions the filter matches, in one undoable step (a filter is required). Fields: name, code (points, polylines); description (points); closed (polylines); layer (any; created if missing); text, height_mm, rotation, align (texts); text (dimensions: shown instead of the length, with <> standing for it; empty shows the length), kind (dimensions); e, n (one point or text) and z (one point; null makes its height unknown). To move or rotate several objects use run_commands (move, rotate).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "filter": filter_schema(),
                    "kind": { "type": "string", "enum": ["points", "polylines", "texts", "dimensions", "all"], "default": "all" },
                    "set": { "type": "object", "minProperties": 1 }
                },
                "required": ["filter", "set"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "add_text",
            "title": "Add text",
            "description": "Adds text to the drawing — street names, easement labels, notes — in one undoable step. at: [e, n] or a point name, the middle of the text's height at its left, centre or right end (align). height_mm is the height of capitals on a printed plan (default 2.5), so text prints the same size at any scale; rotation is the reading direction in degrees anticlockwise from east (0 reads left to right; for a street, the street's direction). Lines can be split with \\n. Text takes its layer's colour.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "texts": {
                        "type": "array", "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "text": { "type": "string" },
                                "at": { "oneOf": [
                                    { "type": "string" },
                                    { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 3 }
                                ] },
                                "height_mm": { "type": "number", "minimum": 0.5, "maximum": 50 },
                                "rotation": { "type": "number" },
                                "align": { "type": "string", "enum": TextAlign::ALL.iter().map(|a| a.key()).collect::<Vec<_>>() }
                            },
                            "required": ["text", "at"],
                            "additionalProperties": false
                        }
                    },
                    "layer": { "type": "string", "description": "Layer name; created if missing. Default: the current layer." }
                },
                "required": ["texts"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "add_dimension",
            "title": "Add dimensions",
            "description": "Adds dimensions for plan sheets in one undoable step: the length between two points drawn as a dimension line with arrows and extension lines, its text at the sheet's text size. from, to: [e, n] or point names. kind: aligned (the plan distance, default), horizontal (difference in eastings) or vertical (difference in northings). Place the line with offset (metres; positive to the left of from → to for aligned, away from 'to' for horizontal and vertical) or line_at (a point it passes through). text replaces the length shown, with <> standing for it (e.g. '<> (fence)'). The same is the 'dim' command.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "dimensions": {
                        "type": "array", "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "from": { "oneOf": [
                                    { "type": "string" },
                                    { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 3 }
                                ] },
                                "to": { "oneOf": [
                                    { "type": "string" },
                                    { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 3 }
                                ] },
                                "kind": { "type": "string", "enum": ["aligned", "horizontal", "vertical"], "default": "aligned" },
                                "offset": { "type": "number" },
                                "line_at": { "oneOf": [
                                    { "type": "string" },
                                    { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 3 }
                                ] },
                                "text": { "type": "string" }
                            },
                            "required": ["from", "to"],
                            "additionalProperties": false
                        }
                    },
                    "layer": { "type": "string", "description": "Layer name; created if missing. Default: the current layer." }
                },
                "required": ["dimensions"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "set_feature_codes",
            "title": "Edit the feature codes",
            "description": format!(
                "Edits the feature-code library (one undoable step): what a point's code means, which layer its points and lines belong to, how they are drawn, and whether points with the code are strung into lines by the 'linework' command. A point's code is matched by its first word; a code may be a pattern with * and ?. codes: entries to add or change, matched by code (an existing code keeps the fields not given; a new one defaults to its own layer, no linework). remove: codes to delete. replace: start from an empty library; preset: 'example' starts from a small generic set. Style fields: symbol ({}), symbol_size (screen points, 7 normal), line_style ({}), line_weight (screen points, 1.5 normal), by_layer (draw in the layer's colour once that layer exists), labels ({{name, code, height, description}} booleans, or null for the global setting). color is #rrggbb. Read the current library with query kind feature_codes. Linework control codes follow the code on a point, as Carlson's special codes do; set their spellings with controls (each field one or more words, space separated; empty turns it off): start (ST +7), end (END -7), close (CLO CL), arc_start (PC) and arc_end (PT; a PC without one makes a three-point arc), gap (GAP: no line on to the next point), rectangle (RECT closes square from three points; RECT10 builds a 10 m deep rectangle right of the last two points, RECT-10 left), circle (CIR2: radius 2 about the point, any code; CIR alone takes the radius from the next point with the code), offset (OH0.5: a copy 0.5 right), offset_left (OFL1.2), offset_vertical (OV-0.15: the copy that much higher), join (JPN101: join to point 101). E.g. 'KB1 ST', 'BLD RECT', 'KB OH0.5 OV-0.15'.",
                symbol_keys().join(", "),
                line_style_keys().join(", ")
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "codes": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "code": { "type": "string" },
                                "description": { "type": "string" },
                                "layer": { "type": "string" },
                                "color": { "type": "string" },
                                "linework": { "type": "boolean" },
                                "style": {
                                    "type": "object",
                                    "properties": {
                                        "symbol": { "type": "string", "enum": symbol_keys() },
                                        "symbol_size": { "type": "number" },
                                        "by_layer": { "type": "boolean" },
                                        "line_style": { "type": "string", "enum": line_style_keys() },
                                        "line_weight": { "type": "number" },
                                        "labels": { "type": ["object", "null"] }
                                    }
                                }
                            },
                            "required": ["code"]
                        }
                    },
                    "remove": { "type": "array", "items": { "type": "string" } },
                    "replace": { "type": "boolean", "default": false },
                    "preset": { "type": "string", "enum": ["example"] },
                    "controls": { "type": "object" }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "set_layer",
            "title": "Create or change a layer",
            "description": "Creates a layer (if no layer has this name) or changes one: rename, color (#rrggbb), visible, locked, line_style and line_weight (for lines on it whose feature code has no style of its own), current (where new objects go). New layers otherwise get the next colour of a palette. Name, colour and line style are one undoable step; a rename also renames the layer in feature codes and surface definitions. visible and locked are display settings (not undone by undo, as in the app); objects on a locked layer can't be selected or edited until it is unlocked.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "layer": { "type": "string" },
                    "rename": { "type": "string" },
                    "color": { "type": "string" },
                    "visible": { "type": "boolean" },
                    "locked": { "type": "boolean" },
                    "line_style": { "type": "string", "enum": line_style_keys() },
                    "line_weight": { "type": "number", "minimum": 0.1, "maximum": 20 },
                    "current": { "type": "boolean" }
                },
                "required": ["layer"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "set_project",
            "title": "Project settings",
            "description": "Sets the project's name, description, conventions preset and coordinate system (shown on plan sheets and used for grid/geographic conversions). coordinate_system: {\"name\": \"MGA2020 zone 50\"} is enough for MGA (zone, south, GRS80 and EPSG 7850 follow); likewise \"MGA94 zone 56\" or \"UTM zone 33N\" (WGS84). Or give type (projected, local, unspecified), zone, hemisphere, ellipsoid (GRS80, WGS84), epsg, geoid_separation (metres), and for a local grid a description. One undoable step.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "description": { "type": "string" },
                    "conventions": { "type": "string" },
                    "coordinate_system": {
                        "type": "object",
                        "properties": {
                            "type": { "type": "string", "enum": ["projected", "local", "unspecified"] },
                            "name": { "type": "string" },
                            "zone": { "type": "integer", "minimum": 1, "maximum": 60 },
                            "hemisphere": { "type": "string", "enum": ["north", "south"] },
                            "ellipsoid": { "type": "string", "enum": ["GRS80", "WGS84"] },
                            "epsg": { "type": "integer" },
                            "geoid_separation": { "type": "number" },
                            "description": { "type": "string" }
                        },
                        "additionalProperties": false
                    }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "site_calibration",
            "title": "Site calibration",
            "description": "Site calibration (localisation): fits GNSS-measured points (projected grid, e.g. MGA) to the local control the site is built on, then moves the measured data onto it. Horizontal: least-squares shifts, rotation and scale (fixed_scale: true holds scale at 1); vertical: none, constant shift or an inclined plane (needs 3+ points with heights). pairs: each measured (GNSS point name) and control (control point name, or its coordinates 'E,N[,Z]' in the project's order); horizontal/vertical (default true) include it in each fit. Omit pairs to reuse the project's saved ones. Without apply (or apply: none) this only previews: per-pair residuals (control minus calibrated, metres) flagged over_limit against tolerance_h/tolerance_v (default 0.02/0.03 m), the scale (ppm), rotation, shifts, plane slopes, redundancy, points outside the control area, and plain-language warnings. Good practice: 3 or more horizontal pairs (4+ better) spread around and enclosing the work, 4+ vertical points; a scale far from 1 or a steep tilt usually means a mismatched pair; turn off a pair with a large residual (horizontal/vertical: false) and refit, rather than accepting it. Then apply: all_except_control (everything except the control points) or selection (the selected objects, see select) moves them in one undoable step and saves the calibration with the project. save: true stores the pairs without applying. In the app the same is the 'sitecal' window (Survey > Site calibration).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "pairs": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "measured": { "type": "string", "description": "Name of the GNSS-measured point." },
                                "control": { "type": "string", "description": "Control point name, or 'E,N[,Z]'." },
                                "horizontal": { "type": "boolean", "default": true },
                                "vertical": { "type": "boolean", "default": true }
                            },
                            "required": ["measured", "control"],
                            "additionalProperties": false
                        }
                    },
                    "fixed_scale": { "type": "boolean", "default": false },
                    "vertical": { "type": "string", "enum": ["none", "constant", "plane"], "default": "plane" },
                    "tolerance_h": { "type": "number", "exclusiveMinimum": 0, "default": 0.02 },
                    "tolerance_v": { "type": "number", "exclusiveMinimum": 0, "default": 0.03 },
                    "apply": { "type": "string", "enum": ["none", "all_except_control", "selection"], "default": "none" },
                    "save": { "type": "boolean", "default": false }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "solve_curve",
            "title": "Solve a circular curve",
            "description": "Completes a circular curve from any two of its elements: radius, delta (central angle: degrees as a number, or text in the project's angle format), arc (length), chord, tangent, external, mid_ordinate. Lengths are in the project's units (numbers, or text with a unit suffix). Some pairs (radius and chord, for example) have two solutions, the minor and major arc; both are returned. Nothing is drawn. To draw a curved boundary leg, use bd with a curve line (see run_commands). The same is the 'curve' command (r=20 a=15.71).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "radius": { "type": ["number", "string"] },
                    "delta": { "type": ["number", "string"] },
                    "arc": { "type": ["number", "string"] },
                    "chord": { "type": ["number", "string"] },
                    "tangent": { "type": ["number", "string"] },
                    "external": { "type": ["number", "string"] },
                    "mid_ordinate": { "type": ["number", "string"] }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "raw_data",
            "title": "Raw field data",
            "description": "Works with raw total station data imported with import_file (one record per file; file is its name, optional when there is one). Actions: show — the file's setups (station, instrument height, backsight, shot count, corrections); with setup (from 1), that setup's shots (target, code, target height, circle readings, distance, excluded). reduce — orients each setup on its backsight (else the first shot to a point with coordinates) and radiates its shots into points carrying their codes, setup by setup, so a traverse carries forward; both faces and repeated shots are averaged; shots to points that already have coordinates are checks (returned). Reducing again updates the points it made. edit — corrects a setup (setup with station, instrument_height, backsight; empty backsight orients on the first known point) or a shot (setup and shot with target, code, target_height, excluded); the recorded observations are kept, and a reduced file is re-reduced in the same undoable step, recorded in the audit trail. revert — drops the corrections to that setup or shot. report — the reduction report as text. After reducing, run_commands 'linework' strings the coded points into lines.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["show", "reduce", "edit", "revert", "report"], "default": "show" },
                    "file": { "type": "string" },
                    "setup": { "type": "integer", "minimum": 1 },
                    "shot": { "type": "integer", "minimum": 1 },
                    "station": { "type": "string" },
                    "instrument_height": { "type": "number" },
                    "backsight": { "type": "string" },
                    "target": { "type": "string" },
                    "code": { "type": "string" },
                    "target_height": { "type": "number" },
                    "excluded": { "type": "boolean" }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "compare_asbuilt",
            "title": "As-built comparison",
            "description": "Compares surveyed points (shots: point names or ids; default the selected points) with the design and reports each difference against tolerances (metres; default 0.02 plan and height). against: points — the design point of the same name, else after removing strip from the start or end of the shot's name (e.g. 'S' for 101S), else the nearest within radius; gives dE, dN, plan miss, dZ. lines — the nearest of the given polylines (lines: ids or names); gives chainage, offset (positive right of the line's direction) and dZ to the line's height. surface — the surface (id or name); gives dZ (shot minus surface: positive means the shot is above design). report: true adds a text report; label: true writes the differences beside each shot on layer (default AS-BUILT) in one undoable step.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "shots": { "type": "array", "items": { "type": ["string", "integer"] } },
                    "against": { "type": "string", "enum": ["points", "lines", "surface"], "default": "points" },
                    "strip": { "type": "string" },
                    "radius": { "type": "number", "exclusiveMinimum": 0 },
                    "lines": { "type": "array", "items": { "type": ["string", "integer"] } },
                    "surface": { "type": ["string", "integer"] },
                    "tolerance_h": { "type": "number", "minimum": 0, "default": 0.02 },
                    "tolerance_v": { "type": "number", "minimum": 0, "default": 0.02 },
                    "report": { "type": "boolean", "default": false },
                    "label": { "type": "boolean", "default": false },
                    "layer": { "type": "string" }
                },
                "additionalProperties": false
            },
            "annotations": { "destructiveHint": false }
        }),
        json!({
            "name": "create_surface",
            "title": "Create a surface",
            "description": "Builds a surface (TIN) from the drawing and makes it the active surface, in one undoable step. Points: those with heights on point_layers and/or matching point_codes (patterns with * and ?; both empty: every point with a height). Breaklines: polylines on breakline_layers or matching breakline_codes (plan-only lines are left out). boundary: a closed polyline (id or name) the surface stops at; holes: closed polylines cut out; max_edge: drop longer triangle edges (metres) around an irregular edge. style: visible, shaded, elevation_colors, triangles, color (#rrggbb), isopach, and contours {show, interval (m), major_every, labels} — contours show on the drawing and plan sheets; turn them into polylines with run_commands 'contours'. The result lists points and triangles used, the height range, and warnings for selected points or breaklines without heights.",
            "inputSchema": {
                "type": "object",
                "properties": surface_properties(),
                "required": ["name"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "set_surface",
            "title": "Change a surface",
            "description": "Changes a surface (by id or name) in one undoable step: rename; any definition field (as in create_surface; the surface is rebuilt); style fields (merged, e.g. {\"contours\": {\"show\": true, \"interval\": 0.5}}); rebuild: true rebuilds it from the drawing as it is now (after points or breaklines changed).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "surface": { "type": ["integer", "string"] },
                    "rename": { "type": "string" },
                    "rebuild": { "type": "boolean", "default": false },
                    "point_layers": { "type": "array", "items": { "type": "string" } },
                    "point_codes": { "type": "array", "items": { "type": "string" } },
                    "breakline_layers": { "type": "array", "items": { "type": "string" } },
                    "breakline_codes": { "type": "array", "items": { "type": "string" } },
                    "boundary": { "type": ["integer", "string", "null"] },
                    "holes": { "type": "array", "items": { "type": ["integer", "string"] } },
                    "max_edge": { "type": ["number", "null"] },
                    "style": surface_style_schema()
                },
                "required": ["surface"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "delete_surface",
            "title": "Delete a surface",
            "description": "Deletes a surface (by id or name). One undoable step; the points and lines it was built from stay.",
            "inputSchema": {
                "type": "object",
                "properties": { "surface": { "type": ["integer", "string"] } },
                "required": ["surface"],
                "additionalProperties": false
            },
            "annotations": { "destructiveHint": true }
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
            "description": "Reads a file into the project (one undoable step). Formats by extension unless given: csv/txt/pts points (column layout as letters, default PENZC: P name, E, N, Z height, C code, D description, - skip), dxf drawing, xml LandXML surfaces, gsi/jxl/rw5 raw instrument data (Leica GSI, Trimble JobXML, Carlson RW5; kept with the project for raw_data reduce and the network adjustment), las (uncompressed) or xyz point cloud.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "format": { "type": "string", "enum": ["csv", "dxf", "landxml", "raw", "cloud"] },
                    "layout": { "type": "string", "description": "CSV column letters, e.g. PNEZD." },
                    "layer": { "type": "string", "description": "CSV: one layer for all the points (created if missing). Default: each point's feature-code layer, else the current layer." }
                },
                "required": ["path"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "export_file",
            "title": "Export a file",
            "description": "Writes an export: dxf (drawing, symbols sized for a sheet's scale), landxml (surfaces), parcels (parcels as LandXML), gpkg (GeoPackage), csv (points; layout letters, default PENZCD), pdf or svg (a plan sheet at true scale; sheet_id, else an overview of the whole drawing). Stake-out files for controllers: jobxml (Trimble Access: points, and the selected polylines as lines), rw5 (Carlson SurvCE/SurvPC stored points), gsi (Leica GSI-16 coordinates; names and codes up to 16 characters) or csv with selection: true (name, N, E, Z, code). They take the selected points and the vertices of the selected polylines (selection: true), else every point. Refuses to replace an existing file unless overwrite is true.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "format": { "type": "string", "enum": ["dxf", "landxml", "parcels", "gpkg", "csv", "pdf", "svg", "jobxml", "rw5", "gsi"] },
                    "selection": { "type": "boolean", "default": false, "description": "Stake-out formats: export the selection (points, and polylines' vertices) instead of every point." },
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
