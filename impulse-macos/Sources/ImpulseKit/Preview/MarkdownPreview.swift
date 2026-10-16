import Foundation
import cmark_gfm
import cmark_gfm_extensions

// Ported from impulse-editor/src/markdown.rs, with one deliberate change:
// pulldown-cmark + the ammonia HTML sanitizer are replaced by cmark-gfm in
// safe mode. Safe mode elides raw HTML entirely and scrubs dangerous link
// URLs, which is a stronger boundary than sanitizing — the trade-off is that
// inline HTML in markdown files no longer renders in the preview.

public enum MarkdownPreview {
  /// Maximum markdown source size (in bytes) before preview is refused.
  static let maxMarkdownSize = 1024 * 1024  // 1 MB

  static let markdownExtensions: Set<String> = ["md", "markdown", "mdown", "mkd", "mkdn"]

  /// Check whether a file path is a markdown file based on its extension.
  /// Mirrors Rust's `rsplit('.')` semantics exactly.
  public static func isMarkdownFile(_ path: String) -> Bool {
    let ext = path.split(separator: ".", omittingEmptySubsequences: false).last ?? ""
    return markdownExtensions.contains(ext.lowercased())
  }

  /// Whether a path is previewable (markdown or SVG).
  public static func isPreviewableFile(_ path: String) -> Bool {
    isMarkdownFile(path) || SVGPreview.isSVGFile(path)
  }

  /// Render GitHub-flavored markdown to an HTML fragment using cmark-gfm in
  /// safe mode (raw HTML elided, unsafe URLs scrubbed) with the table,
  /// strikethrough, and tasklist extensions.
  static func renderHTMLBody(_ source: String) -> String {
    cmark_gfm_core_extensions_ensure_registered()

    guard let parser = cmark_parser_new(CMARK_OPT_DEFAULT) else { return "" }
    defer { cmark_parser_free(parser) }

    for name in ["table", "strikethrough", "tasklist"] {
      if let ext = cmark_find_syntax_extension(name) {
        cmark_parser_attach_syntax_extension(parser, ext)
      }
    }

    let bytes = Array(source.utf8)
    bytes.withUnsafeBufferPointer { buffer in
      buffer.baseAddress.map {
        $0.withMemoryRebound(to: CChar.self, capacity: buffer.count) {
          cmark_parser_feed(parser, $0, buffer.count)
        }
      }
    }

    guard let doc = cmark_parser_finish(parser) else { return "" }
    defer { cmark_node_free(doc) }

    let extensions = cmark_parser_get_syntax_extensions(parser)
    guard let html = cmark_render_html(doc, CMARK_OPT_DEFAULT, extensions) else { return "" }
    defer { free(html) }
    return String(cString: html)
  }

  /// Sanitise a CSS font-family value by stripping dangerous chars.
  static func sanitizeFontFamily(_ value: String) -> String {
    let ok = value.allSatisfy { char in
      (char.isLetter || char.isNumber) || " ,'\"-_".contains(char)
    }
    return ok ? value : "system-ui, sans-serif"
  }

  /// HTML-escape a string for safe interpolation in attributes or text.
  static func htmlEscape(_ s: String) -> String {
    s.replacingOccurrences(of: "&", with: "&amp;")
      .replacingOccurrences(of: "<", with: "&lt;")
      .replacingOccurrences(of: ">", with: "&gt;")
      .replacingOccurrences(of: "\"", with: "&quot;")
      .replacingOccurrences(of: "'", with: "&#x27;")
  }

  /// Adds a Run button to shell code blocks. `$ ` prompts are stripped
  /// (only prompted lines run when a block has any). Those blocks wrap, so
  /// no part of the command hides past the right edge.
  static func runButtonScript(nonce: String) -> String {
    """
    <style nonce="\(nonce)">
    pre { position: relative; }
    pre.impulse-runnable { padding-right: 72px; }
    pre.impulse-runnable code { white-space: pre-wrap; overflow-wrap: anywhere; }
    .impulse-run { position: absolute; top: 6px; right: 6px; font: 12px -apple-system, sans-serif;
      padding: 2px 8px; border-radius: 5px; border: 1px solid rgba(127,127,127,.4);
      background: rgba(127,127,127,.15); color: inherit; cursor: pointer; opacity: .75; }
    .impulse-run:hover { opacity: 1; }
    </style>
    <script nonce="\(nonce)">
    (function () {
      var shells = /(^|\\s)language-(bash|sh|shell|zsh|fish|console|shell-session|terminal)(\\s|$)/;
      function command(code) {
        var lines = code.innerText.replace(/\\n$/, "").split("\\n");
        var prompted = lines.filter(function (l) { return /^\\s*\\$ /.test(l); });
        return (prompted.length ? prompted.map(function (l) { return l.replace(/^\\s*\\$ /, ""); }) : lines).join("\\n");
      }
      document.querySelectorAll("pre > code").forEach(function (code) {
        if (!shells.test(code.className) || !window.webkit || !window.webkit.messageHandlers.impulseRun) return;
        var button = document.createElement("button");
        button.className = "impulse-run";
        button.textContent = "\u{25B6} Run";
        button.title = "Run in a terminal";
        button.addEventListener("click", function () {
          window.webkit.messageHandlers.impulseRun.postMessage(command(code));
        });
        code.parentElement.classList.add("impulse-runnable");
        code.parentElement.appendChild(button);
      });
    })();
    </script>
    """
  }

