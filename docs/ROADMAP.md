# survey3d — Roadmap

As of 2026-10-25. Phases continue the numbering in [`PLAN.md`](PLAN.md), whose Phases 0–7 are done or in
progress.

## Summary

survey3d already covers most of a survey office's daily work (COGO, traverse and network adjustment,
field-to-finish, surfaces, volumes, plan sheets, parcels). The gaps against Carlson Survey, Trimble Business
Center (TBC) and MicroSurvey CAD are in field data breadth, coordinate systems, drafting deliverables, point
cloud measurement and design.

The order puts robustness first, then the cadastral work survey3d is aimed at, then the integrations every
survey office expects. Effort figures are rough engineering estimates, not commitments.

| Phase | Focus | Effort |
| --- | --- | --- |
| 8 — Production hardening | Converter memory and disk checks, test corpus, CRS, GPU portability, CI | about 2 weeks |
| 9 — Cadastral deliverables | Curved legs, curve solver, line and curve tables, parcel on close | about 3 weeks |
| 10 — Field data breadth | RW5, SDR and Topcon import, raw data editing, stake-out export | about 3 weeks |
| 11 — Coordinate systems | Geoid grids, site calibration, grid and ground across the project | about 3 weeks |
| 12 — Point cloud survey tools | Snapping to the full cloud, classification, ground filtering, sections | about 4 weeks |
| 13 — Imagery and data exchange | Orthophotos, basemaps, shapefile, DWG | about 3 weeks |
| Later — Design | Vertical profiles, corridors, grading, only if construction design is in scope | months |

survey3d's distinct strengths stay central throughout: an audit trail of every edit, AI agent control
through MCP, one codebase for web and desktop, and COPC conversion about 6 times faster than PDAL.

## Where survey3d is today

Of the fifteen areas below, survey3d has seven in full, five in part and two missing, and leads in one. The
established packages compared are Carlson Survey, TBC and MicroSurvey CAD; 12d Model is included from
general knowledge, as no feature list for it was found.

