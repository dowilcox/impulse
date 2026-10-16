#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct WorkspaceEditTests {
    private func edit(_ l1: Int, _ c1: Int, _ l2: Int, _ c2: Int, _ text: String) -> LSPTextEdit {
      LSPTextEdit(startLine: l1, startCharacter: c1, endLine: l2, endCharacter: c2, newText: text)
    }

    private func json(_ text: String) -> Any? {
      try? JSONSerialization.jsonObject(with: Data(text.utf8))
    }

    @Test func appliesAgainstTheOriginalText() {
      let text = "let a = 1\nlet b = a\nprint(a)\n"
      let edits = [edit(0, 4, 0, 5, "count"), edit(1, 8, 1, 9, "count"), edit(2, 6, 2, 7, "count")]
      #expect(TextEditApplier.apply(edits, to: text) == "let count = 1\nlet b = count\nprint(count)\n")
      #expect(TextEditApplier.apply(edits.reversed(), to: text) == "let count = 1\nlet b = count\nprint(count)\n")
    }

    @Test func insertsAtOnePositionKeepTheirOrder() {
      let edits = [edit(0, 0, 0, 0, "import A\n"), edit(0, 0, 0, 0, "import B\n")]
      #expect(TextEditApplier.apply(edits, to: "code\n") == "import A\nimport B\ncode\n")
    }

    @Test func columnsAreUTF16AndClamp() {
      // "é" is one UTF-16 unit, "😀" two.
      #expect(TextEditApplier.apply([edit(0, 2, 0, 3, "X")], to: "😀éz") == "😀Xz")
      #expect(TextEditApplier.apply([edit(0, 99, 0, 99, "!")], to: "ab\ncd") == "ab!\ncd", "past the line end")
      #expect(TextEditApplier.apply([edit(9, 0, 9, 0, "!")], to: "ab") == "ab!", "past the document end")
      #expect(TextEditApplier.apply([edit(0, 1, 1, 1, "")], to: "ab\r\ncd") == "ad", "CRLF line breaks")
    }

    @Test func overlappingEditsAreRejected() {
      #expect(TextEditApplier.apply([edit(0, 0, 0, 3, "x"), edit(0, 2, 0, 4, "y")], to: "abcdef") == nil)
      #expect(TextEditApplier.apply([edit(0, 0, 0, 2, "x"), edit(0, 2, 0, 4, "y")], to: "abcdef") == "xyef")
    }

    @Test func parsesChangesSortedByURI() throws {
      let parsed = try #require(
        WorkspaceEdit.parse(
          json(
            """
            {"changes": {
              "file:///b.swift": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}}, "newText": "B"}],
              "file:///a.swift": [{"range": {"start": {"line": 1, "character": 2}, "end": {"line": 1, "character": 3}}, "newText": "A"}]
            }}
            """)))
      #expect(parsed.uris == ["file:///a.swift", "file:///b.swift"])
      #expect(parsed.textEdits.map(\.uri) == ["file:///a.swift", "file:///b.swift"])
      #expect(parsed.textEdits[0].edits == [edit(1, 2, 1, 3, "A")])
      #expect(!parsed.hasResourceOperations)
    }

    @Test func parsesDocumentChangesWithResourceOperations() throws {
      let parsed = try #require(
        WorkspaceEdit.parse(
          json(
            """
            {"documentChanges": [
              {"kind": "create", "uri": "file:///new.swift", "options": {"ignoreIfExists": true}},
              {"textDocument": {"uri": "file:///new.swift", "version": null},
               "edits": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "newText": "hi", "annotationId": "x"}]},
              {"kind": "rename", "oldUri": "file:///old.swift", "newUri": "file:///renamed.swift"},
              {"kind": "delete", "uri": "file:///gone", "options": {"recursive": true}}
            ]}
            """)))
      #expect(
        parsed.operations == [
          .create(uri: "file:///new.swift", overwrite: false, ignoreIfExists: true),
          .edit(uri: "file:///new.swift", edits: [edit(0, 0, 0, 0, "hi")]),
          .rename(oldUri: "file:///old.swift", newUri: "file:///renamed.swift", overwrite: false, ignoreIfExists: false),
          .delete(uri: "file:///gone", recursive: true, ignoreIfNotExists: false),
        ])
      #expect(parsed.uris == ["file:///new.swift", "file:///old.swift", "file:///renamed.swift", "file:///gone"])
      #expect(parsed.hasResourceOperations)
      #expect(parsed.versions.isEmpty, "a null version is any version")
    }

    @Test func keepsTheVersionsEditsWereComputedFor() throws {
      let parsed = try #require(
        WorkspaceEdit.parse(
          json(
            """
            {"documentChanges": [
              {"textDocument": {"uri": "file:///a.ts", "version": 7}, "edits": []},
              {"textDocument": {"uri": "file:///b.ts"}, "edits": []}
            ]}
            """)))
      #expect(parsed.versions == ["file:///a.ts": 7])
    }

    @Test func openDocumentsAreSharedAcrossWindows() {
      var documents = LSPOpenDocuments()
      let uri = "file:///a.ts"
      let first = documents.open(uri)
      let second = documents.open(uri)
      #expect(first && !second, "only the first window sends didOpen")
      #expect(documents.holders(uri) == 2)
      let versions = [documents.nextVersion(uri), documents.nextVersion(uri)]
      #expect(versions == [2, 3], "one sequence for both windows")
      let closedOne = documents.close(uri)
      #expect(!closedOne, "the other window still shows it")
      #expect(documents.version(uri) == 3)
      let closedLast = documents.close(uri)
      #expect(closedLast, "the last window sends didClose")
      let afterClose = documents.nextVersion(uri)
      #expect(documents.version(uri) == nil && afterClose == nil)
      let reopened = documents.open(uri)
      #expect(reopened && documents.version(uri) == 1, "reopened from the start")
    }

    @Test func emptyAndMalformedEdits() {
      #expect(WorkspaceEdit.parse(json("{}"))?.operations == [])
      #expect(WorkspaceEdit.parse("nope") == nil)
      #expect(LSPTextEdit(json: ["range": "bad", "newText": "x"]) == nil)
    }
  }

  struct WorkspaceEditConfirmationTests {
    /// A Cargo workspace: two crates in one repository, and a folder beside
    /// the repository.
    private func makeRepository() throws -> (repo: String, outside: String, cleanup: () -> Void) {
      let base = NSTemporaryDirectory() + "impulse-edit-bounds-" + UUID().uuidString
      let repo = base + "/repo"
      let files = FileManager.default
      for folder in ["/.git", "/crates/app/src", "/crates/lib/src"] {
        try files.createDirectory(atPath: repo + folder, withIntermediateDirectories: true)
      }
      for crate in ["app", "lib"] {
        try "".write(toFile: "\(repo)/crates/\(crate)/Cargo.toml", atomically: true, encoding: .utf8)
        try "".write(toFile: "\(repo)/crates/\(crate)/src/lib.rs", atomically: true, encoding: .utf8)
      }
      try files.createDirectory(atPath: base + "/other", withIntermediateDirectories: true)
      try "".write(toFile: base + "/other/notes.rs", atomically: true, encoding: .utf8)
      return (repo, base + "/other", { try? files.removeItem(atPath: base) })
    }

    private let insert = [LSPTextEdit(startLine: 0, startCharacter: 0, endLine: 0, endCharacter: 0, newText: "x")]

    private func uri(_ path: String) -> String { URL(fileURLWithPath: path).absoluteString }
    private func path(_ uri: String) -> String { URL(string: uri)!.path }

    @Test func aRenameAcrossCratesDoesntAsk() throws {
      let (repo, outside, cleanup) = try makeRepository()
      defer { cleanup() }
      // Renaming a `pub fn` used by the sibling crate: inside the repository.
      let rename = WorkspaceEdit(operations: [
        .edit(uri: uri(repo + "/crates/lib/src/lib.rs"), edits: insert),
        .edit(uri: uri(repo + "/crates/app/src/lib.rs"), edits: insert),
      ])
      #expect(rename.confirmations(within: repo, path: path).isEmpty)
      // Bounded by the crate (a server's own change), the sibling asks.
      #expect(
        rename.confirmations(within: repo + "/crates/lib", path: path) == [
          .changes(path: repo + "/crates/app/src/lib.rs")
        ])
      // Past the repository, and through `..`, asks either way.
      let reaching = WorkspaceEdit(operations: [
        .edit(uri: uri(outside + "/notes.rs"), edits: insert),
        .edit(uri: uri(repo + "/crates/../../other/notes.rs"), edits: insert),
        .edit(uri: uri(repo + "/crates/app/src/lib.rs"), edits: []),
      ])
      #expect(
        reaching.confirmations(within: repo, path: path) == [
          .changes(path: outside + "/notes.rs"), .changes(path: repo + "/crates/../../other/notes.rs"),
        ])
      // Without a boundary, every changed file asks.
      #expect(rename.confirmations(within: nil, path: path).count == 2)
    }

    @Test func aLinkOutOfTheRepositoryIsOutside() throws {
      let (repo, outside, cleanup) = try makeRepository()
      defer { cleanup() }
      try FileManager.default.createSymbolicLink(atPath: repo + "/linked", withDestinationPath: outside)
      #expect(!WorkspaceEdit.isInside(repo + "/linked/notes.rs", repo))
      #expect(!WorkspaceEdit.isInside(repo + "/linked/new.rs", repo), "a file still to be created, through the link")
      #expect(WorkspaceEdit.isInside(repo + "/crates/app/new.rs", repo))
    }

    @Test func filesCreatedMovedOrDeletedAlwaysAsk() throws {
      let (repo, _, cleanup) = try makeRepository()
      defer { cleanup() }
      let lib = repo + "/crates/lib/src/lib.rs"
      let edit = WorkspaceEdit(operations: [
        .create(uri: uri(repo + "/crates/lib/src/new.rs"), overwrite: false, ignoreIfExists: false),
        .create(uri: uri(lib), overwrite: true, ignoreIfExists: false),
        .create(uri: uri(lib), overwrite: false, ignoreIfExists: true),
        .rename(oldUri: uri(lib), newUri: uri(repo + "/crates/lib/src/core.rs"), overwrite: false, ignoreIfExists: false),
        .delete(uri: uri(repo + "/crates/app"), recursive: true, ignoreIfNotExists: false),
        .delete(uri: uri(repo + "/crates/app/src/lib.rs"), recursive: false, ignoreIfNotExists: false),
      ])
      #expect(
        edit.confirmations(within: repo, path: path) == [
          .creates(path: repo + "/crates/lib/src/new.rs"),
          .empties(path: lib),
          .moves(from: lib, to: repo + "/crates/lib/src/core.rs"),
          .deletes(path: repo + "/crates/app", withContents: true),
          .deletes(path: repo + "/crates/app/src/lib.rs", withContents: false),
        ])
    }

    @Test func theBoundaryForTheUsersOwnEdits() throws {
      let (repo, _, cleanup) = try makeRepository()
      defer { cleanup() }
      let crate = repo + "/crates/app"
      #expect(
        WorkspaceEdit.userEditBoundary(trustedFolder: repo, repositoryRoot: repo, serverRoot: crate) == repo)
      // A trusted parent of the repository: the repository.
      let parent = (repo as NSString).deletingLastPathComponent
      #expect(
        WorkspaceEdit.userEditBoundary(trustedFolder: parent, repositoryRoot: repo, serverRoot: crate) == repo)
      // Only a crate trusted: the crate, not the rest of the repository.
      #expect(
        WorkspaceEdit.userEditBoundary(trustedFolder: crate, repositoryRoot: repo, serverRoot: crate) == crate)
      #expect(WorkspaceEdit.userEditBoundary(trustedFolder: parent, repositoryRoot: nil, serverRoot: crate) == parent)
      #expect(WorkspaceEdit.userEditBoundary(trustedFolder: nil, repositoryRoot: repo, serverRoot: crate) == repo)
      #expect(WorkspaceEdit.userEditBoundary(trustedFolder: nil, repositoryRoot: nil, serverRoot: crate) == crate)
    }
  }
#endif