  /// A new script nonce for each rendered page, so nothing written into the
  /// page ahead of time can carry it.
  static func makeNonce() -> String {
    var generator = SystemRandomNumberGenerator()
    let bytes = (0..<16).map { _ in UInt8.random(in: .min ... .max, using: &generator) }
    return Data(bytes).base64EncodedString()
  }

  /// A Run button's command, ready to type into a shell.
  public struct RunCommand: Equatable, Sendable {
    /// Control characters dropped (escape sequences, ⌃U and the like could
    /// rewrite or run the line), tabs as spaces, line ends as `\n`, the
    /// ends trimmed.
    public let text: String
    /// Nothing in it can hide from someone reading the preview: one line,
    /// no long run of spaces pushing the rest aside, and no invisible
    /// format characters (bidirectional overrides, zero-width characters)
    /// or Unicode line breaks. A command that isn't plain should be
    /// checked before it runs.
    public let isPlain: Bool
  }

  /// Runs of spaces at least this long make a command not plain.
  static let hidingSpaceRun = 16

  /// The command a Run button sent (`raw`), or nil when there's nothing to
  /// run.
  public static func runCommand(from raw: String) -> RunCommand? {
    var scalars = String.UnicodeScalarView()
    var previous: Unicode.Scalar?
    for scalar in raw.unicodeScalars {
      defer { previous = scalar }
      switch scalar.value {
      case 0x0A where previous == "\r": continue
      case 0x0A, 0x0D: scalars.append("\n")
      case 0x09: scalars.append(" ")
      case 0x00...0x1F, 0x7F...0x9F: continue
      default: scalars.append(scalar)
      }
    }
    let text = String(scalars).trimmingCharacters(in: .whitespacesAndNewlines)
    guard !text.isEmpty else { return nil }
    let invisible = text.unicodeScalars.contains { scalar in
      switch scalar.properties.generalCategory {
      case .format, .lineSeparator, .paragraphSeparator: return true
      default: return false
      }
    }
    let isPlain =
      !text.contains("\n") && !invisible && !text.contains(String(repeating: " ", count: hidingSpaceRun))
    return RunCommand(text: text, isPlain: isPlain)
  }

