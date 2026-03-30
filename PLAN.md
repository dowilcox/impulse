# Migration Plan: impulse-linux GTK4 → Qt/QML (KDE-native)

## Summary

Replace the entire `impulse-linux` GTK4/libadwaita frontend (~14,000 lines) with a Qt Quick Controls 2 + CXX-Qt frontend. The GTK code is removed immediately. The Rust backend crates (`impulse-core`, `impulse-editor`, `impulse-ffi`) remain unchanged.

## Architecture Overview

```
┌─────────────────────────────────────────────────────────┐
│                    QML UI Layer                          │
│  (Qt Quick Controls 2 + Breeze style)                   │
│  Main.qml, Sidebar.qml, TabBar.qml, StatusBar.qml, ... │
├────────────┬──────────────────────────┬─────────────────┤
│ WebEngine  │     qmltermwidget        │   Qt Quick      │
│ View       │     (QML terminal)       │   Controls      │
│ (Monaco)   │                          │   (sidebar,     │
│            │                          │    settings)     │
├────────────┴──────────────────────────┴─────────────────┤
│              CXX-Qt Bridge Layer (Rust)                  │
│  #[qobject] types exposing impulse-core to QML          │
│  WindowModel, FileTreeModel, EditorBridge, LspBridge... │
├─────────────────────────────────────────────────────────┤
│              impulse-core + impulse-editor               │
│  (unchanged — PTY, LSP, filesystem, git, search, theme) │
└─────────────────────────────────────────────────────────┘
```

### Key Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| UI framework | Qt Quick Controls 2 | Lighter than Kirigami, auto-themed by Breeze on KDE, standard for desktop apps |
| Terminal widget | qmltermwidget | QML-native, no C++ wrapper needed, proven (cool-retro-term uses it) |
| Editor (Monaco) | QML WebEngineView | Qt provides a native QML WebEngineView type, mirrors current WebKitGTK approach |
| Rust↔QML bridge | CXX-Qt 0.8 | KDE-endorsed, actively maintained by KDAB, generates QObjects from Rust |
| Build system | CMake wrapping Cargo | Required for QtWebEngine initialization in C++ main, standard for KDE/Qt apps |
| Crate name | impulse-linux (in-place replacement) | No new crate — GTK code removed, Qt code takes its place |
| GTK frontend | Remove immediately | Clean break, no dual maintenance |

### New System Dependencies

```bash
# Arch/CachyOS
sudo pacman -S qt6-base qt6-declarative qt6-webengine qt6-quickcontrols2 \
               cmake extra-cmake-modules
# qmltermwidget will be built from source or packaged

# Debian/Ubuntu equivalent
sudo apt install qt6-base-dev qt6-declarative-dev qt6-webengine-dev \
                 qml6-module-qtquick-controls cmake extra-cmake-modules
```

### New Directory Structure

```
impulse-linux/
├── CMakeLists.txt              # Top-level CMake build
├── cpp/
│   └── main.cpp                # C++ entry point (QtWebEngine init)
├── rust/
│   ├── Cargo.toml              # CXX-Qt dependencies (staticlib)
│   ├── build.rs                # CXX-Qt builder config
│   └── src/
│       ├── lib.rs              # Module declarations
│       ├── window_model.rs     # QObject: window state (tabs, sidebar, status bar)
│       ├── file_tree_model.rs  # QObject: file tree data + git status
│       ├── search_model.rs     # QObject: file/content search
│       ├── editor_bridge.rs    # QObject: Monaco command/event protocol
│       ├── lsp_bridge.rs       # QObject: LSP registry, diagnostics, completion
│       ├── settings_model.rs   # QObject: settings read/write
│       ├── theme_bridge.rs     # QObject: theme data for QML + Monaco
│       └── keybindings.rs      # Keybinding definitions + override resolution
├── qml/
│   ├── Main.qml                # Root window: sidebar + content + status bar
│   ├── Sidebar.qml             # File tree / search panel switcher
│   ├── FileTreeView.qml        # Recursive file tree with git badges
│   ├── FileNodeDelegate.qml    # Single file/folder row in the tree
│   ├── TabBar.qml              # Editor/terminal tab bar
│   ├── ContentArea.qml         # Tab content area (editors, terminals, previews)
│   ├── TerminalView.qml        # qmltermwidget wrapper with split support
│   ├── EditorView.qml          # WebEngineView for Monaco
│   ├── StatusBar.qml           # Bottom status bar (CWD, git, cursor, language...)
│   ├── SearchPanel.qml         # Project search (file + content)
│   ├── QuickOpenDialog.qml     # Ctrl+P file picker
│   ├── CommandPalette.qml      # Ctrl+Shift+P command palette
│   ├── GoToLineDialog.qml      # Ctrl+G go-to-line
│   └── SettingsWindow.qml      # Settings page (editor, terminal, appearance, keys)
└── resources/
    └── icons/                  # File type SVG icons (reuse from assets/)
```

