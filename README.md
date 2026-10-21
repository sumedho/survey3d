# survey3d

Surveying CAD for cadastral, construction and mining work — Rust, [egui](https://github.com/emilk/egui)
and [wgpu](https://wgpu.rs), running natively and in the browser (WebGPU with WebGL2 fallback).

Architecture, decisions and roadmap: [`docs/PLAN.md`](docs/PLAN.md).

## Run

```sh
cargo run                    # native (survey-app is the default member)
./scripts/serve-web.sh       # web, http://127.0.0.1:8765 (needs `cargo install trunk --locked --version 0.21.14`)
cargo test --workspace
```

The web build compiles SQLite to wasm with clang. On macOS install an LLVM with the wasm32 target
(`brew install llvm`); the script picks it up automatically.

## Workspace

| Crate | Purpose |
|---|---|
| `survey-core` | Pure maths: angles/bearings, conventions (parse/format), geometry, arcs (bulges), offsets, COGO (intersections, chainage/offset), traverse adjustment, field-to-finish, point input |
| `survey-model` | `Document`, entities, layers, `Command`/`ChangeSet`, undo `History` with audit records, R-tree index |
| `survey-io` | Interchange formats: CSV/TXT points (configurable columns), DXF read/write, feature code sheets |
| `survey-store` | `.s3d` project files (SQLite): incremental saves, migrations, hash-chained append-only audit trail |
| `cad-render` | wgpu renderer: f64 camera, relative-to-center chunks, reverse-Z, offscreen MSAA, egui paint callback |
| `egui-cad` | Reusable CAD widgets: theme, viewport, snapping, box selection, label placement, command line, panels |
| `survey-app` | The application: tools, panels, dialogs, file handling |

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
  30+ point symbols and its size, colour by layer or the feature's own, line style (solid, dashed,
  dotted, dash-dot) and weight, and optionally its own labels. Codes may be wildcards (`MH*`). Process
  linework joins coded points (`KB1`, `KB2`, `BLD CL` …) into strings on feature layers; running it again
  replaces them. View → Legend lists the codes in use with their symbols.
* **Points table:** `points` (View → Points table) — filter, sort, double-click a cell to edit, click to select.
* **Point input:** pick with snapping, or type `E,N[,Z]`, `@dE,dN[,dZ]`, `@dist<bearing`, or an existing point name.
  Order, units and angle format follow the project conventions (Project → Conventions).
* **Snapping & drafting:** Settings (gear) → Drafting holds snap modes, aperture, ortho/polar, grid and working
  elevation. Shortcuts: F3 snap, F8 ortho, F7 grid. Tab cycles overlapping candidates; typing
  `end`/`mid`/`int`/`per`/`nea`/`nod`/`gri` applies a one-shot snap. The status bar shows feedback only.
* **Files:** Ctrl+S saves an `.s3d` (download on the web); File → Import points (CSV/TXT), Import / Export
  DXF. On the web, unsaved work is autosaved to browser storage and offered for recovery on the next visit.
* **Labels & appearance:** Settings → Display (or `labels`): point names, codes and heights; dark/light
  theme, accent colour and density.
  Project → Audit trail shows every edit and verifies the hash chain.

## License

MIT — see [LICENSE](LICENSE). Bundled fonts are under their own licences (see `crates/egui-cad/assets`).
