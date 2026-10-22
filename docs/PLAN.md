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
  survey-app/      lib: Session (headless editing: tools, command line, edits, imports/exports);
                   bin: eframe app over a Session; domain modules (cadastral / construction / mining)
```

### Session and records
- `Session` owns the project, tool registry, active tool and command-line log. Every edit the GUI makes
  goes through it, so scripts, tests and agents get the same behaviour: `run(text)` takes command-line
  input exactly as typed. Operations log their results and queue `Event`s (toast, report, zoom, open a
  panel) that the GUI drains each frame and other callers may read or ignore.
- Project records (traverses, parcels, alignments, raw field data, plan sheets) live in `Document`
  as `Keyed<T>` lists with a stable `RecordId`; `ProjectMeta` holds settings only. A change to a record
  is its own undo op (only that record is copied into the history) and marks only its collection for
  saving. Files written before ids load with ids assigned in file order; the next save stores them.

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

## Phase 6 status (2026-10-22): complete

- **6a:** `survey-io::plot` — `Plot` (paths, pens with dash patterns, circles, text with alignment,
  rotation and halo, even-odd clip groups) in page millimetres; `write_pdf` (PDF 1.4, base-14 Helvetica with
  AFM metrics and WinAnsi encoding, Flate content) and `write_svg`. Checked with poppler and rsvg.
- **6b:** `SheetRecord` in `ProjectMeta.sheets` (persisted); `plan::build_plot` draws linework with code
  line types, symbols, collision-avoided point labels, parcel annotation, contours with labels, chainages,
  grid ticks/crosses/lines with values, north arrow, scale bar, legend of codes in the frame and the title
  block (corner box, or a strip on narrow pages); the map is clipped around the boxes. Bright layer colours
  are darkened for paper; optional monochrome.
- **6c:** Plan sheets window (`sheets`) with a live, zoomable preview painted from the same plot, new from
  view / fit drawing / centre and rotate as view, edits committed as one undo step, sheet frame drawn in
  the viewport, PDF/SVG export (`plotpdf`).
- **6d:** DXF export writes code symbols as blocks (`S3D_<SYMBOL>`, inserted at the plan size of the current
  sheet, BYBLOCK colour), code line types as `LTYPE`s with paper-sized dashes, code colours on entities and
  point names as `TEXT` on `POINT_NAMES`. GDAL reads the line types, colours, blocks and text.

Follow-ups: multi-page sets and sheet layouts along an alignment; viewports within a sheet (insets,
location diagram); symbol and text sizes on screen at the sheet's scale (plan-scale display); embedding a
font for full Unicode in PDFs (currently WinAnsi, primes become ' and "); title block templates and logos;
text along curved contours; dimension and leader annotation; DXF text/insert import.

## Phase 6 plan (2026-10-22): plan output

Getting a finished drawing out of the app: plan sheets at a true scale, printed to PDF/SVG.

- **6a — plot writers** (`survey-io::plot`): a paper-space drawing (paths with pens and dash patterns,
  circles, text with alignment/rotation/halo, even-odd clip groups) in millimetres, written as SVG and
  as PDF (hand-written, base-14 Helvetica with its metrics, Flate-compressed content). No new
  dependencies beyond `miniz_oxide` (already in the tree).
- **6b — sheets and the plan builder:** `SheetRecord` in the project (paper A4–A0/ANSI/custom,
  orientation, scale 1:n, centre, rotation, title block fields, content switches, text/symbol sizes in
  paper mm). The builder turns the document into a plot: linework with code styles and line types,
  point symbols, point labels (collision-avoided), parcel bearings/distances/areas, contours with
  labels, alignment chainages, coordinate grid (ticks, crosses or lines with labels), north arrow,
  scale bar, legend of the codes shown and a title block. Paper colours: bright layer colours are
  darkened for white paper; optional monochrome.
- **6c — sheets window:** list of sheets, settings, live preview drawn from the same plot (so it is what
  gets printed), "centre on view" / "fit drawing", the sheet frame drawn in the viewport, PDF and SVG
  export (download on the web).
- **6d — DXF export:** point symbols as BLOCKs inserted at a plan scale, line types (LTYPE table) from
  code styles, point names as TEXT.

## Phase 5 status (2026-10-22): complete

- **5a projections:** `survey_core::projection` — Transverse Mercator by Krüger's series to n⁶ (Karney
  2011) on GRS80, WGS84 or a custom ellipsoid; UTM zones; point scale factor, grid convergence (grid
  bearing of true north: grid bearing = true bearing + convergence), height factor R/(R+h) from the mean
  radius of curvature, combined factor. `CoordinateSystem::Projected` carries the projection and a geoid
  separation (serde defaults). Settings → Coordinates presets (UTM on WGS84, UTM on GRS80, custom TM — no
  regional default); projection calculator (`projcalc`); traverse scale factor "From project".
- **5b network adjustment:** `survey_core::adjustment` — weighted Gauss–Newton for directions (one
  orientation unknown per set), bearings and distances, plus a linear height-difference adjustment;
  residuals, standardised residuals from Qvv, σ₀, χ² global test (Wilson–Hilferty quantiles), standard
  deviations and 95 % ellipses; datum defects reported. `survey_core::network` turns instrument setups (raw
  data or saved traverses) into observations, propagates approximate coordinates by radiation, weights from
  directions + centring and distances mm + ppm, applies the scale factor. Network adjustment window
  (`netadj`) with control selection, ellipses in the viewport and an undoable update of the points.
- **5c raw data:** `survey_io::rawdata` — Leica GSI-8/16 and Trimble JobXML into `FieldData` (setups and
  recorded coordinates) kept in `project_meta.field_data`; face-right pointings reduced to face left;
  `rawin`.
- **5d GeoPackage:** `survey_store::geopackage` — OGC GeoPackage 1.4 with 3D points, lines (arcs
  tessellated to 5 mm) and parcels (anticlockwise polygons with area); EPSG code or TM WKT; checked with
  GDAL; `gpkg`.
- **5e cloud streaming:** the renderer uploads level-of-detail prefixes on demand (1.5M points per frame,
  largest needs first, 2× over-fetch), clamps draws to what is resident, evicts off-screen chunks LRU over
  a residency budget (8M web / 24M native) and requests frames until streaming settles. Voxel thinning
  (`PointCloud::thinned`), "Thin to" in the clouds panel, automatic thinning of huge imports (15M web /
  60M native).

Follow-ups: free-network (minimally constrained) adjustment and inner constraints; GNSS baselines in the
network; geoid model grids instead of a single separation; other projections (Lambert conformal conic,
oblique stereographic) and datum transformations; GSI/JobXML edge cases (multiple faces averaged into
rounds, RTK records, station setup types); GeoPackage import, attributes per feature code and an R-tree
spatial index; out-of-core clouds (reading LAS chunks from disk rather than holding all points in RAM)
and LAZ; sharing streamed buffers between viewports.

## Phase 4 status (2026-10-22): complete

- **4a modules & cadastral:** Cadastral / Construction / Mining switch on and off (Settings → General),
  each with its own menu. `ParcelRecord` (lot, plan, boundary polyline) in `project_meta.parcels`; Parcels
  window (areas, plan closure from courses rounded to the display precision, reports); bearing/distance
  and area annotation (shared boundaries once); LandXML parcels (`CgPoints` + `CoordGeom`).
- **4b construction:** `survey_core::alignment` (element table, set-out at intervals/offsets/TPs);
  `AlignmentRecord` in `project_meta.alignments`; Alignments window, chainage ticks/labels, set-out points
  and tables, stake-out sheet; chainage tool honours start chainage.
- **4c mining:** `survey_core::volume::stockpile` (toe draped on the surface, lowest toe or level), Stockpile
  window with tonnes; density/tonnes in Volumes (period volumes).
- **4d point clouds:** `survey_io::pointcloud` (XYZ text, uncompressed LAS 1.0–1.4 with RGB); `PointCloud`
  (f32 offsets from an origin, optional colours, style) stored in schema v4 `point_clouds`; level-of-detail
  rendering (points shuffled per chunk, per-frame draw counts from on-screen size, global budget, off-screen
  chunks skipped); Point clouds panel (colour by RGB/height/single, size); surface from a cloud by grid
  thinning (lowest/mean/highest).

Follow-ups: LAZ; out-of-core streaming for very large clouds (GPU streaming done in Phase 5); cloud snapping and section display;
ePlan-specific LandXML (monuments, observations); vertical alignments and design templates; stockpile
reports with photos/dates; parcel creation from linework regions.

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
- **Phase 4 — domain modules (done):** cadastral (parcels, closures, areas, plan annotation, LandXML/ePlan);
  construction (alignments, chainage/offset, setout); mining (pickups, EOM volumes, stockpiles, point cloud LOD).
- **Phase 5 — advanced (done):** least-squares network adjustment, JobXML/GSI raw data, projections & scale
  factors, point cloud streaming, GeoPackage export.
- **Phase 6 — plan output (done):** plan sheets with title block, grid, north arrow, scale bar and
  legend; PDF/SVG at true scale; DXF symbols as blocks and line types.