---

## Phased Implementation

### Phase 0: Scaffold & Build System
**Goal:** Empty Qt window compiles and runs via CMake + CXX-Qt.

1. Remove all GTK4/libadwaita Rust code from `impulse-linux/src/`
2. Remove GTK/VTE/WebKit dependencies from Cargo.toml
3. Create `impulse-linux/rust/` with new Cargo.toml:
   - Dependencies: `cxx`, `cxx-qt`, `cxx-qt-lib` (with `qt_full` feature), `impulse-core`, `impulse-editor`, `serde_json`, `tokio`, `log`
   - Build deps: `cxx-qt-build` (with `link_qt_object_files` feature)
   - Crate type: `staticlib`
4. Create `build.rs` with `CxxQtBuilder` and QML module registration
5. Create `cpp/main.cpp`:
   - `QtWebEngineQuick::initialize()` (before QGuiApplication)
   - `QGuiApplication` + `QQmlApplicationEngine`
   - Load `Main.qml`
6. Create `CMakeLists.txt`:
   - Find Qt6 (Core, Gui, Qml, Quick, QuickControls2, WebEngineQuick)
   - `cxx_qt_import_crate()` for the Rust staticlib
   - `cxx_qt_import_qml_module()` for the QML types
   - Link everything into the `impulse` executable
7. Create minimal `Main.qml` with an `ApplicationWindow` that shows "Hello from Qt"
8. Create a minimal `#[cxx_qt::bridge]` QObject in Rust to verify the bridge works
9. Verify: `cmake -B build && cmake --build build && ./build/impulse`

**Deliverable:** An empty Qt window opens. CXX-Qt bridge is functional.

### Phase 1: Window Layout & Theme Foundation
**Goal:** Main window with sidebar/content/status bar structure, themed by KDE.

1. Define `WindowModel` QObject in Rust (`window_model.rs`):
   - Properties: `sidebarVisible`, `sidebarWidth`, `currentDirectory`, `activeTabIndex`
   - Signals: `directoryChanged`, `tabSwitched`
   - Invokables: `toggleSidebar()`, `setDirectory(path)`
2. Define `ThemeBridge` QObject (`theme_bridge.rs`):
   - Properties: theme colors exposed as QStrings (bg, fg, accent, border, etc.)
   - Invokable: `setTheme(id)`, `availableThemes()`
   - Reads from `impulse_core::theme` resolved themes
3. Build `Main.qml`:
   - `ApplicationWindow` with `SplitView` (sidebar | content area)
   - Sidebar placeholder (colored rectangle)
   - Content area placeholder
   - Status bar at bottom
4. Build `StatusBar.qml`:
   - Row of labels: CWD, git branch, shell name, cursor pos, language, encoding, indent
   - Bound to `WindowModel` properties
5. Apply theme colors from `ThemeBridge` to QML components
6. Verify KDE Breeze style is applied automatically

**Deliverable:** Themed window with sidebar/content split and status bar. Sidebar toggles.

### Phase 2: File Tree Sidebar
**Goal:** Navigable file tree with git status and context menus.

1. Define `FileTreeModel` QObject (`file_tree_model.rs`):
   - Subclass `QAbstractItemModel` (tree model for QML TreeView)
   - Wraps `impulse_core::filesystem::read_directory_entries()`
   - Lazy-loads children on expand
   - Properties per node: name, path, isDir, isSymlink, gitStatus, isExpanded
   - Invokables: `setRootPath(path)`, `toggleExpand(index)`, `refresh()`
   - Integrates `impulse_core::git::get_git_status_for_directory()` for badges
2. Build `FileTreeView.qml`:
   - `TreeView` with `FileNodeDelegate` for each row
   - File/folder icons (load SVGs from bundled assets)
   - Git status color coding (modified=yellow, added=green, untracked=gray, etc.)
   - Click to open file (signal to WindowModel)
   - Double-click directory to expand/collapse
