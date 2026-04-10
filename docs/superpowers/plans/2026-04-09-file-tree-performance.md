# File Tree Performance Optimization

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate file tree rendering slowdown when many folders are expanded and fix instant file change detection for the SwiftUI sidebar.

**Architecture:** Replace the recursive `FileNodeView` (which nests children inside parent VStacks, defeating LazyVStack virtualization) with a flattened array of `(node, depth)` entries rendered in a single LazyVStack. Move `loadChildren()` off the main thread to prevent UI hitches on expansion. Sync the FileTreeView filesystem watcher to WindowModel so the SwiftUI sidebar sees filesystem changes immediately.

**Tech Stack:** Swift, SwiftUI (@Observable), AppKit

**Platform note:** macOS-only change. The Linux frontend (Qt QML) uses model/view architecture with built-in virtualization and does not have this issue.

---

## Root Cause Summary

1. **Recursive view hierarchy defeats LazyVStack** — `FileNodeView` nests children inside its own `VStack`. LazyVStack only virtualizes top-level nodes. With 50 expanded folders (500+ nodes), all views are materialized simultaneously.
2. **@Observable broadcast storm** — Each `FileNodeView` holds `@Bindable var node`. Git status updates (every 2s) mutate `gitStatus` on hundreds of nodes, causing hundreds of view re-renders across the non-virtualized tree.
3. **`loadChildren()` blocks main thread** — Synchronous `FileManager.contentsOfDirectory()` runs on the main thread during expansion.
4. **FileTreeView watcher doesn't sync to WindowModel** — The filesystem watcher in `FileTreeView` rebuilds its internal `rootNodes` but never updates `windowModel.fileTreeNodes`, so the SwiftUI sidebar misses filesystem changes until a manual refresh.

## File Structure

**Modified files:**

- `impulse-macos/Sources/ImpulseApp/Sidebar/FileTreeNode.swift` — Add `FlatTreeEntry`, `flatten()`, `buildChildren()`
- `impulse-macos/Sources/ImpulseApp/SwiftUI/Models/WindowModel.swift` — Add `flatFileTree`, `rebuildFlatTree()`, `updateFileTree()`
- `impulse-macos/Sources/ImpulseApp/SwiftUI/Views/FileTreeListView.swift` — Replace recursive view with flat rendering
- `impulse-macos/Sources/ImpulseApp/MainWindow.swift` — Wire `updateFileTree()` at all tree update sites, add `onCollapseAll` rebuild
- `impulse-macos/Sources/ImpulseApp/Sidebar/FileTreeView.swift` — Add `onTreeRefreshed` callback to sync watcher changes

No new files created.

---

### Task 1: Add FlatTreeEntry and tree flattening to FileTreeNode

**Files:**

- Modify: `impulse-macos/Sources/ImpulseApp/Sidebar/FileTreeNode.swift`

- [ ] **Step 1: Add FlatTreeEntry struct and flatten method**

Add after the closing brace of the `FileTreeNode` class (after line 193):

```swift
// MARK: - Flat Tree Entry

/// A single entry in the flattened file tree. Used by FileTreeListView for
/// efficient virtualized rendering via LazyVStack (instead of recursive views).
struct FlatTreeEntry: Identifiable {
    let id: String
    let node: FileTreeNode
    let depth: Int
}

extension FileTreeNode {
    /// Walk the tree and produce a flat array of all visible (expanded) nodes
    /// with their depth levels. O(n) where n = total visible nodes.
    static func flatten(_ nodes: [FileTreeNode], depth: Int = 0) -> [FlatTreeEntry] {
        var result: [FlatTreeEntry] = []
        result.reserveCapacity(nodes.count * 2)
        flattenInto(&result, nodes: nodes, depth: depth)
        return result
    }

    private static func flattenInto(
        _ result: inout [FlatTreeEntry],
        nodes: [FileTreeNode],
        depth: Int
    ) {
        for node in nodes {
            result.append(FlatTreeEntry(id: node.path, node: node, depth: depth))
            if node.isDirectory && node.isExpanded, let children = node.children {
                flattenInto(&result, nodes: children, depth: depth + 1)
            }
        }
    }
}
```

