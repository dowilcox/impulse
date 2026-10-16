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

    @Test func autosaveKeyChangesWithWorkspacesNotTitlesOrCursors() throws {
      func window(title: String, line: Int, activeTab: Int, tasks: [String]) -> SessionWindowState {
        let tabs = [
          SessionTab(pinned: false, panes: [.terminal(cwd: "/tmp/repo", title: title, shell: "zsh")]),
          SessionTab(pinned: false, panes: [.file(path: "/tmp/repo/README.md", line: line, column: 1)]),
        ]
        let main = SessionWorkspaceState(
          kind: "folder", root: "/tmp/repo", tabs: tabs, activeTabIndex: activeTab, fileTreeRoot: nil)
        let taskWorkspaces = tasks.map {
          SessionWorkspaceState(kind: "folder", root: $0, tabs: [], activeTabIndex: nil, fileTreeRoot: nil)
        }
        return SessionWindowState(
          workspaces: [main] + taskWorkspaces, activeWorkspaceIndex: 0, frame: nil, sidebarVisible: true,
          sidebarWidth: 260)
      }
      let saved = try #require(SessionState.autosaveKey([window(title: "✳ claude", line: 1, activeTab: 0, tasks: [])]))
      // An agent's spinning title, a moved cursor or another tab selected: nothing to save.
      #expect(SessionState.autosaveKey([window(title: "⠂ claude", line: 40, activeTab: 1, tasks: [])]) == saved)
      // A new task workspace is.
      #expect(SessionState.autosaveKey([window(title: "✳ claude", line: 1, activeTab: 0, tasks: ["/tmp/repo.worktrees/fix"])]) != saved)
    }
  }

  struct SessionScrollbackTests {
    private func scratchDirectory() throws -> URL {
      let dir = FileManager.default.temporaryDirectory.appendingPathComponent("impulse-session-\(UUID().uuidString)")
      try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
      return dir
    }

    private func session(_ transcripts: [String]) -> SessionState {
      let tabs = transcripts.map {
        SessionTab(pinned: false, panes: [.terminal(cwd: "/tmp", title: nil, shell: "zsh", transcript: $0)])
      }
      let workspace = SessionWorkspaceState(
        kind: "scratch", root: "/tmp", tabs: tabs, activeTabIndex: 0, fileTreeRoot: nil)
      let window = SessionWindowState(
        workspaces: [workspace], activeWorkspaceIndex: 0, frame: nil, sidebarVisible: nil, sidebarWidth: nil)
      return SessionState.snapshot(windows: [window], activeWindowIndex: 0)
    }

    private func scrollbackNames(_ state: SessionState) -> [String?] {
      state.windows.flatMap { $0.workspaces ?? [] }.flatMap(\.tabs).flatMap(\.panes).map(\.scrollback)
    }

    private func files(in directory: URL) -> Set<String> {
      Set((try? FileManager.default.contentsOfDirectory(atPath: directory.path)) ?? [])
    }

    @Test func unchangedOutputIsNotWrittenAgain() throws {
      let dir = try scratchDirectory()
      defer { try? FileManager.default.removeItem(at: dir) }
      var first = session(["$ ls\nREADME.md\n", "$ make\nok\n"])
      SessionScrollback.store(&first, in: dir)
      let names = scrollbackNames(first)
      #expect(names.allSatisfy { $0 != nil })
      let quiet = dir.appendingPathComponent(try #require(names[0]))
      let stamp = try FileManager.default.attributesOfItem(atPath: quiet.path)[.systemFileNumber] as? Int

      // The first terminal printed nothing new; the second did.
      var second = session(["$ ls\nREADME.md\n", "$ make\nok\n$ make test\npassed\n"])
      let kept = SessionScrollback.store(&second, in: dir)
      let secondNames = scrollbackNames(second)
      #expect(secondNames[0] == names[0])
      #expect(secondNames[1] != names[1])
      #expect(try FileManager.default.attributesOfItem(atPath: quiet.path)[.systemFileNumber] as? Int == stamp)
      // Restore finds every terminal's output.
      var restored = try #require(second.windows[0].workspaces)
      for t in restored[0].tabs.indices { restored[0].tabs[t].panes[0].transcript = nil }
      SessionScrollback.load(into: &restored, from: dir)
      #expect(
        restored[0].tabs.map { $0.panes[0].transcript } == ["$ ls\nREADME.md\n", "$ make\nok\n$ make test\npassed\n"])

      // Until the new session is written, the old one's files stay.
      #expect(files(in: dir).contains(try #require(names[1])))
      SessionScrollback.removeFiles(except: kept, in: dir)
      #expect(files(in: dir) == Set(secondNames.compactMap { $0 }))
    }

    @Test func oldOutputStaysWhenTheSessionFileCannotBeWritten() throws {
      let dir = try scratchDirectory()
      defer { try? FileManager.default.removeItem(at: dir) }
      let scrollback = dir.appendingPathComponent("scrollback")
      let sessionFile = dir.appendingPathComponent("session-state.json")
      SessionState.persist(session(["one\n"]), to: sessionFile, scrollbackDirectory: scrollback, synchronously: true)
      let saved = try #require(SessionState.load(from: sessionFile))
      let before = files(in: scrollback)
      #expect(before == Set(scrollbackNames(saved).compactMap { $0 }))

      // The session file can't be written (a folder is in its way): the
      // previous session's output must survive.
      let blocked = dir.appendingPathComponent("blocked")
      try FileManager.default.createDirectory(
        at: blocked.appendingPathComponent("inside"), withIntermediateDirectories: true)
      SessionState.persist(session(["two\n"]), to: blocked, scrollbackDirectory: scrollback, synchronously: true)
      #expect(files(in: scrollback).isSuperset(of: before))

      SessionState.persist(session(["two\n"]), to: sessionFile, scrollbackDirectory: scrollback, synchronously: true)
      let resaved = try #require(SessionState.load(from: sessionFile))
      #expect(files(in: scrollback) == Set(scrollbackNames(resaved).compactMap { $0 }))
      #expect(files(in: scrollback).isDisjoint(with: before))
    }
  }
#endif
