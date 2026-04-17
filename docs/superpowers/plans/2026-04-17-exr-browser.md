# EXR Browser — Right Panel Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the right panel of the main window with (a) file-action buttons (copy path, copy file to location), (b) an EXR browser section that includes thumbnail-size controls and a folder tree, and convert the bottom thumbnail strip into a grid with adaptive column count and larger resize range.

**Architecture:** Five phases ordered by risk (lowest first). Phase 1 wires file-action buttons; Phase 2 parameterises thumbnail size; Phase 3 introduces a new folder-tree model and component; Phase 4 rebuilds the thumbnail panel as a grid with extended resize. Each task is a single commit with `cargo build` + `cargo test` + a manual smoke test (run `cargo run --bin EXruster`, exercise the new UI) as acceptance gates.

**Tech Stack:** Rust (nightly, portable_simd), Slint 1.12, `exr` 1.73, `rayon` 1.11, `rfd` 0.15, `arboard` 3.x (new), `lru` 0.16. UI in `ui/*.slint`, glue in `src/ui/*.rs`.

---

## Background — Current Right-Panel Shape

Read [ui/appwindow.slint](ui/appwindow.slint) before starting — the right panel is Column 3 (`Rectangle { width: (root.width - _non_column_width) * _n3; visible: show-right-panel; ... }` around [ui/appwindow.slint:523-848](ui/appwindow.slint#L523-L848)). Its inner `VerticalBox` owns Exposure/Gamma sliders, Tonemap buttons, Reset, Export options, and the "Export Beauty … Export Lights" stack (lines 782–834). New content goes **below** the Export button stack and **before** the bottom `Rectangle { x: parent.width - 1px; ... }` overlay (line ~842).

The bottom thumbnail panel (`thumbs_panel`) is defined at [ui/appwindow.slint:852-1192](ui/appwindow.slint#L852-L1192). It uses a horizontal `ScrollView` + `HorizontalLayout` today; we will swap the inner layout for a grid and raise the resize ceiling.

Thumbnail generation lives in [src/ui/thumbnails.rs](src/ui/thumbnails.rs) and [src/io/thumbnails.rs](src/io/thumbnails.rs). The constant `THUMBNAIL_HEIGHT = 130` at [src/ui/thumbnails.rs:11](src/ui/thumbnails.rs#L11) must become configurable.

The left layer tree ([ui/appwindow.slint:363-414](ui/appwindow.slint#L363-L414)) is the style reference for the folder tree: 18px rows, hover background `Kolory.suwak_tlo`, selected background `Kolory.hover`, 📂/📁 emoji prefix, depth via left-padding. Match that styling in [ui/components/FolderTree.slint](ui/components/FolderTree.slint).

---

## File Structure

Files this plan touches, and their post-plan responsibility:

- **Modify:** `Cargo.toml` — add `arboard = "3"`.
- **Modify:** [ui/appwindow.slint](ui/appwindow.slint) — add properties, callbacks, new UI blocks in right panel, rework `thumbs_panel` to grid, raise resize ceiling.
- **Create:** [ui/components/FolderTree.slint](ui/components/FolderTree.slint) — reusable folder-tree widget styled after left layers list.
- **Create:** [src/io/folder_tree.rs](src/io/folder_tree.rs) — directory scanning + flat tree model (depth, has_children, expanded flags).
- **Modify:** [src/io/mod.rs](src/io/mod.rs) — export `folder_tree`.
- **Create:** [src/ui/browser_handlers.rs](src/ui/browser_handlers.rs) — Rust callbacks for clipboard copy, copy-to-location, thumbnail size change, folder tree navigation/toggle.
- **Modify:** [src/ui/mod.rs](src/ui/mod.rs) — declare `browser_handlers`.
- **Modify:** [src/ui/state.rs](src/ui/state.rs) — store `folder_tree_root`, `expanded_folders` (`HashSet<PathBuf>`), `current_browsed_folder`.
- **Modify:** [src/ui/thumbnails.rs](src/ui/thumbnails.rs) — accept `thumbnail_height` parameter instead of const.
- **Modify:** [src/ui/file_handlers.rs](src/ui/file_handlers.rs) — push `current-file-path` into UI state on open.
- **Modify:** [src/ui/setup.rs](src/ui/setup.rs) — wire new callbacks.

**No changes:** tone mapping, export handlers, caches (`image_cache.rs`, `full_exr_cache.rs`), SIMD code.

---

# Phase 1 — File Action Buttons (Copy Path, Copy File To)

### Task 1.1: Add `arboard` dependency

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add arboard to dependencies**

In [Cargo.toml](Cargo.toml), inside the `[dependencies]` block (below `dashmap = "6.1"`), add:

```toml
arboard = "3"
```

- [ ] **Step 2: Verify build**

Run: `cargo build`
Expected: builds successfully, `arboard` compiles as a transitive dependency.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "build: add arboard for clipboard access"
```

---

### Task 1.2: Expose `current-file-path` property + copy callbacks in Slint

**Files:**
- Modify: [ui/appwindow.slint](ui/appwindow.slint)

- [ ] **Step 1: Add property and callbacks near other in-out properties**

Find the block of `callback` declarations around [ui/appwindow.slint:131-156](ui/appwindow.slint#L131-L156). Just above `callback histogram-requested();` (line ~156) add:

```slint
    // File action state
    in-out property <string> current-file-path: "";
    callback copy-current-path();
    callback copy-current-file-to-location();
```

- [ ] **Step 2: Add separator + two buttons after Export buttons**

Locate the closing `}` of the "Export buttons" `Rectangle` (the one containing the 6 `PrzyciskAkcji` export buttons) at ~[ui/appwindow.slint:782-834](ui/appwindow.slint#L782-L834). Immediately after that closing `}` and **before** the outer `}` that closes the right-panel `VerticalBox`, insert:

```slint
                 // Separator
                 Rectangle {
                     width: parent.width - controls_inset*2;
                     height: 1px;
                     background: Kolory.linia_podzialu;
                 }

                 Rectangle { height: 6px; }

                 // File action buttons
                 Rectangle {
                     width: parent.width;
                     height: 60px;
                     VerticalBox {
                         width: parent.width;
                         height: parent.height;
                         spacing: 4px;
                         alignment: start;

                         PrzyciskAkcji {
                             text: "Copy File Path";
                             enabled: root.current-file-path != "";
                             height: 26px;
                             width: parent.width - controls_inset*2;
                             clicked => { root.copy-current-path(); }
                         }

                         PrzyciskAkcji {
                             text: "Copy File To...";
                             enabled: root.current-file-path != "";
                             height: 26px;
                             width: parent.width - controls_inset*2;
                             clicked => { root.copy-current-file-to-location(); }
                         }
                     }
                 }
```

- [ ] **Step 3: Rebuild to regenerate Slint bindings**

Run: `cargo build`
Expected: compiles; generated bindings now expose `set_current_file_path`, `on_copy_current_path`, `on_copy_current_file_to_location`.

- [ ] **Step 4: Commit**

```bash
git add ui/appwindow.slint
git commit -m "feat(ui): add copy-path and copy-file buttons to right panel"
```

---

### Task 1.3: Implement copy-path / copy-file handlers in Rust

**Files:**
- Create: `src/ui/browser_handlers.rs`
- Modify: `src/ui/mod.rs`

- [ ] **Step 1: Create the new module**

Create [src/ui/browser_handlers.rs](src/ui/browser_handlers.rs):

```rust
use crate::ui::state::SharedAppState;
use crate::ui::ui_handlers::{push_console, ConsoleModel};
use crate::{log_error, log_warn, AppWindow};
use slint::{ComponentHandle, Weak};
use std::path::PathBuf;

/// Copy the absolute path of the currently open EXR to the system clipboard.
pub fn handle_copy_current_path(
    ui_handle: Weak<AppWindow>,
    app_state: SharedAppState,
    console: ConsoleModel,
) {
    let Some(ui) = ui_handle.upgrade() else { return; };

    let path_opt = app_state
        .read()
        .ok()
        .and_then(|s| s.current_file_path.clone());

    let Some(path) = path_opt else {
        ui.set_status_text("No file open — nothing to copy".into());
        push_console(&ui, &console, "[copy-path] no file open".to_string());
        return;
    };

    let text = path.display().to_string();
    match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text.clone())) {
        Ok(()) => {
            ui.set_status_text(format!("Path copied: {}", text).into());
            push_console(&ui, &console, format!("[copy-path] copied {}", text));
        }
        Err(e) => {
            log_error!("Clipboard error: {}", e);
            ui.set_status_text(format!("Clipboard error: {}", e).into());
        }
    }
}

/// Prompt the user for a destination folder and copy the currently open EXR there.
pub fn handle_copy_current_file_to(
    ui_handle: Weak<AppWindow>,
    app_state: SharedAppState,
    console: ConsoleModel,
) {
    let Some(ui) = ui_handle.upgrade() else { return; };

    let src_opt: Option<PathBuf> = app_state
        .read()
        .ok()
        .and_then(|s| s.current_file_path.clone());

    let Some(src) = src_opt else {
        ui.set_status_text("No file open — nothing to copy".into());
        return;
    };

    let Some(dest_dir) = rfd::FileDialog::new()
        .set_title("Select destination folder")
        .pick_folder()
    else {
        push_console(&ui, &console, "[copy-file] canceled".to_string());
        return;
    };

    let file_name = match src.file_name() {
        Some(n) => n.to_owned(),
        None => {
            log_warn!("Source path has no file name: {}", src.display());
            return;
        }
    };
    let dest = dest_dir.join(&file_name);

    match std::fs::copy(&src, &dest) {
        Ok(bytes) => {
            ui.set_status_text(format!("Copied {} bytes to {}", bytes, dest.display()).into());
            push_console(
                &ui,
                &console,
                format!("[copy-file] {} -> {}", src.display(), dest.display()),
            );
        }
        Err(e) => {
            log_error!("Copy failed: {}", e);
            ui.set_status_text(format!("Copy failed: {}", e).into());
        }
    }
}
```

- [ ] **Step 2: Register the module**

Modify [src/ui/mod.rs](src/ui/mod.rs). After the `pub mod thumbnails;` line, add:

```rust
pub mod browser_handlers;
```

- [ ] **Step 3: Build**

Run: `cargo build`
Expected: compiles (handlers unused — warnings OK for now).

- [ ] **Step 4: Commit**

```bash
git add src/ui/browser_handlers.rs src/ui/mod.rs
git commit -m "feat(ui): add copy-path and copy-file browser handlers"
```

---

### Task 1.4: Wire callbacks + populate `current-file-path` on open

**Files:**
- Modify: `src/ui/setup.rs`
- Modify: `src/ui/file_handlers.rs`

- [ ] **Step 1: Push file path into UI from `handle_open_exr_from_path`**

Open [src/ui/file_handlers.rs](src/ui/file_handlers.rs). At line ~97, inside the `match load_metadata(&ui, &path, &console) { Ok(()) => { ... } }` arm, locate:

```rust
                if let Ok(mut state) = app_state.write() {
                    state.current_file_path = Some(path.clone());
                }
```

Immediately **after** that block, add:

```rust
                ui.set_current_file_path(path.display().to_string().into());
```

- [ ] **Step 2: Wire clipboard / copy-to callbacks in setup**

Open [src/ui/setup.rs](src/ui/setup.rs). Inside `setup_panel_callbacks` (around line ~246), at the **end** of the function (before the closing `}` at line ~431), add:

```rust
    ui.on_copy_current_path({
        let ui_handle = ui.as_weak();
        let app_state = Arc::clone(&app_state);
        let console_model = console_model.clone();
        move || {
            crate::ui::browser_handlers::handle_copy_current_path(
                ui_handle.clone(),
                app_state.clone(),
                console_model.clone(),
            );
        }
    });

    ui.on_copy_current_file_to_location({
        let ui_handle = ui.as_weak();
        let app_state = Arc::clone(&app_state);
        let console_model = console_model.clone();
        move || {
            crate::ui::browser_handlers::handle_copy_current_file_to(
                ui_handle.clone(),
                app_state.clone(),
                console_model.clone(),
            );
        }
    });
