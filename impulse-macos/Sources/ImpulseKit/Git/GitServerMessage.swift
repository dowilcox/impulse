// What a git server said during a push: the `remote:` lines git prints.
// Hosts use them for links ("To create a merge request for x, visit:"
// followed by the address), and Impulse shows them as they are. It's git's
// own channel, so it works the same with any host, and Impulse never needs
// to know which host it is.

import Foundation

public struct GitServerMessage: Equatable, Sendable {
  /// The server's lines, without the `remote:` prefix, blank lines and
  /// pack progress ("Counting objects: 100%").
  public let lines: [String]
  /// The first web address in them.
  public let link: URL?
  /// What the server said about the link: the text before it on its line,
  /// or the line before.
  public let linkCaption: String?

  /// The message in a push's output lines; nil when the server said nothing
  /// worth showing.
  public static func parse(_ output: [String]) -> GitServerMessage? {
    var lines: [String] = []
    for raw in output {
      let line = raw.trimmingCharacters(in: .whitespacesAndNewlines)
      guard line.hasPrefix("remote:") else { continue }
      let text = line.dropFirst("remote:".count).trimmingCharacters(in: .whitespaces)
      guard !text.isEmpty, !isProgress(text) else { continue }
      lines.append(text)
    }
    guard !lines.isEmpty else { return nil }
    var link: URL?
    var caption: String?
    for (index, line) in lines.enumerated() {
      guard let range = line.range(of: #"https?://\S+"#, options: .regularExpression),
        let url = URL(string: String(line[range]))
      else { continue }
      link = url
      let before = line[..<range.lowerBound].trimmingCharacters(in: .whitespaces)
      caption = before.isEmpty ? (index > 0 ? lines[index - 1] : nil) : before
      break
    }
    return GitServerMessage(lines: lines, link: link, linkCaption: caption)
  }

  /// git's pack progress, relayed from the server.
  private static func isProgress(_ text: String) -> Bool {
    let progress = ["Enumerating objects", "Counting objects", "Compressing objects", "Total ", "Resolving deltas"]
    return progress.contains { text.hasPrefix($0) }
  }
}