- [ ] **Step 2: Add buildChildren static method for async loading**

Add inside the `FileTreeNode` class, after the `loadChildren` method (after line 94, before `// MARK: Building a Top-Level Tree`):

```swift
    /// Build children for a directory path without mutating any node.
    /// Intended for background-thread use: call this off main thread, then
    /// assign the result to `node.children` on the main thread.
    static func buildChildren(path: String, showHidden: Bool) -> [FileTreeNode] {
        let fm = FileManager.default
        let url = URL(fileURLWithPath: path, isDirectory: true)

        guard let contents = try? fm.contentsOfDirectory(
            at: url,
            includingPropertiesForKeys: [.isDirectoryKey, .isSymbolicLinkKey],
            options: showHidden ? [] : [.skipsHiddenFiles]
        ) else {
            return []
        }

        var nodes: [FileTreeNode] = []
        for itemURL in contents {
            let itemName = itemURL.lastPathComponent
            if !showHidden && itemName.hasPrefix(".") { continue }
            if itemName == ".DS_Store" { continue }
            let isDir = (try? itemURL.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory ?? false
            nodes.append(FileTreeNode(name: itemName, path: itemURL.path, isDirectory: isDir))
        }

        nodes.sort { lhs, rhs in
            if lhs.isDirectory != rhs.isDirectory { return lhs.isDirectory }
            return lhs.name.localizedCaseInsensitiveCompare(rhs.name) == .orderedAscending
        }

        return nodes
    }
```

- [ ] **Step 3: Build**

Run: `cd /Users/dowilcox/Code/impulse && cargo build -p impulse-ffi && cd impulse-macos && swift build 2>&1 | tail -5`
Expected: Build succeeds (new code is additive, no callers yet)

- [ ] **Step 4: Commit**

```bash
git add impulse-macos/Sources/ImpulseApp/Sidebar/FileTreeNode.swift
git commit -m "Add FlatTreeEntry and tree flattening for virtualized file tree rendering"
```

---

### Task 2: Add flatFileTree and updateFileTree to WindowModel

**Files:**

- Modify: `impulse-macos/Sources/ImpulseApp/SwiftUI/Models/WindowModel.swift`

- [ ] **Step 1: Add flatFileTree property**

Add after line 44 (`var fileTreeRootPath: String = ""`):

```swift
    /// Flattened view of the file tree for LazyVStack rendering.
    /// Rebuilt explicitly when tree structure changes (expand/collapse/rebuild),
    /// NOT on git status changes — individual row views observe those directly
    /// via @Bindable on each node.
    var flatFileTree: [FlatTreeEntry] = []
```

- [ ] **Step 2: Add updateFileTree and rebuildFlatTree methods**

Add after the `updateStatusBar` method (after line 118, before the closing brace):

```swift
    /// Replace the file tree nodes and rebuild the flat rendering list.
    /// Use this instead of setting `fileTreeNodes` directly.
    func updateFileTree(_ nodes: [FileTreeNode], rootPath: String? = nil) {
        fileTreeNodes = nodes
        if let rootPath { fileTreeRootPath = rootPath }
        rebuildFlatTree()
    }

    /// Rebuild the flat tree from current nodes. Call after any structural
    /// change (expand, collapse, children loaded) but NOT after git status
    /// changes — row views observe those via @Bindable.
    func rebuildFlatTree() {
        flatFileTree = FileTreeNode.flatten(fileTreeNodes)
    }
```

- [ ] **Step 3: Build**

Run: `cd /Users/dowilcox/Code/impulse/impulse-macos && swift build 2>&1 | tail -5`
Expected: Build succeeds

- [ ] **Step 4: Commit**

```bash
git add impulse-macos/Sources/ImpulseApp/SwiftUI/Models/WindowModel.swift
git commit -m "Add flatFileTree and updateFileTree to WindowModel for virtualized rendering"
```

---

### Task 3: Replace recursive FileTreeListView with flat rendering

This is the core performance fix. The recursive `FileNodeView` is replaced with a flat `FlatFileRowView` that renders each node as an independent row in the `LazyVStack`. Children are no longer nested inside their parent's view — they're separate entries with calculated indentation.

