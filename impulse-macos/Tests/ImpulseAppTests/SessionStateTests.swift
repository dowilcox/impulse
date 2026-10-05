#if canImport(Testing)
  import Foundation
  @testable import ImpulseApp
  import ImpulseKit
  import Testing

  struct SessionStateTests {
    private func decode(_ json: String) throws -> SessionState {
      try JSONDecoder().decode(SessionState.self, from: Data(json.utf8))
    }

    @Test func versionOneWindowBecomesAScratchWorkspace() throws {
      let json = """
        {
          "version": 1,
          "active_window_index": 0,
          "windows": [{
            "project_root": "/tmp/project",
            "active_tab_index": 2,
            "layout": {"kind": "tab_group", "tab_indices": [0, 1, 2], "active_tab_index": 2},
            "tabs": [
              {"kind": "terminal", "cwd": "/tmp/a", "title": "fish", "shell": "fish", "pinned": true},
              {"kind": "editor", "pinned": false},
              {"kind": "terminal", "cwd": "/tmp/b", "pinned": false,
               "panes": [{"cwd": "/tmp/b1"}, {"cwd": "/tmp/b2"}], "active_pane_index": 1,
               "pane_layout": {"kind": "split", "axis": "horizontal", "ratio": 0.5,
                 "first": {"kind": "pane", "pane_index": 0},
                 "second": {"kind": "pane", "pane_index": 1}}}
            ]
          }]
        }
        """
      var state = try decode(json)
      // load() migrates; do the same here without touching disk.
      state.windows = state.windows.map { $0.migratedFromV1() }
      let workspaces = try #require(state.windows.first?.workspaces)
      #expect(workspaces.count == 1)
      let scratch = workspaces[0]
      #expect(scratch.kind == "scratch")
      #expect(scratch.fileTreeRoot == "/tmp/project")
      // The path-less editor tab is dropped; the split keeps its active pane.
      #expect(scratch.tabs.count == 2)
      #expect(scratch.tabs[0].pinned)
      #expect(scratch.tabs[0].panes.first?.cwd == "/tmp/a")
      #expect(scratch.tabs[1].panes.first?.cwd == "/tmp/b2")
      #expect(scratch.tabs[1].layout == nil)
      // Active tab 2 of the old list is the second surviving tab.
      #expect(scratch.activeTabIndex == 1)
    }

    @Test func versionTwoRoundTripsWorkspacesAndSplits() throws {
      let layout = LayoutTree<Int>.leaf(0)
        .splitting(0, with: 1, axis: .horizontal)
        .splitting(1, with: 2, axis: .vertical)
      let window = SessionWindowState(
        workspaces: [
          SessionWorkspaceState(
            kind: "folder", root: "/tmp/repo", name: "Repo", expanded: true,
            tabs: [
              SessionTab(
                pinned: false,
                panes: [
                  .terminal(cwd: "/tmp/repo", title: nil, shell: "zsh"),
                  .file(path: "/tmp/repo/README.md", line: 12, column: 3),
                  .terminal(cwd: "/tmp/repo/src", title: "build", shell: "zsh"),
                ],
                layout: layout, focusedPane: 2)
            ],
            activeTabIndex: 0, fileTreeRoot: nil)
        ],
        activeWorkspaceIndex: 0, frame: "{{10, 20}, {1200, 800}}", sidebarVisible: true,
        sidebarWidth: 260)
      let state = SessionState.snapshot(windows: [window], activeWindowIndex: 0)

      let data = try JSONEncoder().encode(state)
      let decoded = try JSONDecoder().decode(SessionState.self, from: data)
      #expect(decoded.version == 2)
      let workspace = try #require(decoded.windows.first?.workspaces?.first)
      #expect(workspace.name == "Repo")
      #expect(workspace.expanded == true)
      let tab = try #require(workspace.tabs.first)
      #expect(tab.layout == layout)
      #expect(tab.focusedPane == 2)
      #expect(tab.panes[1].line == 12)
      #expect(tab.panes[1].kind == "file")
      #expect(decoded.windows.first?.frame == "{{10, 20}, {1200, 800}}")
      #expect(decoded.windows.first?.tabs == nil)
    }
  }
#endif
