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
| `survey-core` | Pure maths: angles/bearings, conventions (parse/format), geometry, arcs (bulges), COGO, point input |
| `survey-model` | `Document`, entities, layers, `Command`/`ChangeSet`, undo `History` with audit records, R-tree index |
| `survey-io` | Interchange formats: CSV/TXT points (configurable columns), DXF read/write |
| `survey-store` | `.s3d` project files (SQLite): incremental saves, migrations, hash-chained append-only audit trail |
| `cad-render` | wgpu renderer: f64 camera, relative-to-center chunks, reverse-Z, offscreen MSAA, egui paint callback |
| `egui-cad` | Reusable CAD widgets: theme, viewport, snapping, box selection, label placement, command line, panels |
| `survey-app` | The application: tools, panels, dialogs, file handling |

## Using it

* **Navigate:** wheel/pinch zooms about the cursor, middle-drag (or Shift+right-drag) pans, right-drag orbits in 3D.
  The toolbar cube (or `plan` / `3d`) toggles the plan lock; returning to plan is always north-up.
  `persp` (or View menu) toggles perspective.
* **Commands:** start typing anywhere — `point`, `pl` (polyline), `arc`, `circle`, `inv` (inverse), `ze`,
  `undo`, `help`.
* **Selection:** click, Shift+click to add; drag left→right for a window (fully inside), right→left for a
  crossing (touching).
* **Point input:** pick with snapping, or type `E,N[,Z]`, `@dE,dN[,dZ]`, `@dist<bearing`, or an existing point name.
  Order, units and angle format follow the project conventions (Project → Conventions).
* **Snapping & drafting:** Settings (gear) → Drafting holds snap modes, aperture, ortho/polar, grid and working
  elevation. Shortcuts: F3 snap, F8 ortho, F7 grid. Tab cycles overlapping candidates; typing
  `end`/`mid`/`int`/`per`/`nea`/`nod`/`gri` applies a one-shot snap. The status bar shows feedback only.
* **Files:** Ctrl+S saves an `.s3d` (download on the web); File → Import points (CSV/TXT), Import / Export
  DXF. On the web, unsaved work is autosaved to browser storage and offered for recovery on the next visit.
* **Labels:** Settings → Display (or `labels`) shows point names, codes and heights.
  Project → Audit trail shows every edit and verifies the hash chain.
