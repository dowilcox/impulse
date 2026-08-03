// Tests for DirectoryLister + FileTreePatcher, ported from the Rust unit
// tests in file_tree.rs plus a golden-fixture reproduction of the exact
// scenario the Rust implementation was captured with.
#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  private func entry(_ name: String, _ path: String, dir: Bool, status: String? = nil)
    -> FileEntry
  {
    FileEntry(
      name: name, path: path, isDir: dir, isSymlink: false, size: 10, modified: 20,
      gitStatus: status)
  }

  struct FileTreePatcherTests {
    @Test func stableNodeIdMatchesFixture() throws {
      struct IdCase: Decodable {
        let path: String
        let id: String
      }
      let cases = try Fixtures.decode([IdCase].self, from: "stable_node_id.json")
      for c in cases {
        #expect(FileTreePatcher.stableNodeId(c.path) == c.id, "path=\(c.path)")
      }
    }

    @Test func unchangedChildrenEmitNoOperations() {
      let before = [entry("src", "/repo/src", dir: true), entry("main.rs", "/repo/main.rs", dir: false)]
      let patch = FileTreePatcher.buildChildPatch(
        parentPath: "/repo", before: before, after: before)
      #expect(patch.operations.isEmpty)
    }

    @Test func renameIsRemoveBeforeUpsert() {
      let patch = FileTreePatcher.buildChildPatch(
        parentPath: "/repo",
        before: [entry("old.rs", "/repo/old.rs", dir: false)],
        after: [entry("new.rs", "/repo/new.rs", dir: false)])
      #expect(patch.operations.count == 2)
      guard case .remove(let id) = patch.operations[0] else {
        Issue.record("expected remove first")
        return
      }
      #expect(id == "/repo/old.rs")
      guard case .upsert(_, let index, let node) = patch.operations[1] else {
        Issue.record("expected upsert second")
        return
      }
      #expect(index == 0)
      #expect(node.id == "/repo/new.rs")
    }

    @Test func metadataOrPositionChangesEmitUpsert() {
      let before = [entry("a.rs", "/repo/a.rs", dir: false), entry("b.rs", "/repo/b.rs", dir: false)]
      let after = [
        entry("b.rs", "/repo/b.rs", dir: false),
        entry("a.rs", "/repo/a.rs", dir: false, status: "M"),
      ]
      let patch = FileTreePatcher.buildChildPatch(parentPath: "/repo", before: before, after: after)
      #expect(patch.operations.count == 2)
      #expect(
        patch.operations.allSatisfy {
          if case .upsert = $0 { return true } else { return false }
        })
    }

    @Test func typeReplacementRemovesBeforeUpsert() {
      let patch = FileTreePatcher.buildChildPatch(
        parentPath: "/repo",
        before: [entry("target", "/repo/target", dir: true)],
        after: [entry("target", "/repo/target", dir: false)])
      #expect(patch.operations.count == 2)
      guard case .remove(let id) = patch.operations[0], id == "/repo/target",
        case .upsert(_, _, let node) = patch.operations[1], node.id == "/repo/target",
        !node.isDir
      else {
        Issue.record("unexpected operations: \(patch.operations)")
        return
      }
    }

    @Test func affectedParentPathsAreStableAndDeduped() {
      let events = [
        FileTreeWatchEvent(kind: .modify, paths: ["/repo/src/lib.rs"]),
        FileTreeWatchEvent(kind: .create, paths: ["/repo/src/main.rs"]),
        FileTreeWatchEvent(kind: .remove, paths: ["/tmp/outside.txt"]),
      ]
      let parents = FileTreePatcher.affectedParentPaths(rootPath: "/repo", events: events)
      #expect(parents == ["/repo", "/repo/src"])
    }

    @Test func batchRepresentsMoveAcrossParents() {
      let events = [
        FileTreeWatchEvent(kind: .rename, paths: ["/repo/src/item.rs", "/repo/tests/item.rs"])
      ]
      let batch = FileTreePatcher.buildPatchBatch(
        rootPath: "/repo",
        events: events,
        beforeByParent: [
          "/repo/src": [entry("item.rs", "/repo/src/item.rs", dir: false)],
          "/repo/tests": [],
        ],
        afterByParent: [
          "/repo/src": [],
          "/repo/tests": [entry("item.rs", "/repo/tests/item.rs", dir: false)],
        ])
      #expect(batch.patches.count == 2)
      guard case .remove(let id) = batch.patches[0].operations[0], id == "/repo/src/item.rs",
        case .upsert(_, _, let node) = batch.patches[1].operations[0],
        node.id == "/repo/tests/item.rs"
      else {
        Issue.record("unexpected batch: \(batch)")
        return
      }
    }

    @Test func loadedDirectoryEventRefreshesThatDirectoryToo() {
      let events = [FileTreeWatchEvent(kind: .modify, paths: ["/repo/src"])]
      let batch = FileTreePatcher.buildPatchBatch(
        rootPath: "/repo",
        events: events,
        beforeByParent: ["/repo/src": [entry("old.rs", "/repo/src/old.rs", dir: false)]],
        afterByParent: ["/repo/src": [entry("new.rs", "/repo/src/new.rs", dir: false)]])
      #expect(batch.patches.count == 1)
      #expect(batch.patches[0].parentId == "/repo/src")
      #expect(batch.patches[0].operations.count == 2)
    }
  }

  /// Reproduces the exact filesystem scenario the Rust fixture dumper ran and
  /// compares the resulting patch batch against the golden fixture.
  struct FileTreeFixtureScenarioTests {
    private func normalized(_ value: Any, root: String) -> Any {
      if var dict = value as? [String: Any] {
        dict.removeValue(forKey: "modified")
        // serde encodes Option::None as an explicit null; JSONEncoder omits
        // nil keys. Treat both as absence.
        dict = dict.filter { !($0.value is NSNull) }
        return dict.mapValues { normalized($0, root: root) }
      }
      if let array = value as? [Any] {
        return array.map { normalized($0, root: root) }
      }
      if let string = value as? String, string.contains(root) {
        return string.replacingOccurrences(of: root, with: "$ROOT")
      }
      return value
    }

    @Test func patchBatchMatchesRustFixture() throws {
      let fixture = try Fixtures.json("file_tree_patch.json") as! [String: Any]

      let fm = FileManager.default
      let scratch = URL(fileURLWithPath: NSTemporaryDirectory())
        .appendingPathComponent("impulse-swift-tree-\(ProcessInfo.processInfo.processIdentifier)")
      try? fm.removeItem(at: scratch)
      try fm.createDirectory(at: scratch, withIntermediateDirectories: true)
      defer { try? fm.removeItem(at: scratch) }
      let root = scratch.resolvingSymlinksInPath().path

      func write(_ rel: String, _ content: String) throws {
        try content.write(
          to: URL(fileURLWithPath: root).appendingPathComponent(rel), atomically: true,
          encoding: .utf8)
      }
      try fm.createDirectory(atPath: root + "/alpha", withIntermediateDirectories: true)
      try fm.createDirectory(atPath: root + "/zeta", withIntermediateDirectories: true)
      try write("alpha/a1.txt", "alpha one\n")
      try write("beta.txt", "beta\n")
      try write("zeta/z1.txt", "zeta one\n")
      try write(".hidden", "hidden\n")

      let beforeByParent: [String: [FileEntry]] = [
        root: try DirectoryLister.readDirectoryEntries(path: root, showHidden: false),
        root + "/alpha": try DirectoryLister.readDirectoryEntries(
          path: root + "/alpha", showHidden: false),
      ]

      try write("gamma.txt", "gamma\n")
      try fm.removeItem(atPath: root + "/beta.txt")
      try write("alpha/a2.txt", "alpha two\n")
      try write("alpha/a1.txt", "alpha one, edited\n")

      let events = [
        FileTreeWatchEvent(kind: .create, paths: [root + "/gamma.txt"]),
        FileTreeWatchEvent(kind: .remove, paths: [root + "/beta.txt"]),
        FileTreeWatchEvent(kind: .create, paths: [root + "/alpha/a2.txt"]),
        FileTreeWatchEvent(kind: .modify, paths: [root + "/alpha/a1.txt"]),
      ]

      let batch = FileTreePatcher.buildPatchBatchFromFilesystem(
        rootPath: root, events: events, beforeByParent: beforeByParent, showHidden: false)
      let batchData = try JSONEncoder().encode(batch)
      let batchJSON = try JSONSerialization.jsonObject(with: batchData)

      let got = normalized(batchJSON, root: root) as! NSDictionary
      let want = normalized(fixture["batch"]!, root: root) as! NSDictionary
      #expect(got == want, "patch batch diverges from Rust fixture")
    }
  }
#endif