```

- [ ] **Step 3: Build**

Run: `cargo build`
Expected: clean build. The `Copy File Path` and `Copy File To...` buttons become enabled after an EXR opens.

- [ ] **Step 4: Smoke test**

Run: `cargo run --bin EXruster`. Open an EXR via File → Open EXR. Verify:
- Path appears in status bar via `Copy File Path` (check clipboard with paste in another app)
- `Copy File To...` opens a folder picker and copies the file

- [ ] **Step 5: Commit**

```bash
git add src/ui/file_handlers.rs src/ui/setup.rs
git commit -m "feat(ui): wire copy-path/copy-file buttons to file state"
```

---

# Phase 2 — Thumbnail Size Selector

### Task 2.1: Parameterise thumbnail height

**Files:**
- Modify: `src/ui/thumbnails.rs`

- [ ] **Step 1: Replace the constant with a parameter**

In [src/ui/thumbnails.rs](src/ui/thumbnails.rs), delete line 11:

```rust
const THUMBNAIL_HEIGHT: u32 = 130;
```

Change the signature of `load_thumbnails_for_directory` (line ~16) from:

```rust
pub fn load_thumbnails_for_directory(
    ui_handle: Weak<AppWindow>,
    directory: &Path,
    console: ConsoleModel,
) {
```

to:

```rust
pub fn load_thumbnails_for_directory(
    ui_handle: Weak<AppWindow>,
    directory: &Path,
    console: ConsoleModel,
    thumbnail_height: u32,
) {
```

Inside the body, change the `generate_thumbnails_cpu_raw` call (line ~89) from `THUMBNAIL_HEIGHT` to `thumbnail_height`.

- [ ] **Step 2: Update call sites**

Grep for callers: `grep -rn "load_thumbnails_for_directory" src/`. Each call site (in [src/ui/setup.rs](src/ui/setup.rs) and [src/main.rs](src/main.rs) if present) needs a height argument. Use `130` at every existing call site for now (baseline behaviour preserved):

Example in [src/ui/setup.rs:278](src/ui/setup.rs#L278):

```rust
                    crate::ui::load_thumbnails_for_directory(
                        ui.as_weak(),
                        &dir,
                        console_model.clone(),
                        130,
                    );
```

And similarly at [src/ui/setup.rs:411](src/ui/setup.rs#L411) (inside `on_delete_thumbnail`). Pass `130`.

Check [src/main.rs](src/main.rs) for any call — pass `130` there too.

- [ ] **Step 3: Build**

Run: `cargo build`
Expected: clean build.

- [ ] **Step 4: Smoke test**

Run: `cargo run --bin EXruster`. Open a working folder — thumbnails still render at 130px height (no visible change).

- [ ] **Step 5: Commit**

```bash
git add src/ui/thumbnails.rs src/ui/setup.rs src/main.rs
git commit -m "refactor(thumbnails): make thumbnail height configurable"
```

---

### Task 2.2: Add "EXR Browser" label + 3 size buttons in Slint

**Files:**
- Modify: `ui/appwindow.slint`

- [ ] **Step 1: Add thumbnail-size property + callback**

In [ui/appwindow.slint](ui/appwindow.slint), next to the `current-file-path` / `copy-current-path` declarations added in Task 1.2, add:

```slint
    // EXR browser state
    // 1 = small (130), 2 = medium (260), 3 = large (390)
    in-out property <int> thumbnail-size-level: 1;
    callback thumbnail-size-changed(int);
```

- [ ] **Step 2: Add EXR Browser section below file action buttons**

Just **after** the "File action buttons" `Rectangle` added in Task 1.2, add:

```slint
                 Rectangle { height: 10px; }

                 // EXR Browser label
                 Rectangle {
                     width: parent.width;
                     height: 20px;
                     Text {
                         text: "EXR Browser";
                         x: 5px;
                         y: 3px;
                         width: parent.width;
                         height: parent.height;
                         color: Kolory.tekst;
                         font-size: 13px;
                         font-family: "Geist";
                         font-weight: 300;
                         horizontal-alignment: left;
                         vertical-alignment: center;
                     }
                 }

                 // Size buttons row (3 square buttons)
                 tb_size_row := HorizontalBox {
                     width: parent.width;
                     height: 28px;
                     spacing: 6px;
                     PrzyciskAkcji {
                         text: "×1";
                         height: 28px;
                         width: 28px;
                         highlighted: root.thumbnail-size-level == 1;
                         clicked => { root.thumbnail-size-level = 1; root.thumbnail-size-changed(1); }
                     }
                     PrzyciskAkcji {
                         text: "×2";
                         height: 28px;
                         width: 28px;
                         highlighted: root.thumbnail-size-level == 2;
                         clicked => { root.thumbnail-size-level = 2; root.thumbnail-size-changed(2); }
                     }
                     PrzyciskAkcji {
                         text: "×3";
                         height: 28px;
                         width: 28px;
                         highlighted: root.thumbnail-size-level == 3;
                         clicked => { root.thumbnail-size-level = 3; root.thumbnail-size-changed(3); }
                     }
                 }
```

- [ ] **Step 3: Build and visually verify**

Run: `cargo run --bin EXruster`
Expected: right panel shows "EXR Browser" label and three highlightable square buttons.

- [ ] **Step 4: Commit**

```bash
git add ui/appwindow.slint
git commit -m "feat(ui): add EXR Browser section with size selector"
```

---

### Task 2.3: Wire size buttons to reload thumbnails

**Files:**
- Modify: `src/ui/browser_handlers.rs`
- Modify: `src/ui/setup.rs`
- Modify: `src/ui/state.rs`

- [ ] **Step 1: Track last browsed folder in state**

In [src/ui/state.rs](src/ui/state.rs), add to `AppState`:

```rust
    pub current_browsed_folder: Option<PathBuf>,
```

so the struct becomes:

```rust
#[derive(Default)]
pub struct AppState {
    pub image_cache: Option<ImageCache>,
    pub current_file_path: Option<PathBuf>,
    pub full_exr_cache: Option<Arc<FullExrCacheData>>,
    pub ui_state: UiState,
    pub channel_config: Option<ChannelGroupConfig>,
    pub current_browsed_folder: Option<PathBuf>,
}
```

- [ ] **Step 2: Store folder on thumbnail load**

In [src/ui/setup.rs](src/ui/setup.rs), inside `on_choose_working_folder` (line ~267), before the call to `crate::ui::load_thumbnails_for_directory`, add:

```rust
                    if let Ok(mut state) = app_state.write() {
                        state.current_browsed_folder = Some(dir.clone());
                    }
```

Also include `let app_state = Arc::clone(&app_state);` in the closure capture if not present (check the surrounding move closure signature).

- [ ] **Step 3: Add size-change handler**

Append to [src/ui/browser_handlers.rs](src/ui/browser_handlers.rs):

```rust
pub fn size_level_to_height(level: i32) -> u32 {
    match level {
        2 => 260,
        3 => 390,
        _ => 130,
    }
}

pub fn handle_thumbnail_size_changed(
    ui_handle: Weak<AppWindow>,
    app_state: SharedAppState,
    console: ConsoleModel,
    level: i32,
) {
    let Some(ui) = ui_handle.upgrade() else { return; };
    let height = size_level_to_height(level);

    let folder_opt = app_state
        .read()
        .ok()
        .and_then(|s| s.current_browsed_folder.clone());

    let Some(folder) = folder_opt else {
        ui.set_status_text("No folder selected".into());
        return;
    };

    push_console(
        &ui,
        &console,
        format!("[browser] thumbnail size -> {}px ({})", height, folder.display()),
    );
    crate::ui::load_thumbnails_for_directory(ui.as_weak(), &folder, console, height);
}
```

- [ ] **Step 4: Wire callback in setup**

In [src/ui/setup.rs](src/ui/setup.rs), inside `setup_panel_callbacks`, add (near the copy callbacks from Task 1.4):

```rust
    ui.on_thumbnail_size_changed({
        let ui_handle = ui.as_weak();
        let app_state = Arc::clone(&app_state);
        let console_model = console_model.clone();
        move |level: i32| {
            crate::ui::browser_handlers::handle_thumbnail_size_changed(
                ui_handle.clone(),
                app_state.clone(),
                console_model.clone(),
                level,
            );
        }
    });
```

- [ ] **Step 5: Build + smoke test**

Run: `cargo run --bin EXruster`. Choose a working folder, then click ×2 and ×3 — thumbnails regenerate at larger sizes each click.

- [ ] **Step 6: Commit**

```bash
git add src/ui/browser_handlers.rs src/ui/setup.rs src/ui/state.rs
git commit -m "feat(ui): reload thumbnails on size-selector change"
```

---

# Phase 3 — Folder Tree

### Task 3.1: Add folder-tree model to Slint

**Files:**
- Modify: `ui/appwindow.slint`

- [ ] **Step 1: Declare struct + properties**

Near the existing `export struct ThumbItem` block ([ui/appwindow.slint:16-24](ui/appwindow.slint#L16-L24)), add:

```slint
export struct FolderEntry {
    display_name: string,
    path: string,
    depth: int,
    has_children: bool,
    expanded: bool,
    is_current: bool,
}
```

Inside `AppWindow` (near other `in-out property` declarations), add:

```slint
    in-out property <[FolderEntry]> folder-tree: [];
    callback folder-tree-row-clicked(string);  // path
    callback folder-tree-toggle(string);       // path
```

- [ ] **Step 2: Build**

Run: `cargo build`
Expected: bindings regenerate cleanly.

- [ ] **Step 3: Commit**

```bash
git add ui/appwindow.slint
git commit -m "feat(ui): declare folder-tree model and callbacks"
```

---

### Task 3.2: Create FolderTree.slint component

**Files:**
- Create: `ui/components/FolderTree.slint`
- Modify: `ui/appwindow.slint`

- [ ] **Step 1: Write the component**

Create [ui/components/FolderTree.slint](ui/components/FolderTree.slint):

```slint
import { ScrollView, VerticalBox } from "std-widgets.slint";
import { Kolory } from "../colors.slint";

export struct FolderEntry {
    display_name: string,
    path: string,
    depth: int,
    has_children: bool,
    expanded: bool,
    is_current: bool,
}

export component FolderTree inherits Rectangle {
    in property <[FolderEntry]> entries: [];
    callback row-clicked(string);
    callback toggle-clicked(string);

    background: Kolory.przezroczysty;
    clip: true;

    ScrollView {
        width: parent.width;
        height: parent.height;

        VerticalBox {
            spacing: 1px;
            alignment: start;
            padding: 0px;

            for entry[idx] in root.entries: Rectangle {
                height: 18px;
                width: parent.width;
                clip: true;
                background: entry.is_current ? Kolory.hover
                            : (row_hover.has-hover ? Kolory.suwak_tlo : Kolory.przezroczysty);

                // Toggle arrow (only if has-children)
                Rectangle {
                    x: (entry.depth * 10px) + 2px;
                    y: 0px;
                    width: 12px;
                    height: parent.height;
                    Text {
                        text: entry.has_children ? (entry.expanded ? "▼" : "▶") : " ";
                        color: Kolory.tekst_slabszy;
                        font-size: 9px;
                        font-family: "Geist";
                        vertical-alignment: center;
                        horizontal-alignment: center;
                        width: parent.width;
                        height: parent.height;
                    }
                    TouchArea {
                        width: parent.width;
                        height: parent.height;
                        clicked => { if (entry.has_children) { root.toggle-clicked(entry.path); } }
                    }
                }

                // Folder name + icon
                Text {
                    text: "📂 " + entry.display_name;
                    color: entry.is_current ? Kolory.tekst_silny : Kolory.tekst;
                    font-size: 11px;
                    font-family: "Geist";
                    font-weight: entry.is_current ? 700 : 400;
                    vertical-alignment: center;
                    horizontal-alignment: left;
                    x: (entry.depth * 10px) + 18px;
                    width: parent.width - self.x - 4px;
                    wrap: no-wrap;
                }

                row_hover := TouchArea {
                    x: (entry.depth * 10px) + 16px;
                    width: parent.width - self.x;
                    height: parent.height;
                    clicked => { root.row-clicked(entry.path); }
                }
            }
        }
    }
}
```

- [ ] **Step 2: Import component in appwindow.slint**

In [ui/appwindow.slint](ui/appwindow.slint), near the top imports (line ~12), add:

```slint
import { FolderTree } from "components/FolderTree.slint";
```

**Also delete** the `export struct FolderEntry` block added in Task 3.1 from `appwindow.slint` — it now lives inside `FolderTree.slint` and Slint will share the definition once the component is imported. If you get a duplicate-struct error at build, keep the one inside the component and remove the one at the top of `appwindow.slint`.

- [ ] **Step 3: Mount component inside right panel (below size buttons)**

After the `tb_size_row` block added in Task 2.2, add:

```slint
                 // Folder tree container (takes remaining vertical space)
                 Rectangle {
                     width: parent.width;
                     height: 300px;
                     background: Kolory.przezroczysty;
                     border-color: Kolory.linia_podzialu;
                     border-width: 1px;

                     FolderTree {
                         width: parent.width - 2px;
                         height: parent.height - 2px;
                         x: 1px;
                         y: 1px;
                         entries: root.folder-tree;
                         row-clicked(p) => { root.folder-tree-row-clicked(p); }
                         toggle-clicked(p) => { root.folder-tree-toggle(p); }
                     }
                 }
```

- [ ] **Step 4: Build**

Run: `cargo build`
Expected: clean build. Empty folder tree renders (no entries yet).

- [ ] **Step 5: Commit**

```bash
git add ui/components/FolderTree.slint ui/appwindow.slint
git commit -m "feat(ui): add FolderTree component in right panel"
```

---

### Task 3.3: Directory-scanning module

**Files:**
- Create: `src/io/folder_tree.rs`
- Modify: `src/io/mod.rs`

- [ ] **Step 1: Implement scanner**

Create [src/io/folder_tree.rs](src/io/folder_tree.rs):

```rust
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct FolderNode {
    pub path: PathBuf,
    pub display_name: String,
    pub depth: i32,
    pub has_children: bool,
}

/// List immediate subdirectories of `dir` (no recursion).
/// Skips hidden entries (`.` prefix) and unreadable entries.
pub fn list_subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new(); };
    let mut out: Vec<PathBuf> = rd
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|s| !s.starts_with('.'))
                .unwrap_or(false)
        })
        .collect();
    out.sort_by(|a, b| {
        a.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase()
            .cmp(
                &b.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_lowercase(),
            )
    });
    out
}

