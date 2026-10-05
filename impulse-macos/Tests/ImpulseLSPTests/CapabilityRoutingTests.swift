#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseLSP

  struct CapabilityRoutingTests {
    @Test func methodsMapToTheirCapability() {
      let typescript: [String: Any] = [
        "hoverProvider": true, "documentSymbolProvider": true, "completionProvider": ["triggerCharacters": ["."]],
        "renameProvider": ["prepareProvider": true],
      ]
      let eslint: [String: Any] = ["codeActionProvider": true, "hoverProvider": false]
      #expect(ServerProcess.supports(method: "textDocument/documentSymbol", capabilities: typescript))
      #expect(!ServerProcess.supports(method: "textDocument/documentSymbol", capabilities: eslint))
      #expect(ServerProcess.supports(method: "textDocument/completion", capabilities: typescript), "options object")
      #expect(!ServerProcess.supports(method: "textDocument/hover", capabilities: eslint), "explicit false")
      #expect(ServerProcess.supports(method: "textDocument/prepareRename", capabilities: typescript))
      #expect(!ServerProcess.supports(method: "textDocument/prepareRename", capabilities: ["renameProvider": true]))
      #expect(ServerProcess.supports(method: "custom/thing", capabilities: eslint), "unknown methods pass")
      #expect(!ServerProcess.supports(method: "textDocument/hover", capabilities: nil))
    }
  }
#endif
