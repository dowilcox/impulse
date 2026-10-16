#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct LSPCompletionTests {
    private func parse(_ text: String) -> [LSPCompletionItem] {
      LSPCompletion.parse(try? JSONSerialization.jsonObject(with: Data(text.utf8)))
    }

    private func range(_ line: Int, _ start: Int, _ end: Int) -> LSPRange {
      LSPRange(startLine: line, startCharacter: start, endLine: line, endCharacter: end)
    }

    @Test func plainItemsInsertTheirTextOrLabel() {
      let items = parse(
        """
        [{"label": "println!", "insertText": "println!($0)", "insertTextFormat": 2, "kind": 3},
         {"label": "len", "detail": "fn len(&self) -> usize"}]
        """)
      #expect(
        items == [
          LSPCompletionItem(label: "println!", kind: 3, text: "println!($0)", isSnippet: true),
          LSPCompletionItem(label: "len", detail: "fn len(&self) -> usize", text: "len"),
        ])
    }

    @Test func textEditGivesTheTextAndRange() {
      // rust-analyzer: the label is for show, the edit says what goes in.
      let items = parse(
        """
        {"isIncomplete": false, "items": [{
          "label": "push(…)", "kind": 2, "insertTextFormat": 2,
          "textEdit": {"range": {"start": {"line": 3, "character": 6}, "end": {"line": 3, "character": 8}}, "newText": "push(${1:value})$0"},
          "additionalTextEdits": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "newText": "use std::vec::Vec;\\n"}],
          "sortText": "ffffffef", "filterText": "push"
        }]}
        """)
      #expect(
        items == [
          LSPCompletionItem(
            label: "push(…)", kind: 2, text: "push(${1:value})$0", isSnippet: true, range: range(3, 6, 8),
            additionalTextEdits: [
              LSPTextEdit(startLine: 0, startCharacter: 0, endLine: 0, endCharacter: 0, newText: "use std::vec::Vec;\n")
            ], sortText: "ffffffef", filterText: "push")
        ])
    }

    @Test func insertReplaceEditKeepsBothRanges() {
      let items = parse(
        """
        [{"label": "length", "textEdit": {"newText": "length",
          "insert": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 6}},
          "replace": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 10}}}}]
        """)
      #expect(items.first?.text == "length")
      #expect(items.first?.range == range(1, 4, 6))
      #expect(items.first?.replaceRange == range(1, 4, 10))
    }

    @Test func listDefaultsFillInItems() {
      let items = parse(
        """
        {"items": [
          {"label": "alpha"},
          {"label": "beta", "textEditText": "beta()"},
          {"label": "gamma", "insertText": "gamma[]", "insertTextFormat": 1},
          {"label": "delta", "textEdit": {"range": {"start": {"line": 2, "character": 0}, "end": {"line": 2, "character": 1}}, "newText": "delta"}}
        ], "itemDefaults": {
          "editRange": {"start": {"line": 2, "character": 3}, "end": {"line": 2, "character": 5}},
          "insertTextFormat": 2
        }}
        """)
      #expect(items.map(\.text) == ["alpha", "beta()", "gamma[]", "delta"])
      #expect(items.map(\.range) == [range(2, 3, 5), range(2, 3, 5), range(2, 3, 5), range(2, 0, 1)])
      #expect(items.map(\.isSnippet) == [true, true, false, true])
    }

    @Test func defaultInsertReplaceRange() {
      let items = parse(
        """
        {"items": [{"label": "x"}], "itemDefaults": {"editRange": {
          "insert": {"start": {"line": 0, "character": 1}, "end": {"line": 0, "character": 2}},
          "replace": {"start": {"line": 0, "character": 1}, "end": {"line": 0, "character": 4}}}}}
        """)
      #expect(items.first?.range == range(0, 1, 2))
      #expect(items.first?.replaceRange == range(0, 1, 4))
    }

    @Test func rangesTheEditorCantUseAreLeftOut() {
      // Over two lines: the text still goes in, at the word being typed.
      let items = parse(
        """
        [{"label": "a", "textEdit": {"range": {"start": {"line": 0, "character": 3}, "end": {"line": 1, "character": 0}}, "newText": "alpha"}},
         {"label": "b", "textEdit": {"newText": "beta",
           "insert": {"start": {"line": 0, "character": 2}, "end": {"line": 0, "character": 3}},
           "replace": {"start": {"line": 0, "character": 1}, "end": {"line": 0, "character": 5}}}}]
        """)
      #expect(items.map(\.text) == ["alpha", "beta"])
      #expect(items[0].range == nil)
      #expect(items[1].range == range(0, 2, 3), "insert kept")
      #expect(items[1].replaceRange == nil, "replace doesn't start where insert does")
    }

    @Test func malformedAnswersHaveNoItems() {
      #expect(parse("null").isEmpty)
      #expect(parse("{\"error\": \"no LSP client available\"}").isEmpty)
      #expect(parse("[{\"kind\": 1}]").isEmpty, "no label")
    }
  }
#endif