3. Build `FileNodeDelegate.qml`:
   - Icon + filename + git badge layout
   - Hover highlight
   - Active file highlight
4. Context menus (right-click):
   - New File, New Folder, Rename, Delete
   - Reveal in File Manager, Copy Path
   - Invokables on FileTreeModel for file operations
5. Wire sidebar toggle button in toolbar/header
6. Add "show hidden files" toggle

**Deliverable:** Fully functional file tree with git integration and context menus.

### Phase 3: Terminal Integration
**Goal:** Working terminal tabs with splitting.

1. Integrate qmltermwidget:
   - Add as a git submodule or vendored dependency
   - Build via CMake `add_subdirectory()` or as a QML plugin
   - Register `QMLTermWidget` and `QMLTermSession` types
2. Build `TerminalView.qml`:
   - `QMLTermWidget` with font/colors from settings
   - `QMLTermSession` configured with user's shell (via `impulse_core::shell`)
   - Working directory from WindowModel.currentDirectory
   - Color palette from ThemeBridge terminal colors
3. Define terminal-related properties on WindowModel:
   - `terminalFontFamily`, `terminalFontSize`, `terminalScrollback`
   - Signal: `terminalCwdChanged(path)` (from OSC 7 detection)
4. Terminal splitting:
   - `SplitView` within the terminal tab area
   - Horizontal and vertical splits
   - Focus tracking (which terminal is active)
   - Keyboard shortcuts for split/close/navigate
5. Tab creation:
   - "New Terminal" creates a terminal tab
   - Wire to tab bar (Phase 5)
6. Copy-on-select, search bar (Ctrl+Shift+F in terminal)

**Deliverable:** Terminal tabs with shell integration, splitting, and theming.

### Phase 4: Monaco Editor Integration
**Goal:** Code editor tabs with Monaco via WebEngineView.

1. Define `EditorBridge` QObject (`editor_bridge.rs`):
   - Manages editor instances (HashMap of file_path → editor state)
   - Properties: `isModified`, `currentFile`, `cursorLine`, `cursorColumn`, `language`, `indentInfo`
   - Invokables: `openFile(path)`, `saveFile()`, `sendCommand(json)`, `setTheme(json)`
   - Signal: `editorEvent(json)` (forwards Monaco events to QML)
   - Wraps `impulse_editor::assets::ensure_monaco_extracted()`
   - Wraps `impulse_editor::protocol::EditorCommand/EditorEvent` serialization
2. Build `EditorView.qml`:
   - `WebEngineView` loading Monaco editor HTML
   - JavaScript channel for bidirectional messaging:
     - QML → Monaco: `runJavaScript()` calls with EditorCommand JSON
     - Monaco → QML: `WebChannel` or `runJavaScript` callback for EditorEvent JSON
   - File loading: reads file content, sends `OpenFile` command
   - Save: captures content from `ContentChanged` events, writes to disk
   - Modified indicator (dot/icon on tab)
3. Editor warm-up pool:
   - Pre-create a WebEngineView at startup (like current GTK warm-up pool)
   - Instant first editor tab opening
4. File watching:
   - Detect external file modifications (via `QFileSystemWatcher` or `notify` crate)
   - Prompt reload if file changed externally
5. Preview mode:
   - Markdown preview via `impulse_editor::markdown::render_markdown_preview()`
   - SVG preview via `impulse_editor::svg::render_svg_preview()`
   - Toggle button in status bar
6. Image preview:
   - Image tabs display the image directly (QML `Image` component)

**Deliverable:** Full Monaco editor with file operations, preview, and external change detection.

### Phase 5: Tab System
**Goal:** Complete tab bar managing terminals, editors, and previews.

1. Extend WindowModel with tab state:
   - `tabDisplayInfos` (list model: title, icon, isModified, tabType)
   - `activeTabIndex`
   - Invokables: `createTab(type)`, `closeTab(index)`, `selectTab(index)`, `moveTab(from, to)`
2. Build `TabBar.qml`:
   - Horizontal row of tab buttons
   - Active tab highlight
   - Close button per tab (hover-reveal)
   - Drag-and-drop reordering
   - Hidden when only 1 tab (like macOS frontend)
   - Middle-click to close
   - Context menu: Close, Close Others, Close All
3. Build `ContentArea.qml`:
   - `StackLayout` or `Loader` that swaps content based on active tab
   - Manages TerminalView, EditorView, or Image instances per tab
   - Preserves terminal/editor state when switching tabs
