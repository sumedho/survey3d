# survey3d — Architecture & Roadmap

General surveying application for **cadastral, construction and mining** work.
Rust + egui + wgpu, running natively and on the web (wasm, WebGPU with WebGL2 fallback).

## Agreed decisions

| Topic | Decision |
|---|---|
| UI | egui / eframe (0.37+) with a reusable `egui-cad` widget library |
| Rendering | wgpu via `egui_wgpu` paint callbacks; WebGPU → WebGL2 fallback; native Metal/Vulkan/DX12 |
| Targets | Native and web from the first pass |
| Conventions | Fully configurable, no regional bias. Canonical storage (metres, radians, E/N/Z); a `Conventions` profile controls parsing/display (bearing format, coord order, angle/distance units, chainage format). Presets ship (AU, NA, metric-generic). |
| Precision | Jitter-free for local grids *and* projected grids (MGA/UTM ~7,000,000 N). See *Precision* below. |
| Snapping | Core feature, included in Phase 0. See *Snapping* below. |
| Storage | SQLite project file, extension **`.s3d`**. See *Storage* below. |
| Audit | Permanent, append-only audit trail of edits. |
| Sync | Field↔office sync / multi-user editing is **out of scope**. |

## Workspace layout

```
crates/
  survey-core/     pure math: f64 geometry, Angle/Bearing, DMS/quadrant parsing, Conventions, COGO, transforms
  survey-model/    Document: points, strings, layers, feature codes, observations, surfaces;
                   selection; Command + UndoStack; R-tree spatial index; CoordinateSystem
  survey-store/    ProjectStore trait + SQLite implementation (.s3d), migrations, audit trail
  survey-io/       Importer/Exporter traits: CSV (configurable columns), DXF, LandXML, JobXML, GeoPackage export
  survey-surface/  TIN (spade CDT), contours, sections, volumes
  egui-cad/        reusable CAD widgets — no survey knowledge
  cad-render/      wgpu renderer: grid, points, lines, triangles, picking; relative-to-center encoding
  survey-app/      eframe app; tool registry; domain modules (cadastral / construction / mining)
```

### `egui-cad` widget library
`CadTheme` (tokens, dark/light, compact density, icons) · `Viewport` (orbit / pan / zoom-to-cursor,
perspective/ortho, locked `Plan2d` mode, view cube, zoom extents/window, scale bar) · snap engine + overlays ·
`CommandLine` · `Ribbon`/`Toolbar`/`ToolPalette` · `StatusBar` · `PropertyGrid` · `LayerPanel` · `TreeView` ·
virtualised `DataTable` · `DmsEdit` / `CoordEdit` / `DistanceEdit` · dock layout.

### Extension points
```rust
trait Tool          { fn on_input(..) -> ToolResult; fn prompt(&self) -> &str; fn draw_preview(..); }
trait Command       { fn apply(&mut self, doc: &mut Document); fn revert(&mut self, doc: &mut Document); }
trait Importer / Exporter
trait SnapProvider  { fn query(&self, region: &Aabb3, out: &mut Vec<SnapGeom>); }
trait ProjectStore  // async-friendly so a web-worker (OPFS) backend can slot in
trait DomainModule  { fn register(&self, reg: &mut Registry); }  // tools, ribbon tabs, panels, reports
```

## Precision (large coordinates)

- f64 everywhere on the CPU: model, camera (eye/target/pivot), view matrices, picking, snapping, measurement.
- Geometry is chunked (≤ ~2 km); each chunk has an f64 origin and f32 vertex offsets (relative-to-center).
- Per frame, `view_rotation × translate(chunk.origin − eye)` is computed in f64, then cast to f32.
- Orbit pivots around an f64 point (snapped/picked under cursor).
- Tight per-frame near/far; reverse-Z + `Depth32Float` on native/WebGPU.
- Procedural grid with phase computed in f64; overlays/labels positioned by f64 projection.
- Future: relative-to-eye high/low f32 split for very large point clouds (`PositionEncoding` abstraction).
- Tests: identical screen positions (< 0.01 px) for a feature at local vs MGA coordinates after orbiting;
  exact snap round-trip at MGA scale.

