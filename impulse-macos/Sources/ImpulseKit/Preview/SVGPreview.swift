import Foundation

// Ported from impulse-editor/src/svg.rs.

public enum SVGPreview {
  /// Maximum SVG source size (in bytes) before preview is refused.
  static let maxSVGSize = 1024 * 1024  // 1 MB

  /// Check whether a file path is an SVG file based on its extension.
  /// Mirrors Rust's `rsplit('.')` semantics exactly.
  public static func isSVGFile(_ path: String) -> Bool {
    let ext = path.split(separator: ".", omittingEmptySubsequences: false).last ?? ""
    return ext.lowercased() == "svg"
  }

  private static let scriptRegex = try! NSRegularExpression(
    pattern: #"<script[\s>].*?</script\s*>|<script\s*/>"#,
    options: [.caseInsensitive, .dotMatchesLineSeparators])
  private static let foreignObjectRegex = try! NSRegularExpression(
    pattern: #"<foreignObject[\s>].*?</foreignObject\s*>|<foreignObject\s*/>"#,
    options: [.caseInsensitive, .dotMatchesLineSeparators])
  private static let eventHandlerRegex = try! NSRegularExpression(
    pattern: #"\s+on\w+\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]*)"#,
    options: [.caseInsensitive])
  private static let javascriptURIRegex = try! NSRegularExpression(
    pattern: #"((?:xlink:)?href\s*=\s*(?:"|'))javascript:"#,
    options: [.caseInsensitive])

  /// Sanitize SVG source by removing potentially dangerous elements and
  /// attributes. The CSP is the primary defense; this provides defense-in-depth.
  static func sanitizeSVG(_ source: String) -> String {
    func strip(_ regex: NSRegularExpression, from s: String, template: String = "") -> String {
      regex.stringByReplacingMatches(
        in: s, range: NSRange(s.startIndex..., in: s), withTemplate: template)
    }
    var s = strip(scriptRegex, from: source)
    s = strip(foreignObjectRegex, from: s)
    s = strip(eventHandlerRegex, from: s)
    s = strip(javascriptURIRegex, from: s, template: "$1#blocked:")
    return s
  }

  /// Render an SVG source string to a full standalone HTML document with a
  /// themed background and centered layout. Returns nil if the source
  /// exceeds the size limit.
  public static func render(source: String, bgColor: String) -> String? {
    guard source.utf8.count <= maxSVGSize else {
      NSLog("SVG source exceeds %d byte limit, skipping preview", maxSVGSize)
      return nil
    }

    let bg = sanitizeCSSColor(bgColor, fallback: "#1a1b26")
    let sanitized = sanitizeSVG(source)

    return """
      <!DOCTYPE html>
      <html>
      <head>
      <meta charset="utf-8">
      <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src file: data:; connect-src 'none';">
      <style>
      * { margin: 0; padding: 0; box-sizing: border-box; }
      body {
          background: \(bg);
          display: flex;
          align-items: center;
          justify-content: center;
          min-height: 100vh;
          padding: 24px;
          overflow: auto;
      }
      .svg-container {
          max-width: 100%;
          max-height: 100%;
      }
      .svg-container svg {
          max-width: 100%;
          height: auto;
          display: block;
      }
      </style>
      </head>
      <body>
      <div class="svg-container">
      \(sanitized)
      </div>
      </body>
      </html>
      """
  }
}
