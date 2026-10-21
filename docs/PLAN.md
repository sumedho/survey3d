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

## Roadmap

- **Phase 0 — scaffold (done):** workspace; native + web builds; wgpu viewport (grid, axes, points, lines) with
  relative-to-center precision; 3D orbit + locked 2D plan; `CadTheme`; dock layout (viewport, layers,
  properties, command line, status bar); Conventions system; Document (points, polylines, layers) with
  R-tree and undo/redo; **snapping**; tools: Select, Point, Line/Polyline, Inverse; CSV import;
  `.s3d` save/load with audit trail; core unit tests; clippy/fmt/wasm CI config.
- **Phase 1 — CAD basics:** arcs, window/crossing selection, labels, property editing, DXF import/export, OPFS autosave.
- **Phase 2 — COGO & field-to-finish:** radiation, intersections, offsets, traverse + Bowditch with misclosure
  report, feature code library and auto-linework.
- **Phase 3 — surfaces:** TIN with breaklines, contours, volumes, sections, LandXML surfaces.
- **Phase 4 — domain modules:** cadastral (parcels, closures, areas, plan annotation, LandXML/ePlan);
  construction (alignments, chainage/offset, setout); mining (pickups, EOM volumes, stockpiles, point cloud LOD).
- **Phase 5 — advanced:** least-squares network adjustment, JobXML/GSI raw data, projections & scale factors,
  point cloud streaming, GeoPackage export.
