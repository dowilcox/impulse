#if canImport(Testing)
  import Foundation
  @testable import ImpulseApp
  import enum ImpulseKit.DirectoryLister
  import Testing

  struct FileTreeApplyTests {
    private func scratchDirectory() throws -> String {
      let dir = FileManager.default.temporaryDirectory.appendingPathComponent("impulse-tree-\(UUID().uuidString)")
      try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
      // The real path (/private/var): directory listings report it.
      return TextFile.target(of: dir.path)
    }

    private func write(_ path: String, _ text: String = "x\n") throws {
      try text.write(toFile: path, atomically: true, encoding: .utf8)
    }

    /// Patch a listed folder after changes on disk; the result must match a
    /// fresh listing, keeping the nodes that weren't replaced.
    @Test func patchedChildrenMatchTheFolder() throws {
      let root = try scratchDirectory()
      defer { try? FileManager.default.removeItem(atPath: root) }
      try FileManager.default.createDirectory(atPath: root + "/lib", withIntermediateDirectories: true)
      for name in ["b.txt", "d.txt", "f.txt", "h.txt"] { try write(root + "/" + name) }
      let before = FileTreeNode.buildChildren(path: root, showHidden: false)

      // New files ahead of unchanged ones, one removed, one edited.
      for name in ["a.txt", "c.txt", "g.txt"] { try write(root + "/" + name) }
      try FileManager.default.removeItem(atPath: root + "/d.txt")
      try write(root + "/f.txt", "longer content\n")

      let batch = try #require(
        ImpulseCore.buildFileTreePatchBatch(
          rootPath: root,
          events: [ImpulseCore.FileTreeWatchEvent(kind: "any", paths: [root])],
          beforeByParent: [root: before.map { $0.fileEntrySnapshot() }],
          showHidden: false))
      let patch = try #require(batch.patches.first { $0.parent_id == root })
      // b.txt and h.txt only shifted: the patch leaves them out.
      let named = Set(
        patch.operations.map { operation -> String in
          switch operation {
          case .remove(let id): return id
          case .upsert(_, _, let node): return node.id
          }
        })
      #expect(named.isSuperset(of: ["a.txt", "c.txt", "d.txt", "f.txt", "g.txt"].map { root + "/" + $0 }))
      #expect(named.isDisjoint(with: [root + "/b.txt", root + "/h.txt"]))

      let after = FileTreeDataController.applying(patch.operations, to: before)
      let listed = try DirectoryLister.readDirectoryEntries(path: root, showHidden: false)
      #expect(after.map(\.name) == listed.map(\.name))
      #expect(after.first { $0.name == "f.txt" }?.size == 15)
      for name in ["lib", "b.txt", "f.txt", "h.txt"] {
        #expect(after.first { $0.name == name } === before.first { $0.name == name }, "\(name)")
      }
    }

    @Test func aBuiltTreeComesWithItsOpenFoldersListed() throws {
      let root = try scratchDirectory()
      defer { try? FileManager.default.removeItem(atPath: root) }
      try FileManager.default.createDirectory(atPath: root + "/src/lib", withIntermediateDirectories: true)
      try FileManager.default.createDirectory(atPath: root + "/docs", withIntermediateDirectories: true)
      try write(root + "/src/lib/a.swift")

      let nodes = FileTreeDataController.buildTree(
        rootPath: root, showHidden: false, expanding: [root + "/src", root + "/src/lib"])
      let src = try #require(nodes.first { $0.name == "src" })
      #expect(src.isExpanded)
      let lib = try #require(src.children?.first { $0.name == "lib" })
      #expect(lib.isExpanded)
      #expect(lib.children?.map(\.name) == ["a.swift"])
      // Closed folders stay unlisted.
      #expect(nodes.first { $0.name == "docs" }?.isLoaded == false)
    }

    @Test func aMoveWithinTheFolderKeepsEveryNode() throws {
      let root = "/repo"
      func node(_ name: String) -> FileTreeNode {
        FileTreeNode(name: name, path: root + "/" + name, isDirectory: false)
      }
      func patchNode(_ name: String) -> ImpulseCore.FileTreePatchNode {
        ImpulseCore.FileTreePatchNode(
          id: root + "/" + name, parent_id: root, name: name, path: root + "/" + name, is_dir: false,
          is_symlink: false, size: 0, modified: 0, git_status: nil)
      }
      let before = ["a", "b", "c"].map(node)
      // Reordered (not just shifted): every moved entry is named.
      let after = FileTreeDataController.applying(
        [
          .upsert(parentId: root, index: 0, node: patchNode("c")),
          .upsert(parentId: root, index: 2, node: patchNode("a")),
        ], to: before)
      #expect(after.map(\.name) == ["c", "b", "a"])
      #expect(after[0] === before[2])
      // An index past the end lands at the end.
      let appended = FileTreeDataController.applying(
        [.upsert(parentId: root, index: 9, node: patchNode("z"))], to: before)
      #expect(appended.map(\.name) == ["a", "b", "c", "z"])
    }

    /// Drops: a project file moves, anything else (even beside it) copies.
    @Test func aSiblingFolderSharingThePrefixIsNotInside() {
      #expect(FileDropHelper.isInside("/Users/me/Code/app/src/main.rs", projectRoot: "/Users/me/Code/app"))
      #expect(FileDropHelper.isInside("/Users/me/Code/app", projectRoot: "/Users/me/Code/app/"))
      #expect(!FileDropHelper.isInside("/Users/me/Code/app-old/main.rs", projectRoot: "/Users/me/Code/app"))
      #expect(!FileDropHelper.isInside("/Users/me/Code/application", projectRoot: "/Users/me/Code/app"))
    }
  }
#endif