4. Wire file tree clicks to editor tab creation
5. Wire Ctrl+T for new terminal, Ctrl+W for close tab, Ctrl+Tab for next tab

**Deliverable:** Fully functional tabbed interface with drag-reorder and keyboard navigation.

### Phase 6: Search & Dialogs
**Goal:** Project search, quick-open, and command palette.

1. Define `SearchModel` QObject (`search_model.rs`):
   - Wraps `impulse_core::search::search_filenames()` and `search_contents()`
   - Properties: `query`, `results` (list model), `isSearching`, `caseSensitive`
   - Debounced search (timer-based, cancels previous search via AtomicBool)
2. Build `SearchPanel.qml`:
   - Search input + case-sensitive toggle
   - Results list with file path, line number, preview text
   - Click result to open file at line
   - Result count display
3. Build `QuickOpenDialog.qml`:
   - Popup/Dialog with search input
   - Fuzzy file name search results
   - Enter to open selected file
   - Ctrl+P to trigger
4. Build `CommandPalette.qml`:
   - Popup/Dialog listing available commands
   - Filterable by typing
   - Ctrl+Shift+P to trigger
5. Build `GoToLineDialog.qml`:
   - Simple input for line number
   - Ctrl+G to trigger

**Deliverable:** Project search, quick-open, and command palette all functional.

### Phase 7: LSP Integration
**Goal:** Language intelligence in the editor.

1. Define `LspBridge` QObject (`lsp_bridge.rs`):
   - Wraps `impulse_core::lsp` registry
   - Creates one registry per workspace (like current GTK approach)
   - Invokables: `ensureServers(languageId, fileUri)`, `request(method, params)`, `notify(method, params)`
   - Background polling: timer-based `pollEvent()` that emits signals
   - Signals: `diagnosticsReceived(uri, json)`, `serverInitialized(id)`, `serverError(msg)`
2. Wire LSP to editor:
   - On file open: `textDocument/didOpen` notification
   - On content change: `textDocument/didChange` notification
   - On save: `textDocument/didSave` notification
   - Diagnostics → `EditorBridge.sendCommand(ApplyDiagnostics)`
3. Autocomplete:
   - `CompletionRequested` event from Monaco → LSP `textDocument/completion` request
   - Response → `EditorBridge.sendCommand(ResolveCompletions)`
4. Hover info:
   - `HoverRequested` → LSP `textDocument/hover` → `ResolveHover`
5. Go-to-definition, references, rename, code actions, formatting:
   - Same request/response pattern for each
6. Signature help:
   - `SignatureHelpRequested` → LSP → `ResolveSignatureHelp`

**Deliverable:** Full LSP support matching the current GTK frontend's capabilities.

### Phase 8: Settings
**Goal:** Settings UI and persistence.

1. Define `SettingsModel` QObject (`settings_model.rs`):
   - Wraps `impulse_core::settings::Settings` struct
   - Exposes every setting as a Q_PROPERTY
   - Invokables: `save()`, `resetToDefaults()`, `addFileTypeOverride()`, `removeFileTypeOverride(index)`, `addCommandOnSave()`, `removeCommandOnSave(index)`
   - Signal: `settingsChanged()` (triggers theme/editor/terminal refresh)
   - Reads/writes `~/.config/impulse/settings.json`
2. Build `SettingsWindow.qml`:
   - `ApplicationWindow` or `Dialog` with pages/tabs:
     - **Appearance:** Theme selector (dropdown), font family, font size
     - **Editor:** Tab width, spaces/tabs, word wrap, minimap, line numbers, bracket colorization, etc.
     - **Terminal:** Font family/size, scrollback, cursor shape/blink, copy-on-select, bell
     - **Keybindings:** Per-keybinding override rows, conflict detection
     - **File Type Overrides:** Pattern-based editor overrides (add/remove)
     - **Commands on Save:** File pattern → command mappings (add/remove)
   - Qt Quick Controls 2 components: ComboBox, SpinBox, Switch, TextField, ScrollView
3. Apply settings changes live:
   - Theme change → update ThemeBridge → re-theme all editors + terminals
   - Font change → re-apply to editors + terminals
   - Editor options → send `UpdateSettings` to all Monaco instances

**Deliverable:** Complete settings UI matching current GTK settings page.

### Phase 9: Keybindings & Polish
**Goal:** Full keyboard shortcut system and final polish.

