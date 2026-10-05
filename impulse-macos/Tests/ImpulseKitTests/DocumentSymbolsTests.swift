#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct DocumentSymbolsTests {
    @Test func hierarchicalSymbolsFlattenInDocumentOrder() {
      let json = #"""
        [{"name": "Search", "kind": 23, "detail": "",
          "range": {"start": {"line": 0, "character": 0}, "end": {"line": 9, "character": 1}},
          "selectionRange": {"start": {"line": 0, "character": 7}, "end": {"line": 0, "character": 13}},
          "children": [
            {"name": "run", "kind": 6, "detail": "(String) -> [String]",
             "range": {"start": {"line": 5, "character": 2}, "end": {"line": 8, "character": 3}},
             "selectionRange": {"start": {"line": 5, "character": 7}, "end": {"line": 5, "character": 10}}},
            {"name": "index", "kind": 7,
             "range": {"start": {"line": 1, "character": 2}, "end": {"line": 1, "character": 20}},
             "selectionRange": {"start": {"line": 1, "character": 6}, "end": {"line": 1, "character": 11}}}]},
         {"name": "main", "kind": 12,
          "range": {"start": {"line": 11, "character": 0}, "end": {"line": 12, "character": 1}},
          "selectionRange": {"start": {"line": 11, "character": 5}, "end": {"line": 11, "character": 9}}}]
        """#
      let symbols = DocumentSymbols.parse(Data(json.utf8))
      #expect(symbols.map(\.name) == ["Search", "index", "run", "main"])
      #expect(symbols.map(\.depth) == [0, 1, 1, 0])
      #expect(symbols[2].container == ["Search"])
      #expect(symbols[2].line == 6 && symbols[2].column == 8, "1-based, from the selection range")
      #expect(symbols[2].detail == "(String) -> [String]")
      #expect(symbols[0].detail == nil, "empty details are dropped")
      #expect(symbols[0].kindName == "struct")
      #expect(symbols[3].kindName == "function")
    }

    @Test func flatSymbolInformationUsesContainerNames() {
      let json = #"""
        [{"name": "helper", "kind": 12, "containerName": "Utils",
          "location": {"uri": "file:///a.py", "range": {"start": {"line": 4, "character": 4}, "end": {"line": 6, "character": 0}}}},
         {"name": "Utils", "kind": 5,
          "location": {"uri": "file:///a.py", "range": {"start": {"line": 2, "character": 0}, "end": {"line": 9, "character": 0}}}}]
        """#
      let symbols = DocumentSymbols.parse(Data(json.utf8))
      #expect(symbols.map(\.name) == ["Utils", "helper"])
      #expect(symbols[1].container == ["Utils"])
      #expect(symbols[1].depth == 1)
      #expect(DocumentSymbols.parse(Data("null".utf8)).isEmpty)
      #expect(DocumentSymbols.kindName(99) == "symbol")
    }

    @Test func workspaceSymbolsCarryTheirFiles() {
      let json = #"""
        [{"name": "Search", "kind": 5, "containerName": "app",
          "location": {"uri": "file:///repo/src/search.ts", "range": {"start": {"line": 3, "character": 13}, "end": {"line": 3, "character": 19}}}},
         {"name": "lazy", "kind": 12, "location": {"uri": "file:///repo/a%20b.ts"}},
         {"name": "remote", "kind": 12, "location": {"uri": "https://example.com/x.ts"}}]
        """#
      let results = DocumentSymbols.parseWorkspace(Data(json.utf8))
      #expect(results.map(\.path) == ["/repo/src/search.ts", "/repo/a b.ts"])
      #expect(results[0].symbol.line == 4 && results[0].symbol.column == 14)
      #expect(results[0].symbol.container == ["app"])
      #expect(results[1].symbol.line == 1, "no range yet")
    }
  }
#endif