## Snapping

- Modes (Phase 0): point/node, endpoint, midpoint, nearest, intersection (incl. apparent in plan),
  perpendicular, centre, grid. Later: tangent, extension, quadrant, tracking, surface.
- Pixel aperture; candidates ranked by mode priority then screen distance; Tab cycles candidates.
- Plan mode snaps in XY with configurable Z source; 3D mode snaps true 3D geometry, falls back to a working plane.
- Per-mode glyphs and tooltip; F3 master toggle; per-mode status-bar toggles; one-shot overrides from the command line.
- Ortho/polar lock (F8) with configurable bearing increments.
- Typed input everywhere: absolute, relative `@dE,dN`, polar `@dist<bearing` (parsed per Conventions).
- Engine is pure logic behind a `Camera` trait → unit-tested without a GPU.

## Storage (`.s3d`)

- In-memory `Document` is the source of truth while editing; SQLite is persistence.
- Saves write only dirty entities (tracked by the Command system) in one transaction; enables autosave.
- Native: `rusqlite` (`bundled`), WAL mode. Web Phase 0: in-memory DB, open = upload + deserialize,
  save = serialize + download. Web later: OPFS via `sqlite-wasm-vfs` in a dedicated worker.
- IDs: `i64` integer primary keys (no sync requirement); remap on cross-project paste/import.
- Schema versioned with `PRAGMA user_version` + ordered migrations.
- Tables: `project_meta`, `coord_system`, `conventions`, `layers`, `feature_codes`, `points`, `strings`,
  `string_vertices`, `setups`, `observations` (raw, immutable), `surfaces`, `surface_chunks` (BLOB, same
  origin+offset layout as the renderer), `pointcloud_chunks`, `sources` (import provenance), `attachments`, `audit_log`.
- Domain-specific attributes in `attrs_json` columns; promote to real columns when queried often.

### Audit trail
- Append-only `audit_log(id, ts_utc, user, app_version, command, summary, entity_refs, before_json, after_json, prev_hash, hash)`.
- One entry per committed `Command` (undo/redo are themselves logged as entries, never deletions).
- `hash = sha256(prev_hash || entry)` gives tamper evidence; a "Verify audit trail" tool checks the chain.
- User name comes from app settings (falls back to OS user on native).

## Phase 0 status (2026-10-21): complete

Implemented as planned, with these deviations:
- **No `egui_dock` / `egui-phosphor`:** both still target egui 0.36. Panels use egui 0.37's `Panel`
  API; the Phosphor icon font is bundled in `egui-cad/assets` (MIT) with constants in `egui_cad::icons`.
- **Layout:** fixed panels (menu/toolbar, tool palette, layers + properties, command line, status bar);
  docking can be revisited when `egui_dock` supports egui 0.37.
- **Snap modes:** centre snap waits for arcs (Phase 1). Node outranks a coincident endpoint so a point's
  "height unknown" state is preserved.
- **Layer visibility/lock** are saved display state, not undoable or audited; renames, colours,
  additions and deletions are audited edits.
- **Rendering:** the scene is drawn into an offscreen MSAA colour + reverse-Z depth target in `prepare`,
  then blitted in egui's pass. Shaders are validated (and translated to GLSL ES 3.0) in unit tests.
- **Verified in the browser** (headless Chrome): typed and snapped input, polyline/inverse tools,
  selection/properties editing, undo, 3D orbit/perspective, conventions switching, `.s3d` download.
  Not yet exercised interactively: the CSV import dialog (parser is unit-tested) and opening `.s3d`
  files on the web.

Known follow-ups: scale bar always labels metres; point labels; window/crossing selection;
web autosave (OPFS); remembering camera per project.

## Phase 1 status (2026-10-21): complete