  /// Render a markdown source string to a full standalone HTML document with
  /// themed CSS and highlight.js code highlighting. `highlightJSPath` should
  /// be an absolute `file://` URL string to `highlight.min.js` (empty to
  /// disable highlighting). Returns nil if the source exceeds the size limit.
  /// Scripts run only with the page's nonce, new for each render: the
  /// highlight.js file and Impulse's own scripts carry it.
  ///
  /// With `runButtons`, shell code blocks (bash, sh, zsh, fish, console…)
  /// get a Run button that posts the command to the `impulseRun` message
  /// handler.
  public static func render(
    source: String, theme: MarkdownThemeColors, highlightJSPath: String, runButtons: Bool = false
  ) -> String? {
    guard source.utf8.count <= maxMarkdownSize else {
      NSLog("Markdown source exceeds %d byte limit, skipping preview", maxMarkdownSize)
      return nil
    }

    let body = renderHTMLBody(source)

    let bg = sanitizeCSSColor(theme.bg, fallback: "#1a1b26")
    let fg = sanitizeCSSColor(theme.fg, fallback: "#c0caf5")
    let heading = sanitizeCSSColor(theme.heading, fallback: "#7dcfff")
    let link = sanitizeCSSColor(theme.link, fallback: "#7aa2f7")
    let codeBg = sanitizeCSSColor(theme.codeBg, fallback: "#16161e")
    let border = sanitizeCSSColor(theme.border, fallback: "#292e42")
    let blockquoteFg = sanitizeCSSColor(theme.blockquoteFg, fallback: "#565f89")
    let hljsKeyword = sanitizeCSSColor(theme.hljsKeyword, fallback: "#bb9af7")
    let hljsString = sanitizeCSSColor(theme.hljsString, fallback: "#9ece6a")
    let hljsNumber = sanitizeCSSColor(theme.hljsNumber, fallback: "#ff9e64")
    let hljsComment = sanitizeCSSColor(theme.hljsComment, fallback: "#565f89")
    let hljsFunction = sanitizeCSSColor(theme.hljsFunction, fallback: "#7aa2f7")
    let hljsType = sanitizeCSSColor(theme.hljsType, fallback: "#e0af68")
    let fontFamily = sanitizeFontFamily(theme.fontFamily)
    let codeFontFamily = sanitizeFontFamily(theme.codeFontFamily)

    let nonce = makeNonce()
    let hljsPath = htmlEscape(highlightJSPath.trimmingCharacters(in: .whitespacesAndNewlines))
    let highlightScripts =
      hljsPath.isEmpty
      ? ""
      : """
      <script nonce="\(nonce)" src="\(hljsPath)"></script>
      <script nonce="\(nonce)">if (window.hljs) { window.hljs.highlightAll(); }</script>
      """

    return """
      <!DOCTYPE html>
      <html>
      <head>
      <meta charset="utf-8">
      <meta name="viewport" content="width=device-width, initial-scale=1">
      <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'nonce-\(nonce)'; img-src file: data:; font-src file:;">
      <style>
      * { margin: 0; padding: 0; box-sizing: border-box; }
      html {
          background: \(bg);
          inline-size: 100%;
          min-height: 100%;
      }
      body {
          background: \(bg);
          color: \(fg);
          font-family: \(fontFamily);
          font-size: 15px;
          line-height: 1.6;
          inline-size: 100%;
          min-inline-size: 0;
          max-inline-size: none;
          padding: 24px clamp(16px, 3vw, 40px);
          width: 100%;
          max-width: none;
          min-height: 100vh;
          margin: 0;
          overflow-wrap: break-word;
      }
      h1, h2, h3, h4, h5, h6 {
          color: \(heading);
          margin-top: 1.2em;
          margin-bottom: 0.4em;
          line-height: 1.3;
      }
      h1 { font-size: 2em; border-bottom: 1px solid \(border); padding-bottom: 0.3em; }
      h2 { font-size: 1.5em; border-bottom: 1px solid \(border); padding-bottom: 0.3em; }
      h3 { font-size: 1.25em; }
      p { margin: 0.6em 0; }
      a { color: \(link); text-decoration: none; }
      a:hover { text-decoration: underline; }
      code {
          font-family: \(codeFontFamily);
          background: \(codeBg);
          padding: 0.15em 0.4em;
          border-radius: 4px;
          font-size: 0.9em;
          overflow-wrap: anywhere;
          word-break: break-word;
      }
      pre {
          background: \(codeBg);
          padding: 14px 16px;
          border-radius: 6px;
          overflow-x: auto;
          margin: 0.8em 0;
          border: 1px solid \(border);
      }
      pre code {
          background: none;
          padding: 0;
          font-size: 0.88em;
          line-height: 1.5;
          overflow-wrap: normal;
          white-space: pre;
          word-break: normal;
      }
      blockquote {
          border-left: 3px solid \(border);
          padding-left: 16px;
          color: \(blockquoteFg);
          margin: 0.8em 0;
      }
      table {
          border-collapse: collapse;
          inline-size: 100%;
          max-inline-size: 100%;
          table-layout: fixed;
          width: 100%;
          margin: 0.8em 0;
      }
      th, td {
          border: 1px solid \(border);
          padding: 8px 12px;
          text-align: left;
          vertical-align: top;
          min-inline-size: 0;
          overflow-wrap: anywhere;
          word-break: break-word;
      }
      th code, td code {
          padding: 0.1em 0.3em;
          white-space: normal;
      }
      th {
          background: \(codeBg);
          font-weight: 600;
      }
      ul, ol { padding-left: 2em; margin: 0.6em 0; }
      li { margin: 0.2em 0; }
      li input[type="checkbox"] { margin-right: 0.5em; }
      del { opacity: 0.6; }
      hr {
          border: none;
          border-top: 1px solid \(border);
          margin: 1.5em 0;
      }
      img { max-width: 100%; height: auto; border-radius: 4px; }

      /* highlight.js theme overrides */
      .hljs { background: \(codeBg) !important; color: \(fg); }
      .hljs-keyword, .hljs-selector-tag, .hljs-built_in { color: \(hljsKeyword); }
      .hljs-string, .hljs-attr { color: \(hljsString); }
      .hljs-number, .hljs-literal { color: \(hljsNumber); }
      .hljs-comment, .hljs-doctag { color: \(hljsComment); font-style: italic; }
      .hljs-function, .hljs-title { color: \(hljsFunction); }
      .hljs-type, .hljs-class, .hljs-title.class_ { color: \(hljsType); }
      .hljs-variable { color: \(fg); }
      .hljs-meta { color: \(hljsKeyword); }
      .hljs-params { color: \(fg); }
      </style>
      </head>
      <body>
      \(body)
      \(highlightScripts)
      \(runButtons ? runButtonScript(nonce: nonce) : "")
      </body>
      </html>
      """
  }
}
