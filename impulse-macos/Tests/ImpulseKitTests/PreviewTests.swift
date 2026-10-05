// Tests for the preview renderers (markdown via cmark-gfm safe mode, SVG
// sanitizer, CSS color validation), ported from the Rust unit tests in
// impulse-editor plus an XSS corpus for the new sanitization boundary.
#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct CSSColorTests {
    @Test func validHexColors() {
      #expect(sanitizeCSSColor("#abc", fallback: "x") == "#abc")
      #expect(sanitizeCSSColor("#aabbcc", fallback: "x") == "#aabbcc")
      #expect(sanitizeCSSColor("#aabbccdd", fallback: "x") == "#aabbccdd")
      #expect(sanitizeCSSColor("  #abc  ", fallback: "x") == "#abc")
    }

    @Test func invalidHexColors() {
      #expect(sanitizeCSSColor("#ab", fallback: "x") == "x")
      #expect(sanitizeCSSColor("#abcg", fallback: "x") == "x")
      #expect(sanitizeCSSColor("#aabbccdde", fallback: "x") == "x")
      #expect(sanitizeCSSColor("#", fallback: "x") == "x")
    }

    @Test func validRgbRgba() {
      #expect(sanitizeCSSColor("rgb(255, 0, 0)", fallback: "x") == "rgb(255, 0, 0)")
      #expect(sanitizeCSSColor("rgba(0, 0, 0, 0.5)", fallback: "x") == "rgba(0, 0, 0, 0.5)")
      #expect(sanitizeCSSColor("rgb(100%, 50%, 0%)", fallback: "x") == "rgb(100%, 50%, 0%)")
    }

    @Test func maliciousRgbRejected() {
      #expect(sanitizeCSSColor("rgb(0, 0, 0); background: url(evil)", fallback: "x") == "x")
      #expect(
        sanitizeCSSColor("rgb(0, 0, 0)</style><script>alert(1)</script>", fallback: "x") == "x")
    }

    @Test func emptyAndNamedColorsFallBack() {
      #expect(sanitizeCSSColor("", fallback: "x") == "x")
      #expect(sanitizeCSSColor("   ", fallback: "x") == "x")
      #expect(sanitizeCSSColor("red", fallback: "x") == "x")
      #expect(sanitizeCSSColor("transparent", fallback: "x") == "x")
    }
  }

  struct SVGPreviewTests {
    @Test func isSVGFileBasic() {
      #expect(SVGPreview.isSVGFile("image.svg"))
      #expect(SVGPreview.isSVGFile("IMAGE.SVG"))
      #expect(SVGPreview.isSVGFile("path/to/file.Svg"))
      #expect(!SVGPreview.isSVGFile("file.png"))
      #expect(!SVGPreview.isSVGFile("Makefile"))
      #expect(!SVGPreview.isSVGFile(""))
      #expect(SVGPreview.isSVGFile(".svg"))
    }

    @Test func renderBasic() {
      let html = SVGPreview.render(source: "<svg></svg>", bgColor: "#000000")!
      #expect(html.contains("<svg></svg>"))
      #expect(html.contains("background: #000000"))
      #expect(html.contains("connect-src 'none'"))
    }

    @Test func runButtonsAreOptIn() throws {
      let source = "```bash\nnpm install\n```\n"
      let plain = try #require(MarkdownPreview.render(source: source, theme: .fallback, highlightJSPath: ""))
      #expect(!plain.contains("impulseRun"))
      let runnable = try #require(
        MarkdownPreview.render(source: source, theme: .fallback, highlightJSPath: "", runButtons: true))
      #expect(runnable.contains("messageHandlers.impulseRun"))
      #expect(runnable.contains("language-bash"), "cmark tags fenced code with its language")
      #expect(runnable.contains("nonce=\"aW1wdWxzZVByZXZpZXc=\""), "allowed by the page's CSP")
    }

    @Test func renderOversized() {
      let big = String(repeating: "x", count: SVGPreview.maxSVGSize + 1)
      #expect(SVGPreview.render(source: big, bgColor: "#000") == nil)
    }

    @Test func renderSanitizesBgColor() {
      let html = SVGPreview.render(source: "<svg/>", bgColor: "evil;injection")!
      #expect(html.contains("background: #1a1b26"))
    }

    @Test func sanitizeStripsScriptTags() {
      let result = SVGPreview.sanitizeSVG(#"<svg><script>alert(1)</script><circle r="5"/></svg>"#)
      #expect(!result.contains("<script"))
      #expect(result.contains("<circle"))
    }

    @Test func sanitizeStripsForeignObject() {
      let result = SVGPreview.sanitizeSVG(
        #"<svg><foreignObject><div>evil</div></foreignObject></svg>"#)
      #expect(!result.contains("foreignObject"))
      #expect(!result.contains("evil"))
    }

    @Test func sanitizeStripsEventHandlers() {
      let result = SVGPreview.sanitizeSVG(
        #"<svg onload="alert(1)"><circle onclick="evil()" r="5"/></svg>"#)
      #expect(!result.contains("onload"))
      #expect(!result.contains("onclick"))
      #expect(result.contains(#"r="5""#))
    }

    @Test func sanitizeBlocksJavascriptURIs() {
      let result = SVGPreview.sanitizeSVG(#"<svg><a href="javascript:alert(1)">click</a></svg>"#)
      #expect(!result.contains("javascript:"))
    }

    @Test func sanitizePreservesSafeContent() {
      let input =
        #"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect width="100" height="100" fill="blue"/></svg>"#
      #expect(SVGPreview.sanitizeSVG(input) == input)
    }
  }

  struct MarkdownPreviewTests {
    private func testTheme() -> MarkdownThemeColors {
      var colors = MarkdownThemeColors.fallback
      colors.bg = "#101010"
      colors.fg = "#eeeeee"
      return colors
    }

    @Test func isMarkdownFileBasic() {
      #expect(MarkdownPreview.isMarkdownFile("README.md"))
      #expect(MarkdownPreview.isMarkdownFile("doc.MARKDOWN"))
      #expect(MarkdownPreview.isMarkdownFile("notes.mkdn"))
      #expect(!MarkdownPreview.isMarkdownFile("main.rs"))
      #expect(!MarkdownPreview.isMarkdownFile("Makefile"))
    }

    @Test func rendersBasicMarkdown() {
      let html = MarkdownPreview.render(
        source: "# Title\n\nBody", theme: testTheme(), highlightJSPath: "")!
      #expect(html.contains("<h1>Title</h1>"))
      #expect(!html.contains(#"src="""#))
      #expect(!html.contains("highlightAll"))
    }

    @Test func bootstrapsHighlightWithNonce() {
      let html = MarkdownPreview.render(
        source: "```rust\nfn main() {}\n```",
        theme: testTheme(),
        highlightJSPath: "file:///tmp/highlight.min.js")!
      #expect(html.contains("script-src file: 'nonce-aW1wdWxzZVByZXZpZXc='"))
      #expect(html.contains(#"src="file:///tmp/highlight.min.js""#))
      #expect(html.contains(#"nonce="aW1wdWxzZVByZXZpZXc=""#))
      #expect(html.contains("window.hljs"))
      #expect(html.contains("language-rust"))
    }

    @Test func rendersGfmTablesStrikethroughTasklists() {
      let html = MarkdownPreview.render(
        source: "| A | B |\n| - | - |\n| a | b |\n\n~~gone~~\n\n- [x] done\n- [ ] todo\n",
        theme: testTheme(), highlightJSPath: "")!
      #expect(html.contains("<table>"))
      #expect(html.contains("<del>gone</del>"))
      #expect(html.contains(#"type="checkbox""#))
    }

    @Test func oversizedSourceRefused() {
      let big = String(repeating: "x", count: MarkdownPreview.maxMarkdownSize + 1)
      #expect(MarkdownPreview.render(source: big, theme: testTheme(), highlightJSPath: "") == nil)
    }

    // XSS corpus: raw HTML must never reach the document (cmark safe mode
    // elides it), and dangerous URLs must be scrubbed.
    @Test func rawHTMLIsElided() {
      let cases = [
        "<script>alert(1)</script>",
        "before <img src=x onerror=alert(1)> after",
        "<iframe src=\"https://evil.example\"></iframe>",
        "<div onclick=\"evil()\">text</div>",
        "text <svg onload=alert(1)></svg>",
      ]
      for source in cases {
        let html = MarkdownPreview.render(
          source: source, theme: testTheme(), highlightJSPath: "")!
        #expect(!html.contains("<script>alert"), "raw script leaked for: \(source)")
        #expect(!html.contains("onerror"), "onerror leaked for: \(source)")
        #expect(!html.contains("<iframe"), "iframe leaked for: \(source)")
        #expect(!html.contains("onclick"), "onclick leaked for: \(source)")
        #expect(!html.contains("onload"), "onload leaked for: \(source)")
      }
    }

    @Test func dangerousLinkURLsScrubbed() {
      let html = MarkdownPreview.render(
        source: "[click](javascript:alert(1)) and ![img](data:text/html,<script>)",
        theme: testTheme(), highlightJSPath: "")!
      #expect(!html.contains("javascript:"))
      #expect(!html.contains("data:text/html"))
    }

    @Test func escapedHighlightPathCannotBreakAttribute() {
      let html = MarkdownPreview.render(
        source: "hi",
        theme: testTheme(),
        highlightJSPath: "file:///x\"></script><script>alert(1)</script>")!
      #expect(!html.contains("\"></script><script>alert(1)</script>"))
    }
  }
#endif