- **Arcs** are DXF-style *bulges* on polyline segments (`survey_core::arc`): one representation for
  cadastral boundaries, circles (two semicircles) and DXF interchange. Exact plan-view geometry: lengths,
  areas, line–arc / arc–arc intersections; tessellated only for drawing and 3D apparent snaps.
- **Tools:** Arc (3-point) and Circle (centre + picked/typed radius). Centre snap added (offered while
  hovering an arc). Single-letter command aliases were removed so they never shadow tool options.
- **Window / crossing selection** (`egui_cad::selection`): drag left→right = window, right→left = crossing.
- **Point labels** (`egui_cad::labels`): name/code/height with collision culling; Settings → Display.
- **DXF** (`survey_io::dxf`, dependency-free): reads POINT, LINE, LWPOLYLINE, POLYLINE/VERTEX (2D/3D), ARC,
  CIRCLE, layers + colours (ACI / true colour), mirrored OCS; writes R12. Imports are one undoable,
  audited `ImportEntities` command that also creates layers.
- **Storage:** schema v2 (vertex bulges) via the migration system; per-project saved view; snapshot
  bytes for recovery.
- **Web autosave:** unsaved edits are snapshotted to OPFS every 15 s; the next session offers recovery.
- Fixes: unit-aware scale bar; untouched new projects are no longer "unsaved".

Known follow-ups: polyline tool arc segments (tangent/3-point within a polyline), editing vertices,
DXF TEXT/INSERT, point names in DXF export (as text), native autosave.

## Phase 2 status (2026-10-22): complete

- **Editing:** grips (stretch / insert vertex / reshape arc / delete vertex; heights kept unless the pick
  snaps to a known height), Move, Copy, Rotate (reference bearing → new bearing, clockwise like
  bearings), Offset (distance or through point; arcs change radius, corners are trimmed/extended, gaps
  bevelled), Explode, Join, Reverse, tangent and three-point arcs inside the polyline tool. Moves and
  copies are plan-only so heights never change by snapping.
- **COGO:** radiation, bearing–bearing / bearing–distance / distance–distance intersections,
  chainage & offset along polylines (with arcs; offsets positive right).
- **Traverse:** `survey_core::traverse` — angles or bearings, angular misclosure against a closing
  reference distributed equally, linear misclosure, precision, Bowditch / Transit, heights from dZ.
  Window with live reduction; creates points (code `TRAV`) and an optional polyline in one command.
- **Field-to-finish:** `survey_core::field_to_finish` — feature library saved in the project
  (`project_meta.feature_codes`, no migration needed), configurable control codes, string numbers,
  arcs PC…PT. Generated strings are named `CODE.n` and replaced on re-processing.
- **Panels:** points table (virtualised, sort, filter, inline edit), Reports window, `list`.
- Tool trait gained `activate()` (pick up the selection) and `claims_text()` (tool values before point
  names/commands); tools can be command-only (`palette: false`).

Known follow-ups: traverse observations are not yet stored in the project (only their results);
scale factors / combined scale factor for traverse distances (Phase 5 projections); mirror/scale/trim/
extend; drag-to-move grips; coordinate CSV export from the points table.

## Phase 3 status (2026-10-22): complete

- **3a** `survey_core::tin` (spade CDT: points, breaklines with crossing vertices, boundary/holes as
  constraints with surface heights, max edge; relative coordinates; `TinIndex` grid for height lookup).
  Surfaces live beside entities (`SurfaceId`, undoable insert/update/delete plus a style-only op; audit
  summaries, not triangles), schema v3 `surfaces` table (JSON definition/style, binary vertices/faces),
  `SurfaceDefinition::gather` for rebuilds. Renderer: filled-triangle pipeline and multiple scenes per
  viewport (surfaces rebuild only when they change); plan view squeezes surfaces behind lines and points.
  Surfaces panel, Create/Edit window, SURF pick draping.
