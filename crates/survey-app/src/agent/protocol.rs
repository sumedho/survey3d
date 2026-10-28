//! The Model Context Protocol over stdio: newline-delimited JSON-RPC 2.0.
//! Only stdout carries protocol messages; diagnostics go to stderr.

use std::io::{BufRead, Write};

use serde_json::{Value, json};

use crate::agent::server::Server;
use crate::agent::tools;

/// Protocol revisions this server speaks, newest first.
const VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// What the server tells the agent about itself when it connects.
const INSTRUCTIONS: &str = "survey3d is surveying CAD (cadastral, construction, mining). \
This server holds one open project. Start with project_info. \
Look at things with query (points, polylines, layers, surfaces, records…) and render (a PNG of a plan sheet or of the whole drawing). \
Most work goes through run_commands, which types lines into the application's command line exactly as a user would: \
'help' lists the tools and commands; a tool such as 'pl' (polyline) then takes points as E,N[,Z], @dE,dN, @distance<bearing or a point name; \
'' (empty) is Enter, which finishes; tools have options such as 'c' to close. Coordinates, bearings and distances follow the project's conventions (see project_info). \
Commands such as delete, join, list, mkparcel act on the selection: set it with select. \
Lots and traverses from a plan: 'bd', a start point, then one 'bearing distance[ dZ][ code]' line per leg; each leg creates a point and moves on to it. \
A curved leg is typed as the plan gives it, with r= (radius) and l or r for the way it turns: by its chord ('45.3020 15.32 r=20 l', chord bearing and chord distance) or tangent to the leg before ('r=20 a=15.71 r', any two of r= a= arc c= chord d= delta t= tangent; tb= gives the starting tangent bearing); 'major' picks the major arc. \
'c' takes the last leg as returning to the start: it reports the misclosure (bearing, distance, 1:N precision), perimeter and area, unadjusted, and draws the closed boundary; 'j' instead joins back to the start with a computed closing leg (reported). \
After either, the next line names the lot to make it a parcel ('Lot 12', or 'y' for the next Lot number); '' skips. \
's' toggles sideshots (a point without moving on), 'u' undoes the last leg, '' ends an open run. \
Curve elements from any two: solve_curve (or the 'curve' command). Dimensions for plans: add_dimension (or 'dim'). \
Plan sheets (set_record kind sheet) list short or all parcel boundaries in line and curve tables, tagged L1, C1 on the drawing, with content.tables: short or all. \
A leg's direction can be an angle instead of a bearing: 'a92.1500 25.31' turned clockwise from the line arriving at the current point ('al' anticlockwise), 'dr12.3000 25.31' / 'dl' a deflection right or left of that line prolonged; before the first leg set the backsight with 'bs <point or bearing>'. \
Lines extended to where they meet (building corners, kerbs): 'ix' then each line as two points ('101 102'); trim_extend trims or extends polylines to boundary lines. \
Point tools on the selection: 'renumber 1001' (in name order; 'renumber +1000' shifts; 'renumber 1-50 1001' by range, no selection needed), 'raise 0.15' (or negative to lower heights), and 'scale' (base point, then a factor such as 0.99982 or measured and true lengths '100.012 100.000'); move and rotate take typed values too. \
Field side shots: 'rad', the station, a backsight point or bearing, then 'angle distance[ dZ]' (clockwise from the backsight); 't' toggles traverse mode, occupying each new point with the last as backsight. \
Raw total station data (import_file: Leica GSI, Trimble JobXML, Carlson RW5) is kept with the project: raw_data shows its setups and shots, 'reduce' orients each setup on its backsight and places its shots as coded points (checks to known points reported), then 'linework' strings them; correct a target height, code, backsight, instrument height or station, or exclude a shot, with raw_data action edit (the file is re-reduced in the same undoable step, and the audit trail records the correction). Control and traverses: adjust_traverse reduces and adjusts a traverse (Bowditch or transit; save it to keep it), adjust_network adjusts saved traverses and raw data together by least squares (σ₀, χ² test, standardised residuals to find blunders, error ellipses; apply to update the points), resection fixes a free station from known points, figure_closure checks a lot's closure or solves its two missing values. Stake-out files for controllers: export_file with jobxml (Trimble, points and lines), rw5 (Carlson), gsi (Leica) or csv, of the selection or every point. As-built checks: compare_asbuilt with the surveyed points against design points (by name, else nearest), design lines (offset, chainage) or a surface (cut/fill), with tolerances; label: true writes the differences on the drawing. GNSS data onto local control: site_calibration with pairs (measured GNSS point, control point or E,N[,Z]); it previews residuals and warnings first; drop doubtful pairs and refit, then apply (all_except_control or selection). \
Imagery: image add puts an orthophoto (GeoTIFF, or JPEG/PNG with a world file) under the drawing; an image without placement is fitted to control points (image control_points with at = where a feature is on the image now, world = where it belongs); render shows it. \
Roads: alignment (element table with spiral transitions, points at chainage and offset, chainage and offset of points, set-out points, the centreline drawn with its spirals); solve_curve also solves spiral, compound and reverse curves. Building outlines from taped right-angle walls: run_commands ['bldg', 'E,N', 'E,N', '6.2', '-4.1', 'c'] (two corners along the first wall, then each wall's length turning right, or left when negative; c closes and reports the misclosure). \
Coordinates and set-out: convert_coordinates (grid ↔ latitude/longitude with scale factors, convergence and the geoid separation), convert_heights (GNSS ellipsoidal heights to orthometric with the geoid model), setout (angles and distances from a station and backsight to points); set_project sets the geoid model (geoid, geoid_dir) and grid or ground distances. \
Quantities and design: compute_volumes (surface to surface or to a level, cut/fill), stockpile_volume (a pickup above its toe, with tonnes), sections (long and cross sections through surfaces along a line), subdivide (cut a given area off a parcel, parallel to an edge or swung about a point). \
Set up the job first: set_project (name, coordinate system), set_feature_codes (what point codes mean, their symbols, layers and linework) and set_layer (colours, line styles). \
Point codes are strung into lines with 'linework'; text such as street names goes in with add_text; fix names, codes and layers with edit_entities. \
Every edit is one undoable step ('undo', 'redo') recorded in the project's audit trail. \
Nothing is written to disk until save_project or export_file.";