**Files:**

- Modify: `impulse-macos/Sources/ImpulseApp/SwiftUI/Views/FileTreeListView.swift`

- [ ] **Step 1: Replace FileTreeListView body to use flatFileTree**

Replace lines 1-29 (`FileTreeListView` struct) with:

```swift
import SwiftUI
import AppKit

/// Displays the project file tree as a flat, virtualized scrollable list.
/// Each expanded directory's children appear as separate entries with
/// calculated indentation — no recursive view nesting. This ensures
/// LazyVStack only materializes visible rows regardless of how many
/// folders are expanded.
struct FileTreeListView: View {
    var model: WindowModel
    @State private var isRootDropTarget = false

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(model.flatFileTree) { entry in
                    FlatFileRowView(node: entry.node, depth: entry.depth, model: model)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .onDrop(of: [.fileURL], isTargeted: $isRootDropTarget) { providers in
            FileDropHelper.handleDrop(
                providers: providers,
                targetDir: model.fileTreeRootPath,
                projectRoot: model.fileTreeRootPath,
                onComplete: { model.onRefreshTree?() }
            )
            return true
        }
        .background(isRootDropTarget ? Color.accentColor.opacity(0.08) : Color.clear)
    }
}
```

- [ ] **Step 2: Replace FileNodeView with FlatFileRowView**

Replace lines 31-119 (the entire `FileNodeView` struct) with the flat, non-recursive version:

```swift
/// A single flat row in the file tree. Not recursive — children are separate
/// entries in the flat list with incremented depth.
private struct FlatFileRowView: View {
    @Bindable var node: FileTreeNode
    let depth: Int
    var model: WindowModel
    @State private var isHovered = false
    @State private var isDropTarget = false

    private var isActiveFile: Bool {
        !node.isDirectory && node.path == model.activeFilePath
    }

    private var rowBackground: Color {
        if isDropTarget && node.isDirectory {
            return Color.accentColor.opacity(0.3)
        } else if isActiveFile {
            return Color.accentColor.opacity(0.2)
        } else if isHovered {
            return Color.primary.opacity(0.06)
        }
        return Color.clear
    }

    var body: some View {
        HStack(spacing: 0) {
            if depth > 0 {
                Spacer()
                    .frame(width: CGFloat(depth) * 16)
            }

            if node.isDirectory {
                Image(systemName: node.isExpanded ? "chevron.down" : "chevron.right")
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(.tertiary)
                    .frame(width: 16, height: 16)
            } else {
                Spacer().frame(width: 16)
            }

            FileTreeRow(node: node, iconCache: model.iconCache)
        }
        .padding(.vertical, 3)
        .padding(.horizontal, 8)
        .background(
            RoundedRectangle(cornerRadius: 5)
                .fill(rowBackground)
        )
        .contentShape(Rectangle())
        .onHover { hovering in
            isHovered = hovering
        }
        .onDrag { [nodePath = node.path] in
            NSItemProvider(object: nodePath as NSString)
        }
        .onDrop(of: [.fileURL, .text], isTargeted: $isDropTarget) { providers in
            handleDrop(providers: providers)
        }
        .onTapGesture {
            if node.isDirectory {
                handleDirectoryTap()
            } else {
                model.onOpenFile?(node.path, nil)
            }
        }
        .contextMenu { nodeContextMenu(for: node) }
    }

    // MARK: - Directory Expand/Collapse

    private func handleDirectoryTap() {
        if node.isExpanded {
            // Collapse — always immediate.
            withAnimation(.easeInOut(duration: 0.15)) {
                node.isExpanded = false
                model.rebuildFlatTree()
            }
        } else if node.isLoaded {
            // Already loaded — expand immediately.
            withAnimation(.easeInOut(duration: 0.15)) {
                node.isExpanded = true
                model.rebuildFlatTree()
            }
            triggerGitStatusRefresh()
        } else {
            // Need to load children off main thread.
            // Flip chevron immediately so the user knows their click registered.
            node.isExpanded = true
            let showHidden = model.showHiddenFiles
            let path = node.path
            DispatchQueue.global(qos: .userInitiated).async {
                let children = FileTreeNode.buildChildren(path: path, showHidden: showHidden)
                DispatchQueue.main.async {
                    node.children = children
                    withAnimation(.easeInOut(duration: 0.15)) {
                        model.rebuildFlatTree()
                    }
                    triggerGitStatusRefresh()
                }
            }
        }
    }

    private func triggerGitStatusRefresh() {
        guard let children = node.children, !children.isEmpty else { return }
        let nodePath = node.path
        let rootPath = model.fileTreeRootPath
        DispatchQueue.global(qos: .utility).async {
            FileTreeNode.refreshGitStatus(
                nodes: children, repoPath: rootPath, dirPath: nodePath
            )
        }
    }

    // MARK: - Drop Handling

    private func handleDrop(providers: [NSItemProvider]) -> Bool {
        let targetDir: String
        if node.isDirectory {
            targetDir = node.path
        } else {
            targetDir = (node.path as NSString).deletingLastPathComponent
        }
        FileDropHelper.handleDrop(
            providers: providers,
            targetDir: targetDir,
            projectRoot: model.fileTreeRootPath,
            onComplete: { model.onRefreshTree?() }
        )
        return true
    }

    // MARK: - Context Menu

    @ViewBuilder
    private func nodeContextMenu(for node: FileTreeNode) -> some View {
        if node.isDirectory {
            Button("New File...") {
                model.onNewFile?(node.path)
            }
            Button("New Folder...") {
                model.onNewFolder?(node.path)
            }
            Divider()
        }

        Button("Reveal in Finder") {
            if node.isDirectory {
                NSWorkspace.shared.selectFile(nil, inFileViewerRootedAtPath: node.path)
            } else {
                NSWorkspace.shared.activateFileViewerSelecting(
                    [URL(fileURLWithPath: node.path)]
                )
            }
        }

        Button("Copy Path") {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(node.path, forType: .string)
        }

        Button("Copy Relative Path") {
            let relativePath: String
            if node.path.hasPrefix(model.fileTreeRootPath) {
                relativePath = String(
                    node.path.dropFirst(model.fileTreeRootPath.count)
                        .drop(while: { $0 == "/" })
                )
            } else {
                relativePath = node.name
            }
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(relativePath, forType: .string)
        }

        if !node.isDirectory {
            Divider()
            Button("Open with Default App") {
                NSWorkspace.shared.open(URL(fileURLWithPath: node.path))
            }
        }

        if !node.isDirectory,
           node.gitStatus == .modified || node.gitStatus == .added {
            Divider()
            Button("Discard Changes", role: .destructive) {
                let alert = NSAlert()
                alert.messageText = "Discard Changes"
                alert.informativeText = "Are you sure you want to discard all changes to \"\(node.name)\"? This cannot be undone."
                alert.alertStyle = .warning
                alert.addButton(withTitle: "Discard")
                alert.addButton(withTitle: "Cancel")
                guard alert.runModal() == .alertFirstButtonReturn else { return }
                if ImpulseCore.gitDiscardChanges(filePath: node.path, workspaceRoot: model.fileTreeRootPath) {
                    NotificationCenter.default.post(
                        name: .impulseReloadEditorFile,
                        object: nil,
                        userInfo: ["filePath": node.path]
                    )
                    model.onRefreshTree?()
                }
            }
        }

        Divider()

        Button("Rename\u{2026}") {
            let currentName = (node.path as NSString).lastPathComponent
            let alert = NSAlert()
            alert.messageText = "Rename"
            alert.informativeText = "Enter a new name:"
            alert.addButton(withTitle: "Rename")
            alert.addButton(withTitle: "Cancel")
            let input = NSTextField(frame: NSRect(x: 0, y: 0, width: 260, height: 24))
            input.stringValue = currentName
            alert.accessoryView = input
            alert.window.initialFirstResponder = input
            guard alert.runModal() == .alertFirstButtonReturn else { return }
            let newName = input.stringValue.trimmingCharacters(in: .whitespaces)
            guard !newName.isEmpty, newName != currentName,
                  !newName.contains("/"), !newName.contains("\0"),
                  !newName.contains("..") else { return }
            let parentDir = (node.path as NSString).deletingLastPathComponent
            let newPath = ((parentDir as NSString).appendingPathComponent(newName) as NSString).standardizingPath
            let normalizedParent = (parentDir as NSString).standardizingPath
            guard (newPath as NSString).deletingLastPathComponent == normalizedParent else { return }
            do {
                try FileManager.default.moveItem(atPath: node.path, toPath: newPath)
                model.onRefreshTree?()
            } catch {
                let errAlert = NSAlert()
                errAlert.messageText = "Rename Failed"
                errAlert.informativeText = error.localizedDescription
                errAlert.alertStyle = .warning
                errAlert.runModal()
            }
        }

        Button("Move to Trash", role: .destructive) {
            do {
                try FileManager.default.trashItem(
                    at: URL(fileURLWithPath: node.path),
                    resultingItemURL: nil
                )
                model.onRefreshTree?()
            } catch {
                let errAlert = NSAlert()
                errAlert.messageText = "Move to Trash Failed"
                errAlert.informativeText = error.localizedDescription
                errAlert.alertStyle = .warning
                errAlert.runModal()
            }
        }
    }
}
```