- **3b** `survey_core::contour` (marching triangles, chained open/closed lines, major levels); contours drawn
  with the surface, labels along major contours, contours → polylines.
- **3c** `survey_core::volume`: merged CDT of both TINs and the boundary, exact linear integration split at
  zero; cut/fill/net, areas, difference (isopach) surface.
- **3d** `survey_core::tin::profile` (exact breaks, gaps off-surface); Sections window (long and cross
  sections, exaggeration, hover readout, report).
- **3e** `survey_io::landxml` (quick-xml): read TIN surfaces (any ids, invisible faces, feet), write LandXML 1.2.

Follow-ups: smoothing for contours; surface statistics and slope/aspect shading; volume by grid for very
large TINs and multi-polygon boundaries; sections plotted with design templates; surfaces in DXF (3DFACE);
LandXML alignments and points.

## Phase 2.5 status (2026-10-22): complete

- **2.5a** `survey_core::closure` (figure closure, two-unknown solutions incl. ambiguous cases, arcs by
  chord) and `survey_core::accuracy` (optional user limits, stored as `project_meta.tolerances`); Closure
  window; Settings → Accuracy.
- **2.5b** traverse directions (right / left / deflection / bearing), slope + zenith + HI/HT, scale
  factor, angle counts, field-book report with checks; `TraverseRecord` saved in
  `project_meta.traverses` and resolved by point name.
- **2.5c** `survey_core::subdivide` (parallel and swing cuts by bisection on clipped area),
  `survey_core::resection` (weighted Gauss–Newton from many starts, orientation, residuals, σ₀);
  Area tool, Subdivide and Resection windows.
- **2.5d** `survey_core::solvers` (curve from any two elements, triangle SSS/SAS/ASA/AAS/SSA), fillet,
  divide, point along line, averaging; COGO calculator window.
- **2.5e** `survey_core::style`: 31 symbols defined as primitives, evaluated as SDFs in the point shader
  from a uniform symbol table (WebGL2-safe) and drawn with the same primitives in egui; line patterns
  in the line shader with distance carried along polylines. `FeatureCode.style` (symbol, size, by layer
  / own colour, line style, weight, label override), wildcard codes (`*`, `?`; plain codes win, then the
  most specific pattern), CSV code sheets gain style columns (old sheets still load), legend window.
  Tools that take codes (point, radiate) claim library codes before command names (`CP` vs `cp`).

Follow-ups: DXF export of symbols as blocks and of line types; symbol size in world units (plan scale);
attributes per code; traverse observations from raw data files; a light-canvas legend preview.

## Phase 2.5 plan (2026-10-22): more COGO, accuracy and code styling

Research (Trimble Access / TBC feature libraries and COGO, Carlson code tables and COGO, Civil 3D
description keys, Leica Captivate codelists, 12d) agreed with the user; COGO first, then styling.

- **2.5a Closures:** figure / lot closure from a polyline or typed courses (lines and arcs):
  misclosure bearing and distance, precision, area, perimeter. Missing-value solutions for any two
  unknowns (bearing + distance of one course, two distances, two bearings, a bearing and a distance;
  two-solution cases offer both).
- **2.5b Traverse + accuracy:** traverses stored in the project (re-open, edit, recompute when control
  moves); angle right / left / deflection / bearing; slope distance + zenith with instrument and target
  heights; scale factor; open, loop and link traverses; field-book style report. Accuracy limits are
  user-defined (no regional defaults): angular `k·√n` seconds and linear `1:N` / absolute, with pass or
  fail shown in the traverse and closure results.
- **2.5c Area & resection:** area by pick or polyline (arcs), area subdivision by parallel line or
  swing line about a hinge point; resection (two or three known points, angles and distances, least
  squares when redundant) with residuals.
- **2.5d Solvers & helpers:** curve solver (any two of radius / delta / arc / chord / tangent /
  external / mid-ordinate), triangle solver (SSS, SAS, ASA, SAA, SSA with the ambiguous case), fillet,
  divide line or arc (n parts or interval), point along a line, average points, point-to-line inverse.