| Area | What the established packages offer | survey3d | Gap |
| --- | --- | --- | --- |
| COGO | Inverse, intersections, sideshots, traverse entry, resection, offsets, curve solvers, lot closure, subdivision | Has | — |
| Traverse and adjustment | Traverse reduction, least squares network adjustment | Has | — |
| Field data import | RW5, JobXML, GSI, Topcon, Sokkia SDR; raw data editing | Partial | GSI, JobXML and RW5 with coded reduction and raw editing; no SDR or Topcon |
| GNSS processing | Baseline processing, RTK and PPK review (TBC) | Missing | Out of scope for now |
| Field-to-finish | Codes to symbols, layers and linework, with curve and offset control codes | Has | No best-fit, smoothing or templates (Carlson's BFIT, SMO, XSCT) |
| Coordinate systems | Projections, geoid models, site calibration, grid and ground | Partial | Projection calculator with combined scale factor, one geoid separation per project, site calibration; no geoid grids or project-wide grid/ground |
| Surfaces and volumes | TIN, breaklines, contours, volumes, sections, TIN editing, slope analysis | Has | No interactive TIN editing or slope analysis |
| Point clouds | Import, classification, feature extraction, cloud to surface, sections | Partial | COPC streaming and conversion; no classification, snapping to the full cloud or sections |
| Imagery | Orthophotos and basemaps under the drawing | Missing | Phase 13 |
| Plans and drafting | Plan sheets, title blocks, line and curve labels and tables, dimensions, legal descriptions, DWG | Partial | No legal descriptions; DXF only |
| Parcels | Lots from linework, closure, area, subdivision | Has | — |
| Design | Vertical profiles, corridors, grading, earthworks | Partial | Horizontal alignments only |
| Stake-out | Stake-out data to controllers; as-built comparison | Has | — |
| Data exchange | DWG, DXF, LandXML, shapefile, GeoPackage, CSV | Has | No shapefile or DWG |
| Audit and automation | Rare: edit history, scripting | Ahead | Audit trail, MCP agent control |

## Phase 8 — Production hardening (about 2 weeks)

The point cloud viewer and the native COPC converter work on the one real file tested (168 million points,
25 s to convert, 6 times faster than PDAL); this phase makes them safe on arbitrary files and hardware.

- [x] **Split oversized chunks** in the converter recursively, so a densely scanned spot in a large extent
  cannot load tens of millions of points onto one worker.
- [x] **Check free disk space** before converting (about the uncompressed size, roughly 6 GB for the test
  file) and ask before overwriting an existing `.copc.laz`.
- [ ] **Build a test corpus:** LAS 1.2 to 1.4, point formats 0 to 10, no colour or no GPS time, GeoTIFF-only
  CRS, extra bytes, bad header bounds, negative coordinates, tiny, flat, single-point, duplicate-only, and a
  1 billion point file.
- [ ] **Keep the coordinate system:** convert GeoTIFF keys to WKT (LAS 1.4 formats 6+ require it), and report
  dropped extra attributes and records.
- [ ] **Test the viewer** on Windows (DX12, Vulkan), Linux and integrated GPUs; size the GPU budget from device
  limits; fall back cleanly where the vertex-stage storage buffer is unsupported.
- [ ] **Re-enable CI** (paused while the point cloud work moved quickly), with a software GPU so the renderer
  tests run.
- [x] **Surface streaming errors** in the log instead of silently keeping the old view.

**Done when:** the corpus converts and reads back exactly, and the viewer runs on all three desktop platforms.

## Phase 9 — Cadastral deliverables (about 3 weeks)

Plans of subdivision need curves and tables that survey3d does not produce yet; Carlson and MicroSurvey
market heavily on both.

- [x] **Curved legs in `bd`:** radius with arc length or chord, and left or right, as written on plans. Done:
  by chord bearing and distance (`45.3020 15.32 r=20 l`) or tangent to the leg before (`r=20 a=15.71 r`).
- [x] **Curve solver:** any two of radius, arc length, chord, delta and tangent give the rest. Done: the
  `curve` command, the calculator and the MCP `solve_curve` tool.
- [x] **Line and curve tables** on plan sheets, with labels (L1, C1) on the drawing. Done: short boundaries
  or all of them (sheet setting `tables`).
- [x] **Parcel on close:** offer to create the parcel when `bd` closes a figure, instead of a separate
  `mkparcel` step.
- [x] **Compute the closing leg:** an option in `bd` to solve the last line from the start point rather than
  check it. Done: `j`.
- [x] **Dimensions** for plan sheets (aligned and linear). Done: `dim` and the MCP `add_dimension` tool;
  aligned, horizontal and vertical.
- [x] **Check field-to-finish control codes** (start, end, close, curve and offset) against Carlson's set.
  Done: Carlson's spellings by default (`+7`/`-7`, `CLO`, `PC` without `PT` as a three-point arc) and its
  `GAP`, `RECT`, `CIR`, `OH`/`OFL`/`OV` and `JPN`; each code configurable with several spellings.

**Done when:** a curved-frontage lot can be keyed from a plan, closed with its misclosure, and plotted with
line and curve tables.

## Phase 10 — Field data breadth (about 3 weeks)

survey3d reads Leica GSI and Trimble JobXML raw data only, so crews on Carlson, Sokkia or Topcon controllers
cannot bring their work in.

- [x] **Coded reduction** (found missing while planning this phase): raw shots kept no codes and went only into
  the network adjustment. Done: codes read from GSI (WI 41/71), JobXML (`Code`) and RW5 (notes); `reduce`
  orients each setup on its backsight and radiates its shots into coded points, setup by setup (so a traverse
  carries forward), averaging both faces; shots to known points are reported as checks.
- [x] **Carlson RW5** raw data import (SurvCE and SurvPC controllers). Done: occupations, backsights, line of
  sight, traverse, sideshot and angle-set records, stored points, units from the mode record; GNSS and
  off-centre records are counted and reported, not read.
- [ ] **Sokkia SDR** and **Topcon** raw data import. Waiting on sample files (the SDR field layout is known for
  station, target, backsight, coordinate and observation records; Topcon GTS-7 is documented by Maptek).
- [x] **Raw data editing:** correct a target height, code or backsight and re-reduce, with the change recorded
  in the audit trail. Done: the Raw data window and the MCP `raw_data` tool; corrections (station, instrument
  height, backsight, target, code, target height, excluded shot) are stored beside the recorded observations
  and also feed the network adjustment.
- [x] **Stake-out export:** points and lines to controller formats (CSV variants, JobXML, RW5 point files).
  Done: Trimble JobXML (points and two-point lines), Carlson RW5 stored points, Leica GSI-16 and CSV, from the
  selection or every point; `export_file` in MCP. Not yet tested on a controller.
- [x] **As-built comparison:** stake-out or as-built shots against design, reported as differences. Done:
  against design points (by name or nearest), lines (chainage, offset, dZ) or a surface (cut/fill), with
  tolerances, a report and labels; MCP `compare_asbuilt`.

**Done when:** a day's field data from each of the four main controller families imports, reduces and can be
sent back out for stake-out.

## COGO additions

From a check of everyday COGO against Carlson and MicroSurvey (2026-10-27).

- [x] **Apparent intersections, trim and extend:** `ix` makes a point where two lines (or arcs) would meet if
  extended, picked or typed as point pairs; `trim` and `extend` cut lines back to, or lengthen them to,
  boundary lines, arcs included. MCP: `trim_extend`.
- [x] **Angles in `bd`:** turned angles (`a`, `al`) and deflections (`dr`, `dl`) from the line arriving at the
  current point, with `bs` setting a backsight for the first leg.
- [x] **Point tools:** `renumber` (selection, shift or range, with clash checks; points from raw data are
  renamed through their shots), `raise` for heights, and `scale` by a factor or measured and true lengths.
- [ ] **Spiral (clothoid) curves** in alignments and the curve solver, for road set-out.
- [ ] **Compound and reverse curves** in the curve solver.
- [ ] **Building dimensions by right-angle legs** for detail surveys.

## Phase 11 — Coordinate systems (about 3 weeks)

Nearly every GNSS job needs heights on a geoid model and coordinates tied to local control. survey3d's
projection calculator already gives point scale factor, convergence and the combined scale factor, but the
project holds a single geoid separation and nothing ties coordinates to local control.

- [ ] **Geoid grids:** load common models (for example AUSGeoid2020, GEOID18, EGM2008) and convert ellipsoid
  heights to orthometric heights per point, replacing the single project separation.
- [x] **Site calibration (localization):** fit GNSS coordinates to local control points, with residuals
  reported. Done: `sitecal` window and the MCP `site_calibration` tool (similarity fit, free or fixed
  scale; constant or inclined-plane heights; live residuals and best-practice warnings).
- [ ] **Grid and ground across the project:** a project or per-point combined scale factor, and distances in
  inverses, listings and plans shown as grid or ground.
- [ ] **Project coordinate system in exports:** carried into LandXML, DXF metadata and COPC.

**Done when:** a GNSS job can be calibrated to local control and its points reported as grid or ground with
geoid heights.

## Phase 12 — Point cloud survey tools (about 4 weeks)

The streaming viewer shows large clouds well, but surveyors need to measure from them; Carlson 2025 and TBC
compete on classification and extraction.

- [ ] **Snap to the full cloud:** search the nodes currently loaded, not the 2 million point overview, so
  corners, kerbs and pegs can be picked.
- [ ] **Colour by classification and intensity,** and filter by class (ground only, buildings hidden).
- [ ] **Ground classification** for clouds without classes, as the base for surfaces from drone data.
- [ ] **Sections through clouds** and clipping boxes, alongside the existing surface sections.
- [ ] **Shared point budget** across all visible clouds (today each cloud draws up to 3 million points).
- [ ] **Relink moved files** with a locate dialog and a content check.
- [ ] **Point size:** fix the overlap in dense areas properly so the 7 pt default is no longer needed.

**Done when:** a drone cloud can be ground-classified, turned into a surface, and its features picked and
sectioned at full resolution.

## Phase 13 — Imagery and data exchange (about 3 weeks)

Every comparable package draws imagery under the survey, and drone jobs deliver an orthophoto alongside the
cloud.

- [ ] **Georeferenced orthophotos** (GeoTIFF, including cloud-optimised GeoTIFF) streamed under the drawing
  and on plan sheets.
- [ ] **Web basemaps** (XYZ or WMTS tiles) for context, off by default for privacy and offline work.
- [ ] **Shapefile** import and export for GIS clients.
- [ ] **DWG** import and export, the format most clients and councils ask for (DXF is supported today).

**Done when:** a drone orthophoto and a council's DWG base can be loaded under the survey and plotted.

## Later — Design and remaining gaps

Design is where 12d and Carlson Civil are deepest; it is months of work and worth starting only if
construction design, not just survey, is in scope.

- **Vertical profiles:** vertical alignments with grades and vertical curves on top of the existing horizontal
  alignments and long sections.
- **Corridors and templates:** road and channel cross-section templates swept along an alignment, with
  surfaces and volumes from them.
- **Grading:** pads, batters and daylight lines to an existing surface.
- **Surface tools:** interactive TIN editing (swap edges, delete triangles) and slope and drainage analysis.
- **Legal descriptions:** metes-and-bounds text written from a parcel.
- **GNSS baseline processing:** TBC's speciality; better left to the receiver vendors' tools.
- **COPC on the web build,** if web users need point clouds.

## Risks, dependencies and open decisions

The biggest risk is GPU portability: the point cloud renderer has only run on one Mac, and it gates Phase 12.

| Item | Kind | Why it matters | Affects |
| --- | --- | --- | --- |
| Viewer untested off macOS | Risk | The cloud shader reads a storage buffer in the vertex stage, which some drivers limit | Phases 8 and 12 |
| Raw format specifications | Dependency | RW5, SDR and Topcon formats need sample files from real controllers to test against | Phase 10 |
| Geoid grids | Dependency | Grids are large, licensed per country, and must be shipped or downloaded | Phase 11 |
| DWG library | Dependency | No mature open-source DWG writer in Rust; the options are the ODA SDK (licensed) or a converter | Phase 13 |
| Target market | Decision | Which country's cadastral conventions, geoids and plan standards come first sets Phases 9 and 11 | Phases 9 and 11 |
| Construction design | Decision | Whether vertical design and corridors are in scope at all | Later |
| Web build | Decision | Whether point clouds and imagery must work in the browser, which rules out native-only code paths | Phases 12 and 13 |

## Sources

- [Carlson SURVEY brochure](https://www.carlsonsw.com/api/wp-content/uploads/011_Carlson_SURVEY.pdf) and
  [Carlson Land Development catalog](https://www.carlsonsw.com/api/wp-content/uploads/CarlsonLandDevCatalog-web.pdf)
- [Carlson Survey listing, SoftwareOne](https://platform.softwareone.com/product/carlson-survey/PCP-7520-6735)
- [Carlson 2025 release, IntelliCAD](https://intellicad.org/articles-and-press-releases/topic/survey)
- [Carlson SurvPC: Manual Traverse](https://help.carlsonsw.com/en/survpc/main-menu/cogo/manual-traverse/manual-traverse.html)
  and [Sideshot](https://help.carlsonsw.com/en/survpc/map-screen/cogo/sideshot/sideshot.html)
- [Trimble Business Center](https://geospatial.trimble.com/products/software/trimble-business-center)
- [Hands on with Trimble Business Center, GeoWeek News](https://www.geoweeknews.com/articles/hands-w-trimble-business-center-surprisingly-good-point-cloud-tool-survey/)
- [What is MicroSurvey CAD?](https://www.microsurvey.com/?p=222804) and
  [MicroSurvey cadastral solutions](https://www.microsurvey.com/solutions/cadastral/cadastral-standalone-cad-solutions/)
- [MicroSurvey CAD, CR Kennedy](https://survey.crkennedy.com.au/products/sw-cad-ult/microsurvey-cad-2022-ultimate)
- [Autodesk Civil 3D: create a traverse](https://help.autodesk.com/cloudhelp/2024/ENG/Civil3D-UserGuide/files/GUID-F1A2B934-E79F-41A0-A10B-63F411AB60ED.htm)
- survey3d capabilities: the survey3d codebase as of the date above.