/// Reads requests from `input` until it closes, answering on `output`.
pub fn serve(input: impl BufRead, mut output: impl Write, server: &mut Server) -> std::io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(Value::Array(batch)) => {
                let replies: Vec<Value> = batch.into_iter().filter_map(|m| handle(m, server)).collect();
                (!replies.is_empty()).then_some(Value::Array(replies))
            }
            Ok(message) => handle(message, server),
            Err(e) => Some(error(Value::Null, PARSE_ERROR, &format!("Parse error: {e}"))),
        };
        if let Some(reply) = reply {
            serde_json::to_writer(&mut output, &reply)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}

/// One message: the reply to a request, or `None` for a notification or response.
pub fn handle(message: Value, server: &mut Server) -> Option<Value> {
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        // A response to something we sent (we send nothing) or garbage.
        return message
            .get("id")
            .filter(|_| message.get("result").is_none() && message.get("error").is_none())
            .map(|id| error(id.clone(), INVALID_REQUEST, "Invalid request"));
    };
    // Notifications (no id) get no reply.
    let id = message.get("id")?.clone();
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => {
            let requested = params.get("protocolVersion").and_then(Value::as_str);
            let version = requested.filter(|v| VERSIONS.contains(v)).unwrap_or(VERSIONS[0]);
            result(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": {
                        "name": "survey3d",
                        "title": "survey3d",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "instructions": INSTRUCTIONS,
                }),
            )
        }
        "ping" => result(id, json!({})),
        "tools/list" => result(id, json!({ "tools": tools::definitions() })),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(error(id, INVALID_PARAMS, "tools/call needs a tool name"));
            };
            if !tools::exists(name) {
                return Some(error(id, INVALID_PARAMS, &format!("Unknown tool: {name}")));
            }
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            result(id, server.call(name, &args).to_json())
        }
        _ => error(id, METHOD_NOT_FOUND, &format!("Method not found: {method}")),
    })
}

fn result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exchange(server: &mut Server, requests: &[Value]) -> Vec<Value> {
        let input: String = requests.iter().map(|r| format!("{r}\n")).collect();
        let mut out = Vec::new();
        serve(input.as_bytes(), &mut out, server).expect("serve");
        String::from_utf8(out)
            .expect("utf8")
            .lines()
            .map(|l| serde_json::from_str(l).expect("json"))
            .collect()
    }

    #[test]
    fn handshake_list_and_errors() {
        let mut server = Server::new_for_tests();
        let replies = exchange(
            &mut server,
            &[
                json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}),
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
                json!({"jsonrpc":"2.0","id":3,"method":"nope"}),
                json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"nope"}}),
                json!({"jsonrpc":"2.0","id":5,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}),
            ],
        );
        assert_eq!(replies.len(), 5, "no reply to the notification");
        assert_eq!(replies[0]["result"]["protocolVersion"], "2025-06-18");
        assert!(replies[0]["result"]["instructions"].as_str().is_some());
        let tools = replies[1]["result"]["tools"].as_array().expect("tools");
        assert!(tools.iter().any(|t| t["name"] == "run_commands"));
        for t in tools {
            assert_eq!(t["inputSchema"]["type"], "object", "{}", t["name"]);
        }
        assert_eq!(replies[2]["error"]["code"], METHOD_NOT_FOUND);
        assert_eq!(replies[3]["error"]["code"], INVALID_PARAMS);
        assert_eq!(replies[4]["result"]["protocolVersion"], VERSIONS[0]);
    }

    #[test]
    fn malformed_input_gets_a_parse_error() {
        let mut server = Server::new_for_tests();
        let mut out = Vec::new();
        serve("{not json\n".as_bytes(), &mut out, &mut server).expect("serve");
        let reply: Value = serde_json::from_slice(&out).expect("json");
        assert_eq!(reply["error"]["code"], PARSE_ERROR);
    }
}
