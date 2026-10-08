#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct GitServerMessageTests {
    @Test func aLinkOnTheNextLine() throws {
      let output = [
        "Enumerating objects: 5, done.",
        "remote: Counting objects: 100% (5/5), done.",
        "remote: ",
        "remote: To create a merge request for fix-elevation, visit:",
        "remote:   https://git.example.edu/web/trailhead/-/merge_requests/new?merge_request%5Bsource_branch%5D=fix-elevation",
        "remote: ",
        "To git.example.edu:web/trailhead.git",
        " * [new branch]      fix-elevation -> fix-elevation",
      ]
      let message = try #require(GitServerMessage.parse(output))
      #expect(message.lines.count == 2)
      #expect(message.linkCaption == "To create a merge request for fix-elevation, visit:")
      #expect(
        message.link?.absoluteString
          == "https://git.example.edu/web/trailhead/-/merge_requests/new?merge_request%5Bsource_branch%5D=fix-elevation")
    }

    @Test func aLinkAfterTextOnTheSameLine() throws {
      let message = try #require(GitServerMessage.parse(["remote: View it at https://example.com/r/12 now"]))
      #expect(message.linkCaption == "View it at")
      #expect(message.link?.absoluteString == "https://example.com/r/12")
    }

    @Test func textWithoutALink() throws {
      let message = try #require(GitServerMessage.parse(["remote: warning: large files detected"]))
      #expect(message.lines == ["warning: large files detected"])
      #expect(message.link == nil)
      #expect(message.linkCaption == nil)
    }

    @Test func onlyProgressIsNothing() {
      #expect(GitServerMessage.parse([]) == nil)
      #expect(
        GitServerMessage.parse([
          "remote: Resolving deltas: 100% (2/2), completed with 2 local objects.", "remote: Total 3 (delta 1)",
          "To example.com:r.git", "   abc..def  main -> main",
        ]) == nil)
    }
  }
#endif