pub fn has_subdirs(dir: &Path) -> bool {
    list_subdirs(dir).into_iter().next().is_some()
}

fn display_name(p: &Path) -> String {
    p.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| p.display().to_string())
}

/// Build a flat, depth-ordered tree from `root`, expanding only those paths
/// present in `expanded`. Each node carries its depth and a `has_children`
/// flag (cheap filesystem check).
pub fn build_flat_tree(
    root: &Path,
    expanded: &std::collections::HashSet<PathBuf>,
) -> Vec<FolderNode> {
    let mut out = Vec::new();
    fn walk(
        dir: &Path,
        depth: i32,
        expanded: &std::collections::HashSet<PathBuf>,
        out: &mut Vec<FolderNode>,
    ) {
        let subs = list_subdirs(dir);
        let has_children = !subs.is_empty();
        out.push(FolderNode {
            path: dir.to_path_buf(),
            display_name: display_name(dir),
            depth,
            has_children,
        });
        if expanded.contains(dir) {
            for s in subs {
                walk(&s, depth + 1, expanded, out);
            }
        }
    }
    walk(root, 0, expanded, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_subdirs_alphabetically() {
        let tmp = std::env::temp_dir().join(format!("exruster-ft-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("bravo")).unwrap();
        std::fs::create_dir_all(tmp.join("alpha")).unwrap();
        let subs = list_subdirs(&tmp);
        assert_eq!(
            subs.iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
                .collect::<Vec<_>>(),
            vec!["alpha".to_string(), "bravo".to_string()]
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
```

- [ ] **Step 2: Register module**

In [src/io/mod.rs](src/io/mod.rs), add (anywhere in the `pub mod` list):

```rust
pub mod folder_tree;
```

- [ ] **Step 3: Run test**

Run: `cargo test folder_tree -- --nocapture`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/io/folder_tree.rs src/io/mod.rs
git commit -m "feat(io): add folder_tree scanning module"
```

---

### Task 3.4: Folder-tree state + UI refresh helper

**Files:**
- Modify: `src/ui/state.rs`
- Modify: `src/ui/browser_handlers.rs`

- [ ] **Step 1: Extend `AppState`**

In [src/ui/state.rs](src/ui/state.rs):

```rust
use std::collections::{HashMap, HashSet};
```

Then add to the struct:

```rust
    pub folder_tree_root: Option<PathBuf>,
    pub folder_tree_expanded: HashSet<PathBuf>,
```

Result:

```rust
#[derive(Default)]
pub struct AppState {
    pub image_cache: Option<ImageCache>,
    pub current_file_path: Option<PathBuf>,
    pub full_exr_cache: Option<Arc<FullExrCacheData>>,
    pub ui_state: UiState,
    pub channel_config: Option<ChannelGroupConfig>,
    pub current_browsed_folder: Option<PathBuf>,
    pub folder_tree_root: Option<PathBuf>,
    pub folder_tree_expanded: HashSet<PathBuf>,
}
```

- [ ] **Step 2: Add refresh helper in browser_handlers**

Append to [src/ui/browser_handlers.rs](src/ui/browser_handlers.rs):

```rust
use slint::{Model, ModelRc, SharedString, VecModel};

/// Rebuild the Slint `folder-tree` model from the current `AppState`.
/// Safe to call from the main thread at any time.
pub fn refresh_folder_tree(ui: &AppWindow, app_state: &SharedAppState) {
    let (root, expanded, current) = {
        let Ok(s) = app_state.read() else {
            ui.set_folder_tree(ModelRc::new(VecModel::from(Vec::<crate::FolderEntry>::new())));
            return;
        };
        (
            s.folder_tree_root.clone(),
            s.folder_tree_expanded.clone(),
            s.current_browsed_folder.clone(),
        )
    };

    let Some(root) = root else {
        ui.set_folder_tree(ModelRc::new(VecModel::from(Vec::<crate::FolderEntry>::new())));
        return;
    };

    let nodes = crate::io::folder_tree::build_flat_tree(&root, &expanded);
    let entries: Vec<crate::FolderEntry> = nodes
        .into_iter()
        .map(|n| crate::FolderEntry {
            display_name: SharedString::from(n.display_name),
            path: SharedString::from(n.path.display().to_string()),
            depth: n.depth,
            has_children: n.has_children,
            expanded: expanded.contains(&n.path),
            is_current: current.as_deref() == Some(n.path.as_path()),
        })
        .collect();
    ui.set_folder_tree(ModelRc::new(VecModel::from(entries)));
}
```

- [ ] **Step 3: Build**

Run: `cargo build`
Expected: clean build (unused-function warnings acceptable).

- [ ] **Step 4: Commit**

```bash
git add src/ui/state.rs src/ui/browser_handlers.rs
git commit -m "feat(ui): add folder-tree state and refresh helper"
```

---

### Task 3.5: Wire toggle + navigation + initial tree on working-folder choice

**Files:**
- Modify: `src/ui/browser_handlers.rs`
- Modify: `src/ui/setup.rs`

- [ ] **Step 1: Add toggle + navigate handlers**

Append to [src/ui/browser_handlers.rs](src/ui/browser_handlers.rs):

```rust
pub fn handle_folder_toggle(
    ui_handle: Weak<AppWindow>,
    app_state: SharedAppState,
    path_str: String,
) {
    let path = PathBuf::from(&path_str);
    if let Ok(mut s) = app_state.write() {
        if s.folder_tree_expanded.contains(&path) {
            s.folder_tree_expanded.remove(&path);
        } else {
            s.folder_tree_expanded.insert(path);
        }
    }
    if let Some(ui) = ui_handle.upgrade() {
        refresh_folder_tree(&ui, &app_state);
    }
}

pub fn handle_folder_row_click(
    ui_handle: Weak<AppWindow>,
    app_state: SharedAppState,
    console: ConsoleModel,
    path_str: String,
) {
    let Some(ui) = ui_handle.upgrade() else { return; };
    let path = PathBuf::from(&path_str);
    if !path.is_dir() {
        return;
    }

    {
        let mut s = match app_state.write() {
            Ok(s) => s,
            Err(_) => return,
        };
        s.current_browsed_folder = Some(path.clone());
        s.folder_tree_expanded.insert(path.clone());
    }

    refresh_folder_tree(&ui, &app_state);

    let level = ui.get_thumbnail_size_level();
    let height = size_level_to_height(level);
    crate::ui::load_thumbnails_for_directory(ui.as_weak(), &path, console, height);
}
```

- [ ] **Step 2: Initialise tree on working-folder choice**

In [src/ui/setup.rs](src/ui/setup.rs), inside `on_choose_working_folder` (around line ~277), **replace** the current `if let Some(dir) = ...` block with:

```rust
                if let Some(dir) = crate::io::file_operations::open_folder_dialog() {
                    // Root the folder tree at the chosen folder's parent (so
                    // siblings are visible for navigation). Fall back to the
                    // chosen folder itself when no parent exists.
                    let tree_root = dir.parent().map(|p| p.to_path_buf()).unwrap_or(dir.clone());
                    if let Ok(mut state) = app_state.write() {
                        state.current_browsed_folder = Some(dir.clone());
                        state.folder_tree_root = Some(tree_root);
                        state.folder_tree_expanded.insert(dir.clone());
                    }
                    crate::ui::browser_handlers::refresh_folder_tree(&ui, &app_state);

                    let level = ui.get_thumbnail_size_level();
                    let height = crate::ui::browser_handlers::size_level_to_height(level);
                    crate::ui::load_thumbnails_for_directory(
                        ui.as_weak(),
                        &dir,
                        console_model.clone(),
                        height,
                    );
                } else {
                    push_console(
                        &ui,
                        &console_model,
                        "[folder] selection canceled".to_string(),
                    );
                }
```

- [ ] **Step 3: Wire the two callbacks**

At the end of `setup_panel_callbacks`, add:

```rust
    ui.on_folder_tree_toggle({
        let ui_handle = ui.as_weak();
        let app_state = Arc::clone(&app_state);
        move |path: slint::SharedString| {
            crate::ui::browser_handlers::handle_folder_toggle(
                ui_handle.clone(),
                app_state.clone(),
                path.to_string(),
            );
        }
    });

    ui.on_folder_tree_row_clicked({
        let ui_handle = ui.as_weak();
        let app_state = Arc::clone(&app_state);
        let console_model = console_model.clone();
        move |path: slint::SharedString| {
            crate::ui::browser_handlers::handle_folder_row_click(
                ui_handle.clone(),
                app_state.clone(),
                console_model.clone(),
                path.to_string(),
            );
        }
    });
```

- [ ] **Step 4: Build + smoke test**

Run: `cargo run --bin EXruster`. Click "Select working folder", pick a folder that has subdirectories. Verify:
- Folder tree populates in the right panel with ▼/▶ arrows
- Clicking ▶ expands a folder and lists its children
- Clicking a folder name loads thumbnails from that folder (bottom panel updates) and highlights the row in the tree

- [ ] **Step 5: Commit**

```bash
git add src/ui/browser_handlers.rs src/ui/setup.rs
git commit -m "feat(ui): wire folder-tree navigation and expansion"
```

---

# Phase 4 — Thumbnail Grid + Expanded Resize

### Task 4.1: Raise resize ceiling so panel can fill to preview edge

**Files:**
- Modify: `ui/appwindow.slint`

- [ ] **Step 1: Replace the 400px cap**

Find the bottom-panel resize handle at [ui/appwindow.slint:1183-1190](ui/appwindow.slint#L1183-L1190):

```slint
                moved => {
                    if (self.pressed) {
                        root.bottom-panel-expanded-height = max(80px, min(400px,
                            root.bottom-panel-expanded-height - (self.mouse-y - self.pressed-y)));
                    }
                }
```

Replace with (leave 30px menu + 24px status + 40px button bar + 30px margin = 124px of fixed chrome; max = window height minus that):

```slint
                moved => {
                    if (self.pressed) {
                        root.bottom-panel-expanded-height = max(80px, min(root.height - 124px,
                            root.bottom-panel-expanded-height - (self.mouse-y - self.pressed-y)));
                    }
                }
```

- [ ] **Step 2: Build + smoke test**

Run: `cargo run --bin EXruster`. Show the bottom panel (Space). Drag its top handle upward — it should now grow up to just below the preview image edge instead of stopping at 400px.

- [ ] **Step 3: Commit**

```bash
git add ui/appwindow.slint
git commit -m "feat(ui): allow thumbnail panel to resize up to preview edge"
```

---

### Task 4.2: Convert thumbnail strip to adaptive grid

**Files:**
- Modify: `ui/appwindow.slint`

- [ ] **Step 1: Replace the thumbnail layout**

Locate the `scroll_view := ScrollView { ... thumbs_content := HorizontalLayout { ... } }` block at [ui/appwindow.slint:959-1050](ui/appwindow.slint#L959-L1050). Replace the entire `scroll_view` declaration with the following (keeps the surrounding `viewport := Rectangle { ... }` as-is; only the `scroll_view :=` body changes):

```slint
            scroll_view := ScrollView {
                width: parent.width;
                height: parent.height;
                // Vertical scroll for grid layout
                viewport-width: parent.width;
                viewport-height: grid_container.preferred-height;

                grid_container := Rectangle {
                    width: parent.width;
                    // cell size driven by thumbnail-size-level (1=small, 2=medium, 3=large)
                    property <length> cell_size:
                        root.thumbnail-size-level == 3 ? 240px
                        : (root.thumbnail-size-level == 2 ? 180px : 130px);
                    property <length> cell_pad: 36px; // room for file name + size/layers text below image
                    property <length> spacing: 12px;
                    property <int>    cols: max(1, floor((parent.width + spacing) / (cell_size + spacing)));
                    property <length> cell_h: cell_size + cell_pad;

                    preferred-height: ceil((root.thumbnails.length * 1.0) / cols) * (cell_h + spacing);

                    for t[index] in root.thumbnails: Rectangle {
                        x: mod(index, grid_container.cols) * (grid_container.cell_size + grid_container.spacing) + grid_container.spacing / 2;
                        y: floor(index / grid_container.cols) * (grid_container.cell_h + grid_container.spacing) + grid_container.spacing / 2;
                        width: grid_container.cell_size;
                        height: grid_container.cell_h;
                        background: Kolory.przezroczysty;

                        VerticalBox {
                            padding: 0px;
                            spacing: 1.6px;
                            alignment: start;
                            height: parent.height;

                            image_frame := Rectangle {
                                width: parent.width;
                                height: grid_container.cell_size;
                                background: Kolory.przezroczysty;
                                border-width: (root.opened-thumbnail-path == t.path) ? 2px : (click_area.has-hover ? 2px : 0px);
                                border-color: (root.opened-thumbnail-path == t.path) ? #ffffff : root.hover;
                                clip: true;

                                Image {
                                    width: parent.width;
                                    height: parent.height;
                                    source: t.img;
                                    image-fit: contain;
                                    horizontal-alignment: center;
                                    vertical-alignment: center;
                                }

                                click_area := TouchArea {
                                    width: parent.width;
                                    height: parent.height;
                                    mouse-cursor: pointer;
                                    clicked => {
                                        root.opened-thumbnail-path = t.path;
                                        root.open-thumbnail(t.path);
                                    }
                                }
                            }

                            desc := Rectangle {
                                width: parent.width;
                                height: 36px;
                                clip: true;
                                background: Kolory.przezroczysty;
                                VerticalBox {
                                    width: parent.width;
                                    height: parent.height;
                                    spacing: 0px;
                                    alignment: start;
                                    Text { text: t.name;  color: Kolory.tekst; font-size: 10px; font-family: "Geist"; horizontal-alignment: left; x: 6px; }
                                    Text { text: t.size + "  •  " + t.layers;  color: Kolory.tekst; font-size: 8px;  font-family: "Geist"; horizontal-alignment: left; x: 6px; }
                                }
                            }
                        }
                    }
                }
            }
```

Notes:
- `floor`, `mod`, `ceil` are built-in Slint math expressions on dimensionless numbers. If the compiler rejects `floor(...)` on a `length` expression, cast by dividing to a number: `floor((parent.width + spacing) / 1px / (cell_size / 1px + spacing / 1px))`.
- `root.thumbnails.length` is the Slint model count.
- The old `TimingStats`-style tooltip/context-menu machinery on tiles is dropped for now; add back in a later task if needed (not required by the spec).

- [ ] **Step 2: Remove the now-redundant horizontal scrollbar UI**

Delete the `thumbs_bar := Rectangle { ... }` block at [ui/appwindow.slint:1123-1168](ui/appwindow.slint#L1123-L1168) — the vertical `ScrollView` provides its own scrollbar.

Also adjust `viewport` at [ui/appwindow.slint:952-957](ui/appwindow.slint#L952-L957):

```slint
        viewport := Rectangle {
            x: 6px;
            y: 6px;
            width: parent.width - 12px;
            height: parent.height - 12px;
            clip: true;
```

(Drop the `- thumbs_panel.thumbs_bar_height - 4px` terms; the bar no longer exists.)

- [ ] **Step 3: Build + smoke test**

Run: `cargo run --bin EXruster`. Load a folder with ≥6 EXRs. Verify:
- Thumbnails arranged in a grid
- Resizing the main window reflows columns (more cols as window widens)
- Clicking ×1 / ×2 / ×3 in right panel regenerates thumbnails at larger cell sizes
- Vertical scrollbar appears when rows exceed panel height

- [ ] **Step 4: Commit**

```bash
git add ui/appwindow.slint
git commit -m "feat(ui): adaptive grid layout for thumbnail panel"
```

---

### Task 4.3: Remove obsolete tooltip/context-menu properties (optional cleanup)

**Files:**
- Modify: `ui/appwindow.slint`

- [ ] **Step 1: Strip unused properties introduced for horizontal-strip UX**

In the `thumbs_panel` declaration, the following properties are no longer referenced now that the horizontal scroll-wheel navigation and tooltip-on-hover logic moved out: `tt_visible`, `tt_text`, `tt_x`, `tt_y`, `tt_delay_ticks`, `tt_owner_path`, `thumbs_bar_height`. The context-menu state (`ctx_visible`, `ctx_path`, `ctx_x`, `ctx_y`, `ctx_longpress_ticks`, `dbl_click_ticks`) **stays** because `delete-thumbnail` flow still uses it elsewhere.

Search [ui/appwindow.slint](ui/appwindow.slint) for each removed property name to confirm there are zero references, then delete the declaration.

- [ ] **Step 2: Build**

Run: `cargo build`
Expected: clean build, no warnings about unused Slint properties.

- [ ] **Step 3: Smoke test**

Same as Task 4.2 — no regression expected.

- [ ] **Step 4: Commit**

```bash
git add ui/appwindow.slint
git commit -m "chore(ui): remove obsolete thumbnail-strip properties"
```

---

# Phase 5 — Verification

### Task 5.1: End-to-end smoke test & test-suite run

**Files:** none

- [ ] **Step 1: Full test suite**

Run: `cargo test`
Expected: 23 previously passing tests still pass + 1 new (`folder_tree::tests::lists_subdirs_alphabetically`); the pre-existing SIMD gamma LUT failure (see session notes for 2026-04-17) remains.

- [ ] **Step 2: Manual verification matrix**

Launch: `cargo run --bin EXruster`

Confirm each of the following:

- [ ] Right panel shows, in order below the Export block: separator, **Copy File Path**, **Copy File To...**, label **EXR Browser**, three square ×1 / ×2 / ×3 buttons, folder tree area.
- [ ] Both copy buttons are disabled while no file is open and become enabled after File → Open EXR.
- [ ] **Copy File Path** writes the absolute path to the clipboard (verify with paste into Notepad).
- [ ] **Copy File To...** opens a folder picker and copies the file to the chosen folder.
- [ ] Selecting the active size button highlights it; clicking ×2 / ×3 regenerates visible thumbnails at larger sizes.
- [ ] Folder tree renders after Select Working Folder; ▶/▼ toggles expansion; clicking a row loads that folder's thumbnails and highlights the current row bold.
- [ ] Thumbnail panel arranges tiles in a grid; resizing the window reflows columns.
- [ ] Dragging the panel's top edge extends it up to (roughly) the preview image bottom edge.

- [ ] **Step 3: Capture any regressions as issues** before marking the plan complete.

---

## Self-Review Notes

- Every task contains the exact code or diff required; no "see above" references.
- Task 2.1 deliberately preserves baseline behaviour by passing the literal `130` at every existing call site; Task 2.3 activates dynamic heights.
- Task 3.1 adds `FolderEntry` in `appwindow.slint`; Task 3.2 moves the canonical definition into `FolderTree.slint`. If Slint complains about a duplicate, keep only the one in `FolderTree.slint` (Task 3.2 Step 2 already notes this).
- Task 4.2 mentions a fallback if Slint's `floor` rejects length arithmetic — use dimensionless division (`/ 1px`).
- Clipboard access via `arboard` works cross-platform; on Windows it binds to `clipboard-win` (already transitively present).
- All commits are self-contained; each task compiles and runs independently.