- **2.5e Code styling:** each feature code gains a point symbol from a built-in vector library (~30
  surveying symbols, GPU instanced), symbol size, colour by layer or custom, line style (solid, dashed,
  dotted, dash-dot) and weight, wildcard patterns (`EP*`), label choices; symbol picker and legend.

## UI design system (egui-cad)

Based on research into professional CAD/3D tools (AutoCAD, Fusion, Blender, Onshape) and dark-UI
guidance (Material dark theme, WCAG):

- **Tokens, not colours.** `CadTheme` exposes semantic tokens: surfaces by elevation
  (`bg → panel → raised → overlay`, plus `viewport_bg`), three border weights, three text levels,
  accent (+ hover / pressed / subtle / on-accent / focus), status colours and canvas colours.
- **Dark mode = lighter means higher.** No pure black or white; elevation by lightness rather than
  shadow; light mode uses white overlays with soft shadows.
- **One desaturated accent** (Amber / Blue / Teal), used for the active tool, focus and selection.
- **Contrast is tested**: body text ≥ 7:1 and secondary text ≥ 4.5:1 on every surface, text on the
  accent ≥ 4.5:1, accent vs panel ≥ 3:1, in both modes and all accents (`theme.rs` tests).
- **Density**: Compact / Comfortable on a 2/4 px grid.
- **Typography**: Inter (variable; regular + medium via `wght`) for UI, JetBrains Mono for numbers
  and input (equal-width digits), Phosphor icons in a dedicated family (Inter defines glyphs in the
  Private Use Area, so icons must use `fonts::icon`). All OFL/MIT, bundled in `egui-cad/assets`.
- **Widgets**: icon buttons with rest/hover/pressed/selected/disabled/focus states and rich tooltips
  (title, description, shortcut keycap); collapsible sections with header actions; segmented
  controls; switches; keycaps; floating frames; empty states; command palette (Ctrl/Cmd+K, fuzzy
  search, recents); toasts; cursor HUD (dynamic input, F12); interactive orientation gizmo (click for
  elevations / plan / isometric) and a floating view-control pill.
- **Canvas colour adaptation**: on a light canvas, near-white data draws in the text colour and very
  bright colours are darkened for display; stored colours never change.

## Roadmap

- **Phase 0 — scaffold (done):** workspace; native + web builds; wgpu viewport (grid, axes, points, lines) with
  relative-to-center precision; 3D orbit + locked 2D plan; `CadTheme`; dock layout (viewport, layers,
  properties, command line, status bar); Conventions system; Document (points, polylines, layers) with
  R-tree and undo/redo; **snapping**; tools: Select, Point, Line/Polyline, Inverse; CSV import;
  `.s3d` save/load with audit trail; core unit tests; clippy/fmt/wasm CI config.
- **Phase 1 — CAD basics (done):** arcs, window/crossing selection, labels, property editing, DXF import/export, OPFS autosave.
- **Phase 2 — COGO & field-to-finish (done):** editing (grips, move/copy/rotate/offset, explode/join),
  radiation, intersections, chainage/offset, traverse + Bowditch/Transit with misclosure report, feature
  code library and auto-linework, points table, reports.
- **Phase 2.5 — COGO, accuracy, code styling (done):** closures, traverse upgrade, area,
  resection, solvers, code symbols and line styles.
- **Phase 3 — surfaces (done):** TIN with breaklines, contours, volumes, sections, LandXML surfaces.
- **Phase 4 — domain modules:** cadastral (parcels, closures, areas, plan annotation, LandXML/ePlan);
  construction (alignments, chainage/offset, setout); mining (pickups, EOM volumes, stockpiles, point cloud LOD).
- **Phase 5 — advanced:** least-squares network adjustment, JobXML/GSI raw data, projections & scale factors,
  point cloud streaming, GeoPackage export.