**Note:** The `FileTreeRow` struct (line 262-325) and `FileDropHelper` enum (line 327-424) are unchanged — leave them in place.

- [ ] **Step 3: Build**

Run: `cd /Users/dowilcox/Code/impulse/impulse-macos && swift build 2>&1 | tail -20`
Expected: Build succeeds. If there are errors about `FileNodeView` references elsewhere, check SidebarView.swift — it should already reference `FileTreeListView` which is unchanged.

- [ ] **Step 4: Commit**

```bash
git add impulse-macos/Sources/ImpulseApp/SwiftUI/Views/FileTreeListView.swift
git commit -m "Replace recursive FileNodeView with flat FlatFileRowView for virtualized rendering

The recursive structure nested children inside parent VStacks, defeating
LazyVStack virtualization. With many folders expanded, all 500+ nodes had
live SwiftUI views. The flat approach renders each node as an independent
row in the LazyVStack — only visible rows (~30-50) are materialized.

Also moves loadChildren() off the main thread to prevent UI hitches."
```

---

### Task 4: Wire updateFileTree and rebuildFlatTree at all tree update sites

Every place in `MainWindow.swift` that sets `windowModel.fileTreeNodes` must now use `updateFileTree()` (or call `rebuildFlatTree()`) so the flat list stays in sync.

**Files:**

- Modify: `impulse-macos/Sources/ImpulseApp/MainWindow.swift`

- [ ] **Step 1: Update initial tree load (around line 272)**

Find the initial tree load block (inside the `DispatchQueue.global` callback around line 263-275):

```swift
                self.fileTreeView.updateTree(nodes: nodes, rootPath: rootPath)
                self.fileTreeView.showHidden = showHidden
                self.fileTreeCacheInsert(key: rootPath, nodes: nodes)
                // Push to SwiftUI sidebar
                self.windowModel.fileTreeNodes = nodes
                self.windowModel.fileTreeRootPath = rootPath
```

Replace those lines with:

```swift
                self.fileTreeView.updateTree(nodes: nodes, rootPath: rootPath)
                self.fileTreeView.showHidden = showHidden
                self.fileTreeCacheInsert(key: rootPath, nodes: nodes)
                // Push to SwiftUI sidebar
                self.windowModel.updateFileTree(nodes, rootPath: rootPath)
```

- [ ] **Step 2: Update onRefreshTree callback (around line 347)**

Find the main thread callback inside `onRefreshTree` (around line 345-351):

```swift
                DispatchQueue.main.async { [weak self] in
                    guard let self else { return }
                    self.windowModel.fileTreeNodes = nodes
                    self.fileTreeView.updateTree(nodes: nodes, rootPath: root)
                    self.fileTreeCacheInsert(key: root, nodes: nodes)
                }
```

Replace with:

```swift
                DispatchQueue.main.async { [weak self] in
                    guard let self else { return }
                    self.windowModel.updateFileTree(nodes, rootPath: root)
                    self.fileTreeView.updateTree(nodes: nodes, rootPath: root)
                    self.fileTreeCacheInsert(key: root, nodes: nodes)
                }
```

- [ ] **Step 3: Update onCollapseAll callback (around line 353-366)**

Find the `onCollapseAll` closure:

```swift
        windowModel.onCollapseAll = { [weak self] in
            guard let self else { return }
            func collapseRecursively(_ nodes: [FileTreeNode]) {
                for node in nodes {
                    if node.isDirectory && node.isExpanded {
                        if let children = node.children {
                            collapseRecursively(children)
                        }
                        node.isExpanded = false
                    }
                }
            }
            collapseRecursively(self.windowModel.fileTreeNodes)
        }
```

Replace with:

```swift
        windowModel.onCollapseAll = { [weak self] in
            guard let self else { return }
            func collapseRecursively(_ nodes: [FileTreeNode]) {
                for node in nodes {
                    if node.isDirectory && node.isExpanded {
                        if let children = node.children {
                            collapseRecursively(children)
                        }
                        node.isExpanded = false
                    }
                }
            }
            collapseRecursively(self.windowModel.fileTreeNodes)
            self.windowModel.rebuildFlatTree()
        }
```

- [ ] **Step 4: Update onToggleHidden callback (around line 380-385)**

Find the main thread callback inside `onToggleHidden`:

```swift
                DispatchQueue.main.async { [weak self] in
                    guard let self else { return }
                    self.windowModel.fileTreeNodes = nodes
                    self.fileTreeView.updateTree(nodes: nodes, rootPath: root)
                    self.fileTreeView.showHidden = showHidden
                    self.fileTreeCacheInsert(key: root, nodes: nodes)
                }
```

Replace with:

```swift
                DispatchQueue.main.async { [weak self] in
                    guard let self else { return }
                    self.windowModel.updateFileTree(nodes, rootPath: root)
                    self.fileTreeView.updateTree(nodes: nodes, rootPath: root)
                    self.fileTreeView.showHidden = showHidden
                    self.fileTreeCacheInsert(key: root, nodes: nodes)
                }
```

- [ ] **Step 5: Check for switchFileTreeRoot**

Search `MainWindow.swift` for any other places that set `windowModel.fileTreeNodes` (likely in `switchFileTreeRoot` around line 2048+). Apply the same `updateFileTree()` change to each occurrence. The pattern is always the same: replace `self.windowModel.fileTreeNodes = nodes` and `self.windowModel.fileTreeRootPath = root` with `self.windowModel.updateFileTree(nodes, rootPath: root)`.

Run: `grep -n 'windowModel.fileTreeNodes\s*=' impulse-macos/Sources/ImpulseApp/MainWindow.swift`

Update every match that wasn't already covered in steps 1-4.

- [ ] **Step 6: Build**

Run: `cd /Users/dowilcox/Code/impulse/impulse-macos && swift build 2>&1 | tail -10`
Expected: Build succeeds

- [ ] **Step 7: Commit**

```bash
git add impulse-macos/Sources/ImpulseApp/MainWindow.swift
git commit -m "Wire updateFileTree/rebuildFlatTree at all tree update sites in MainWindow"
```

---

### Task 5: Sync FileTreeView filesystem watcher to WindowModel

The filesystem watcher in `FileTreeView` detects file changes (create, delete, rename) and rebuilds the tree — but only updates `FileTreeView.rootNodes`. The SwiftUI sidebar (which reads `windowModel.fileTreeNodes`) never sees these changes. This causes the "instant file changes" issue.

**Files:**

- Modify: `impulse-macos/Sources/ImpulseApp/Sidebar/FileTreeView.swift`
- Modify: `impulse-macos/Sources/ImpulseApp/MainWindow.swift`

- [ ] **Step 1: Add onTreeRefreshed callback to FileTreeView**

In `FileTreeView.swift`, add a callback property in the properties section (around line 108, after `var showHidden: Bool = false`):

