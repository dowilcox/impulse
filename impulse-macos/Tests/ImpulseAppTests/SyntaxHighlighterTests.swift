#if canImport(Testing)
  import Foundation
  @testable import ImpulseApp
  import Testing

  struct SyntaxHighlighterTests {
    typealias Span = SyntaxHighlighter.Span

    @Test func parsesNestedSpansEntitiesAndLines() {
      let html =
        "<span class=\"hljs-keyword\">let</span> x = <span class=\"hljs-string\">&quot;a&lt;b&quot;</span>\n"
        + "<span class=\"hljs-comment\">/* one\ntwo */</span> <span class=\"hljs-title function_\">f</span>()"
      let lines = SyntaxHighlighter.parse(html: html, lineCount: 3)
      #expect(lines[0] == [Span(location: 0, length: 3, token: .keyword), Span(location: 8, length: 5, token: .string)])
      #expect(lines[1] == [Span(location: 0, length: 6, token: .comment)], "a comment crossing a line is split")
      #expect(lines[2] == [Span(location: 0, length: 6, token: .comment), Span(location: 7, length: 1, token: .function)])
    }

    @Test func innerSpansWithoutAKnownClassKeepTheOuterToken() {
      let html = "<span class=\"hljs-string\">&quot;<span class=\"hljs-subst-x\">${x}</span>&quot;</span>"
      #expect(SyntaxHighlighter.parse(html: html, lineCount: 1)[0] == [Span(location: 0, length: 6, token: .string)])
    }

    @Test func utf16Offsets() {
      let html = "😀 <span class=\"hljs-number\">1</span>"
      #expect(SyntaxHighlighter.parse(html: html, lineCount: 1)[0] == [Span(location: 3, length: 1, token: .number)])
    }

    @Test func languageNames() {
      #expect(SyntaxHighlighter.hljsLanguage("typescriptreact", path: "a.tsx") == "typescript")
      #expect(SyntaxHighlighter.hljsLanguage("shellscript", path: "x.sh") == "bash")
      #expect(SyntaxHighlighter.hljsLanguage("plaintext", path: "Sources/App.swift") == "swift")
      #expect(SyntaxHighlighter.hljsLanguage("plaintext", path: "notes.txt") == nil)
    }

    @Test func highlightsWithTheVendoredScript() throws {
      let script = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        .deletingLastPathComponent().appendingPathComponent("vendor/highlight/highlight.min.js")
      try #require(FileManager.default.fileExists(atPath: script.path))
      let highlighter = SyntaxHighlighter()
      highlighter.scriptURL = script
      let spans = try #require(
        highlighter.highlight(lines: ["func greet() -> String {", "  return \"hi\" // done", "}"], language: "swift"))
      #expect(spans.count == 3)
      #expect(spans[0].contains { $0.token == .keyword && $0.location == 0 && $0.length == 4 })
      #expect(spans[1].contains { $0.token == .string })
      #expect(spans[1].contains { $0.token == .comment })
      #expect(highlighter.highlight(lines: ["x"], language: "no-such-language") == nil)
    }

    /// Jobs run in turn on the highlighter's own queue, off the main thread.
    @Test func asyncJobsRunInTurnOffTheMainThread() async throws {
      let script = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        .deletingLastPathComponent().appendingPathComponent("vendor/highlight/highlight.min.js")
      try #require(FileManager.default.fileExists(atPath: script.path))
      let highlighter = SyntaxHighlighter()
      highlighter.scriptURL = script
      let results = await withCheckedContinuation { continuation in
        var order: [Int] = []
        for job in 0..<3 {
          highlighter.async { highlighter in
            order.append(job)
            guard job == 2 else { return }
            let spans = highlighter.highlightNow(lines: ["let x = 1"], language: "swift")
            continuation.resume(returning: (order, Thread.isMainThread, spans))
          }
        }
      }
      #expect(results.0 == [0, 1, 2])
      #expect(!results.1)
      #expect(results.2?.first?.contains { $0.token == .keyword } == true)
    }
  }
#endif
