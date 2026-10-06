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
#endif
