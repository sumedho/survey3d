# survey3d

Surveying CAD for cadastral, construction and mining work — Rust, [egui](https://github.com/emilk/egui)
and [wgpu](https://wgpu.rs), running natively and in the browser (WebGPU with WebGL2 fallback).

Architecture, decisions and completed phases: [`docs/PLAN.md`](docs/PLAN.md); what comes next:
[`docs/ROADMAP.md`](docs/ROADMAP.md).

## Run

```sh
cargo run                    # native (survey-app is the default member)
cargo run -- job.s3d         # native, opening a project
./scripts/serve-web.sh       # web, http://127.0.0.1:8765 (needs `cargo install trunk --locked --version 0.21.14`)
cargo test --workspace
```

The web build compiles SQLite to wasm with clang. On macOS install an LLVM with the wasm32 target
(`brew install llvm`); the script picks it up automatically.

## Releases

Downloads are on the [Releases](https://github.com/sumedho/survey3d/releases) page:

| Platform | Installer | Plain archive (both programs) |
|---|---|---|
| macOS (Apple silicon, Intel) | `.dmg` | `.tar.gz` |
| Windows | `-setup.exe` (per-user, no admin) | `.zip` |
| Linux x86_64 | `.AppImage`, `.deb` | `.tar.gz` |

Each includes `survey3d-mcp` beside `survey3d`. The builds are not code-signed yet: on macOS, after
the first launch is blocked, allow it in System Settings → Privacy & Security → Open Anyway; on
Windows, SmartScreen's "unknown publisher" warning needs More info → Run anyway. With the AppImage,
point MCP clients at the `survey3d-mcp` from the `.tar.gz` (the AppImage's own copy moves each run).

To cut a release, bump `version` in the workspace `Cargo.toml`, commit, then tag and push:
`git tag v0.2.0 && git push origin v0.2.0`. `.github/workflows/release.yml` builds every platform and
opens a draft release with the files and their SHA-256 sums; check it and publish. Running the
workflow by hand from the Actions tab builds the same files as artifacts, without a release. Locally,
`cargo install cargo-packager --locked`, then `cargo build --release -p survey-app -p survey-mcp &&
cargo packager --release` (packaging settings are in `crates/survey-app/Cargo.toml`).

## Workspace

| Crate | Purpose |
|---|---|
| `survey-core` | Pure maths: angles/bearings, conventions (parse/format), geometry, arcs (bulges), offsets, COGO (intersections, chainage/offset), traverse adjustment, least-squares networks, map projections, TINs and volumes, field-to-finish, point input |
| `survey-model` | `Document`, entities, layers, project records (traverses, parcels, alignments, field data, sheets) with stable ids, `Command`/`ChangeSet`, undo `History` with audit records, R-tree index |
| `survey-io` | Interchange formats: CSV/TXT points (configurable columns), DXF read/write (symbol blocks, line types), feature code sheets, LandXML, LAS/XYZ point clouds, Leica GSI and Trimble JobXML raw data; PDF and SVG plot writers |
| `survey-store` | `.s3d` project files (SQLite): incremental saves, migrations, hash-chained append-only audit trail; GeoPackage export |
| `cad-render` | wgpu renderer: f64 camera, relative-to-center chunks, reverse-Z, offscreen MSAA, egui paint callback |
| `egui-cad` | Reusable CAD widgets: theme, viewport, snapping, box selection, label placement, command line, panels |
| `survey-app` | The application. `Session` (library): project, tools, command line and edits with no UI — drive it with `Session::run("pl")`. `SurveyApp`: the egui interface over a session (panels, dialogs, file handling) |
| `survey-mcp` | `survey3d-mcp`: a Model Context Protocol server so an agent can open, edit, query and render projects |

## Using it

* **Navigate:** wheel/pinch zooms about the cursor, middle-drag (or Shift+right-drag) pans, right-drag orbits in 3D.
  The toolbar cube (or `plan` / `3d`) toggles the plan lock; returning to plan is always north-up.
  `persp` (or View menu) toggles perspective.
* **Commands:** press **Ctrl/Cmd+K** to search every command and tool, or start typing anywhere —
  `point`, `pl` (polyline), `arc`, `circle`, `inv` (inverse), `ze`, `undo`, `help`. With dynamic input
  (F12) the prompt, live measurement and typed input follow the cursor.
* **Views:** click the orientation gizmo (bottom-left) for plan, elevations (axis ends) or isometric
  (background); the header toolbar has zoom extents, isometric, plan/3D and perspective.
* **Selection:** click, Shift+click to add; drag left→right for a window (fully inside), right→left for a
  crossing (touching).
* **Editing:** selected objects show grips — click one, then pick (or type) the new position. Vertex grips
  stretch (type `d` to delete the vertex); a segment's middle grip inserts a vertex, or reshapes an arc.
  Heights are kept unless the pick snaps to a known height. `move`, `copy`, `rotate` (from a reference
  bearing to a new bearing) and `offset` work on the selection or ask for objects; `explode`, `join` and
  `reverse` act on selected polylines. In the polyline tool, `a` switches to tangent arcs (`s` for a
  three-point arc) and `l` back to lines.
* **COGO:** `rad` (radiation: station, backsight, then `angle, distance[, dZ]`), `bb` / `bd` / `dd`
  (bearing/distance intersections), `chainage` (report chainage & offset along a polyline, or set out by
  `chainage[, offset]`), `along` (points at distance and offset along a line), `divide` (n parts or
  `@interval` along a polyline), `fillet`, `average` (mean of selected points with their spread), `area`
  (selected polylines or picked corners), `list` (legs of selected polylines). Results collect in
  View → Reports (copy or save as text).
* **Traverse:** `traverse` — angles right/left, deflections or bearings; horizontal or slope distances
  (zenith, instrument and target heights); scale factor; open, loop or link; Bowditch/Transit; a
  field-book report. Traverses are saved with the project by name and recompute from their control.
* **Closures & parcels:** `closure` checks a figure typed as courses (lines and arcs) or taken from a
  selected polyline — misclosure, precision, area — and solves any two blank values. `subdivide` cuts a
  given area off a parcel parallel to an edge or swung about a point.
* **Resection:** `resection` fixes a station from directions and/or distances to known points (least
  squares with residuals when redundant). `calc` solves curves (any two elements) and triangles.
* **Accuracy:** Project → Accuracy limits sets your own angular, linear (1:N and an allowance) and
  vertical limits; traverse and closure results then show PASS or FAIL. None are set by default.
* **Field-to-finish:** `codes` opens the feature code library (layers, colours, line or point features,
  control codes such as `ST`, `END`, `CL`, `PC`/`PT`; CSV import/export). Each code has a style: one of
  30+ point symbols and its size, colour (the feature's layer colour, or its own), line style (solid, dashed,
  dotted, dash-dot) and weight, and optionally its own labels. Codes may be wildcards (`MH*`). Process
  linework joins coded points (`KB1`, `KB2`, `BLD CL` …) into strings on feature layers; running it again
  replaces them. View → Legend lists the codes in use with their symbols.
* **Points table:** `points` (View → Points table) — filter, sort, double-click a cell to edit, click to select.
* **Surfaces:** Surface → Create surface… (or `surface`) builds a TIN from points with heights (all layers or
  chosen layers / code patterns), breakline polylines (by layer or code), an optional boundary and holes, and a
  maximum edge length. The Surfaces panel shows each surface (visibility, rebuild, edit, delete) and the active
  one's style: hill-shaded and coloured by height, triangle edges, contours with labelled major contours. The
  SURF status toggle takes pick heights from the surface. `volume` computes cut/fill between surfaces or a
  surface and a level (optionally inside a boundary) exactly, and can create a cut/fill surface; `sections`
  plots long and cross sections along a polyline; `contours` converts contours to polylines; `landxmlin` /
  `landxmlout` import and export LandXML surfaces.
* **Projections:** Settings → Coordinates → Projected grid sets a UTM zone (WGS84 or GRS80 — MGA, ETRS89,
  NAD83 and similar) or any Transverse Mercator, an EPSG code and a geoid separation. `projcalc` converts
  latitude/longitude ↔ grid and gives point scale factor, convergence, height and combined factors; the
  traverse "From project" button fills its scale factor with the combined factor at the start point.
* **Network adjustment:** `rawin` imports Leica GSI (8/16) or Trimble JobXML observations (and adds any
  coordinates in the file). `netadj` adjusts imported data and saved traverses together by least squares:
  set the a priori precision, choose the fixed control, and read σ₀, the χ² test, standardised residuals
  (|w| > 3 flags a likely blunder) and 95 % error ellipses (drawn in the view, exaggerated); Report gives
  the full listing and Update creates or moves the adjusted points (undoable).
* **Modules** (Settings → General → Modules; each adds a menu):
  * **Cadastral:** `mkparcel` makes parcels of selected closed polylines; `parcels` lists areas and plan
    closures, reports them and switches on bearing/distance and area annotation; `parcelxml` exports
    LandXML parcels.
  * **Construction:** `mkalign` makes an alignment of a centreline polyline (start chainage, chainage ticks);
    `alignments` shows the element table and sets out points at intervals and offsets with a set-out table;
    `stakeout` lists angles and distances from an instrument station to the selected points.
  * **Mining:** `stockpile` measures a stockpile inside a toe (toe surface, lowest toe or level base) with
    tonnes; `volume` takes a density for period volumes; `cloudin` imports LAS or XYZ point clouds, drawn
    with level of detail (points stream to the GPU as you zoom), thinned to a spacing, and convertible to a
    surface by grid thinning (Point clouds panel).
* **Point input:** pick with snapping, or type `E,N[,Z]`, `@dE,dN[,dZ]`, `@dist<bearing`, or an existing point name.
  Order, units and angle format follow the project conventions (Project → Conventions).
* **Snapping & drafting:** Settings (gear) → Drafting holds snap modes, aperture, ortho/polar, grid and working
  elevation. The status bar has toggles for object snap (SNAP and each mode), GRID (snap to grid; spacing
  follows the visible grid or a fixed value), ORTHO and DYN, beside a fixed-width cursor readout.
  Shortcuts: F3 object snap, F9 snap to grid, F8 ortho, F12 dynamic input, F7 grid display. Tab cycles overlapping candidates; typing
  `end`/`mid`/`int`/`per`/`nea`/`nod`/`gri` applies a one-shot snap. 
* **Plan sheets:** File → Plan sheets… (`sheets`): frame the area in the view and press *New from view*
  (centred, rotated as the plan view, at the standard scale that fits). Each sheet has a paper size (A4–A0,
  US sizes or custom) and orientation, a scale, centre and rotation, a title block (title, subtitle,
  surveyor, date, reference, sheet, notes) and switches for linework, symbols, point labels, parcel
  bearings/distances/areas, contours, chainages, the coordinate grid (ticks, crosses or lines), north arrow,
  scale bar, legend and black-and-white. The preview is the printed page; export it as PDF or SVG at true
  scale (`plotpdf` exports the current sheet). The sheet frame is drawn in the view while the window is open.
* **Files:** Ctrl+S saves an `.s3d` (download on the web); File → Import points (CSV/TXT), Import / Export
  DXF (exports carry code symbols as blocks, line types, colours and point names, sized for the current
  plan sheet's scale), Import raw data, Export GeoPackage (`gpkg`: points, lines and parcels for GIS). On the web, unsaved work is autosaved to browser storage and offered for recovery on the next visit.
* **Labels & appearance:** Settings → Display (or `labels`): point names, codes and heights; dark/light
  theme, accent colour and density.
  Project → Audit trail shows every edit and verifies the hash chain.

## Agents (MCP)

`survey3d-mcp` is a [Model Context Protocol](https://modelcontextprotocol.io) server (stdio) that gives
an agent the same session the application uses: it types on the command line exactly as you would,
so every tool, point-input form and convention works, and every edit is undoable and recorded in the
project's audit trail under `<user> (agent via MCP)`.

```sh
cargo build --release -p survey-mcp
# Claude Code (add --scope project to share it through .mcp.json):
claude mcp add survey3d -- "$PWD/target/release/survey3d-mcp" --project /path/to/job.s3d
```

This repository's `.mcp.json` already registers it for Claude Code (as `cargo run --release -p survey-mcp`,
so it builds on first use; Claude Code asks you to approve it once). The agent then opens a project
with `open_project`.

Other clients take the usual entry: `{"command": "/path/to/survey3d-mcp", "args": ["--project", "job.s3d"]}`.
Options: `--project FILE` (otherwise a new project), `--read-only` (never writes the project file; edits
stay in memory), `--user NAME` (audit trail; default `$USER`).

### Driving the open window (agent link)

To watch an agent work, let it drive the survey3d window you have open: Settings → General → **Agent
link (MCP)** → *Let agents drive this window* (or the `agent` command). The app then serves MCP on
`http://127.0.0.1:3850/mcp` (port in the same settings). Each change appears as it is made, is one undoable
step, and is recorded in the audit trail as yours via the agent. A status-bar chip shows the link and
what the agent last did; a tool you are in the middle of is cancelled when the agent edits, not when it
only looks. Linux, macOS and Windows alike.

* **Claude Code** (HTTP): *Copy Claude Code command* in the settings gives
  `claude mcp add --transport http survey3d http://127.0.0.1:3850/mcp --header "Authorization: Bearer <token>"`.
* **Clients that start a program** (such as Claude Desktop): `survey3d-mcp --attach` relays stdio to the
  window — *Copy stdio config* gives `{"mcpServers": {"survey3d": {"command": "…/survey3d-mcp", "args": ["--attach"]}}}`.
  When the window is not open, the agent is told to open it and turn the link on.

The link listens on 127.0.0.1 only, refuses requests whose Host or Origin is not local (against DNS
rebinding from web pages) and requires a bearer token. The token and port are kept in a per-user file
only you can read (`agent-link.json` in `%APPDATA%\survey3d`, `~/Library/Application Support/survey3d` or
`~/.config/survey3d`), so a client set up once keeps working. It is off by default.

| Tool | What it does |
|---|---|
| `project_info` | File, conventions, counts, layers, surfaces, clouds, records, selection, active tool and prompt, undo/redo |
| `run_commands` | Types lines on the command line (`help`, `pl`, `1000,2000`, `@10<90`, `c`, `inverse`, `undo`…) and returns the output and reports |
| `query` | Points, polylines, text, layers, feature codes, surfaces, clouds, sheets/parcels/alignments/traverses/field data, selection, audit trail — filtered and paged |
| `select` | Sets the selection that commands such as `delete`, `join`, `list`, `mkparcel`, `linework` act on |
| `set_project`, `set_feature_codes`, `set_layer` | Job setup: name and coordinate system (e.g. `"MGA2020 zone 50"`), the feature-code library (symbols, layers, line styles, linework), layer colours, line styles and the current layer |
| `add_points`, `add_polyline`, `add_text` | Structured input in one undo step (a polyline without heights is plan-only) |
| `edit_entities` | Rename, recode, move to another layer, edit text — the objects a filter matches, in one undo step |
| `create_surface`, `set_surface`, `delete_surface` | Surfaces from point layers/codes, breaklines, a boundary and holes; contour and display style; rebuild after the drawing changes |
| `set_record`, `delete_record` | Create or edit (merging fields) sheets, parcels, alignments and traverses by id, one or many at once |
| `import_file`, `export_file` | CSV points, DXF, LandXML, GSI/JobXML, LAS/XYZ in; DXF, LandXML, GeoPackage, CSV, PDF, SVG out (no silent overwrites) |
| `render` | A PNG of a plan sheet, or an overview of the whole drawing, so the agent can see its work |
| `new_project`, `open_project`, `save_project` | Won't drop unsaved changes unless told to |

## License

MIT — see [LICENSE](LICENSE). Bundled fonts are under their own licences (see `crates/egui-cad/assets`).