```swift
    /// Called on the main thread after the tree is rebuilt from a filesystem
    /// watcher event. Passes the new root nodes so the caller can sync them
    /// to WindowModel for the SwiftUI sidebar.
    var onTreeRefreshed: (([FileTreeNode]) -> Void)?
```

- [ ] **Step 2: Fire the callback in refreshTree()**

In `FileTreeView.swift`, inside the `refreshTree()` method's main-thread callback (around line 458, after `self.watchExpandedSubdirectories(self.rootNodes)` and before `self.refreshGitStatus()`), add:

```swift
                // Sync new nodes to the SwiftUI sidebar via the callback.
                self.onTreeRefreshed?(self.rootNodes)
```

The surrounding code should now read:

```swift
                self.rebuildNodeIndex()
                self.watchExpandedSubdirectories(self.rootNodes)
                // Sync new nodes to the SwiftUI sidebar via the callback.
                self.onTreeRefreshed?(self.rootNodes)
                // Restore scroll after AppKit finishes its layout pass.
                self.pendingScrollRestore = savedOrigin
```

- [ ] **Step 3: Wire the callback in MainWindow**

In `MainWindow.swift`, find where `fileTreeView` is configured (near the initial tree build, around line 268). Add after `self.fileTreeView.updateTree(nodes: nodes, rootPath: rootPath)`:

Actually, the callback should be wired early in initialization, before `updateTree` is called. Find where `fileTreeView` is first used in `setupLayout()` or the initializer. Add this wiring alongside the other callback setups (around line 306-310 area, near where windowModel callbacks are set):

```swift
        fileTreeView.onTreeRefreshed = { [weak self] nodes in
            guard let self else { return }
            self.windowModel.updateFileTree(nodes, rootPath: self.fileTreeRootPath)
            self.fileTreeCacheInsert(key: self.fileTreeRootPath, nodes: nodes)
        }
```

Place this before the first call to `fileTreeView.updateTree()`.

- [ ] **Step 4: Build**

Run: `cd /Users/dowilcox/Code/impulse/impulse-macos && swift build 2>&1 | tail -10`
Expected: Build succeeds

- [ ] **Step 5: Commit**

```bash
git add impulse-macos/Sources/ImpulseApp/Sidebar/FileTreeView.swift impulse-macos/Sources/ImpulseApp/MainWindow.swift
git commit -m "Sync FileTreeView filesystem watcher changes to WindowModel

The filesystem watcher rebuilt FileTreeView's internal rootNodes but never
updated windowModel.fileTreeNodes, so the SwiftUI sidebar missed file
changes until a manual refresh. Now fires onTreeRefreshed callback to
keep both in sync."
```

---

### Task 6: Build and smoke test

- [ ] **Step 1: Full build**

```bash
cd /Users/dowilcox/Code/impulse
cargo build -p impulse-core -p impulse-editor -p impulse-ffi
cd impulse-macos
swift build
```

Expected: All build successfully with no errors or warnings.

- [ ] **Step 2: Build the .app bundle**

```bash
cd /Users/dowilcox/Code/impulse
./impulse-macos/build.sh --dev
```

Expected: `.app` bundle builds successfully.

- [ ] **Step 3: Manual smoke test**

Launch the dev build and verify:

1. **File tree renders correctly** — files and folders appear with icons and git status badges
2. **Expand/collapse works** — clicking folders expands and collapses them with animation
3. **Many folders expanded** — expand 20+ folders, verify no visible lag when scrolling or expanding more
4. **Git status updates** — edit a file externally, verify git status badge updates within ~2 seconds
5. **Filesystem changes** — create/delete a file externally (e.g., `touch /path/to/project/test.txt`), verify the SwiftUI sidebar shows the change within ~1 second
6. **Context menu** — right-click files and folders, verify all actions work
7. **Drag and drop** — drag files between folders, verify they move correctly
8. **Active file highlight** — open a file in the editor, verify it's highlighted in the sidebar
9. **Collapse All** — use the toolbar button, verify all folders collapse

- [ ] **Step 4: Commit any fixes if needed**

If the smoke test reveals issues, fix them and commit.