1. Keybinding system:
   - Port keybinding registry from current `keybindings.rs`
   - Capture-phase key handling (Shortcut items in QML, or `Keys.onPressed` with event filtering)
   - Built-in bindings: Ctrl+T (new terminal), Ctrl+W (close tab), Ctrl+P (quick-open), Ctrl+Shift+P (palette), Ctrl+` (toggle terminal), Ctrl+B (toggle sidebar), Ctrl+S (save), etc.
   - Terminal splits: Ctrl+Shift+D (horizontal), Ctrl+D (vertical split)
   - Custom keybinding overrides from settings
   - Custom command keybindings (run shell commands)
2. Keyboard focus management:
   - Ensure WebEngineView and qmltermwidget properly receive/release focus
   - Tab switching doesn't steal focus from terminal/editor
3. Drag-and-drop:
   - Drop files from file manager onto terminal (inserts path)
   - Drop files onto editor area (opens file)
   - Tab drag-reorder (Phase 5)
4. Toast notifications:
   - Port toast/notification system (file saved, error messages, etc.)
5. CLI flags:
   - `--dev` mode (separate config + app identity)
   - `--install-lsp-servers` / `--check-lsp-servers`
6. Window state persistence:
   - Save/restore window size, sidebar width, open tabs, active directory
7. Update checker:
   - Background version check via `impulse_core::update`
   - Unobtrusive notification in status bar

**Deliverable:** Production-ready application matching GTK frontend's feature set.

### Phase 10: Build, Package & Release
**Goal:** Ship it.

1. Update `CMakeLists.txt` with install targets:
   - Binary to `/usr/bin/impulse`
   - Desktop file to `/usr/share/applications/`
   - Icons to `/usr/share/icons/hicolor/`
   - QML files bundled into the binary via Qt Resource System
2. Update `pkg/arch/PKGBUILD`:
   - Replace GTK/VTE/WebKit deps with Qt6/qmltermwidget deps
   - CMake build instead of cargo build
3. Generate .deb and .rpm packages:
   - Update dependency lists
   - CMake's CPack or manual packaging
4. Update `scripts/release.sh`:
   - Build step changes from `cargo build -p impulse-linux` to `cmake --build`
   - Package generation updated for new deps
5. Update CLAUDE.md:
   - New build commands, dependencies, architecture description
6. Remove dead code:
   - Any leftover GTK references
   - Unused workspace members if any

**Deliverable:** Packaged, releasable Qt frontend.

---

## Risk Assessment

| Risk | Severity | Mitigation |
|------|----------|------------|
| CXX-Qt is pre-1.0 | Medium | Pin to v0.8.x, monitor releases. API is stabilized as of v0.7-0.8. KDAB actively maintains it. |
| qmltermwidget Qt6 maturity | Medium | Test early in Phase 3. Fallback: wrap C++ qtermwidget instead. |
| WebEngineView JS bridge complexity | Low | Qt WebEngine supports `runJavaScript()` and `WebChannel` — well-documented. Same pattern as current WebKitGTK. |
| GPLv2 from qmltermwidget | Low/Medium | Only affects distribution licensing. If Impulse is GPL-compatible, no issue. Flag if proprietary licensing is needed. |
| Large rewrite scope (~14k lines) | High | Phased approach with working milestones. Each phase produces a testable increment. |
| Loss of GTK frontend during migration | Medium | User's explicit choice. Git history preserves all GTK code for reference. |

## Estimated Complexity per Phase

| Phase | New Files | Approx Lines | Depends On |
|-------|-----------|-------------|------------|
| 0 - Scaffold | 5 | ~200 | Nothing |
| 1 - Layout & Theme | 5 | ~600 | Phase 0 |
| 2 - File Tree | 4 | ~1,200 | Phase 1 |
| 3 - Terminal | 2 | ~800 | Phase 1 |
| 4 - Editor | 3 | ~1,500 | Phase 1 |
| 5 - Tab System | 3 | ~800 | Phases 3, 4 |
| 6 - Search & Dialogs | 5 | ~900 | Phase 2 |
| 7 - LSP | 2 | ~1,000 | Phase 4 |
| 8 - Settings | 3 | ~1,200 | Phase 1 |
| 9 - Keybindings & Polish | 2 | ~800 | All above |
| 10 - Package & Release | 3 | ~300 | All above |

**Total: ~37 files, ~9,300 lines** (down from 14k GTK lines — QML is more concise than imperative GTK Rust code)
