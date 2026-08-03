// Parity tests for the shell parser port, asserted against the golden
// fixture generated from the Rust implementation, plus the Rust unit tests
// from impulse-core/src/shell_parser.rs ported to Swift.
#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct ShellParserFixtureTests {
    /// Recursively drop NSNull entries from decoded JSON objects so serde's
    /// explicit `null` and JSONEncoder's omitted optional keys compare equal.
    static func stripNulls(_ value: Any) -> Any {
      if let dict = value as? [String: Any] {
        var out: [String: Any] = [:]
        for (key, item) in dict where !(item is NSNull) {
          out[key] = stripNulls(item)
        }
        return out
      }
      if let array = value as? [Any] {
        return array.map { stripNulls($0) }
      }
      return value
    }

    @Test func matchesRustGoldenFixture() throws {
      let elements = try #require(Fixtures.json("shell_parser.json") as? [[String: Any]])
      #expect(elements.count >= 15)
      for element in elements {
        let input = try #require(element["input"] as? String)
        let cursor = try #require(element["cursor"] as? Int)

        let result = parseShellInput(input, cursor: cursor)
        let encoded = try JSONEncoder().encode(result)
        let decoded = try #require(
          JSONSerialization.jsonObject(with: encoded) as? [String: Any])

        let expected = Self.stripNulls(element) as! NSDictionary
        let actual = Self.stripNulls(decoded) as! NSDictionary
        #expect(actual == expected, "input=\(input) cursor=\(cursor)")
      }
    }
  }

  // Ported from the Rust unit tests in impulse-core/src/shell_parser.rs.
  struct ShellParserUnitTests {
    private func parse(_ input: String) -> ShellParseResult {
      parseShellInput(input, cursor: input.utf8.count)
    }

    @Test func parsesEnvAssignmentAndCommandContext() {
      let parsed = parse("FOO=bar cargo test -p impulse-core")

      #expect(parsed.assignments.count == 1)
      #expect(parsed.assignments[0].text == "FOO=bar")
      #expect(parsed.completion.command == "cargo")
      #expect(parsed.completion.kind == .argument)
      #expect(parsed.completion.prefix == "impulse-core")
    }

    @Test func preservesQuotedArgumentAndReportsUnfinishedQuote() {
      let parsed = parse("cd \"src/my folder")

      #expect(parsed.incomplete)
      #expect(parsed.completion.command == "cd")
      #expect(parsed.completion.kind == .argument)
      #expect(parsed.completion.prefix == "src/my folder")
      #expect(parsed.completion.quoteState == .double)
    }

    @Test func handlesEscapedSpaces() {
      let parsed = parse("echo hello\\ world")

      #expect(!parsed.incomplete)
      #expect(parsed.completion.command == "echo")
      #expect(parsed.completion.prefix == "hello world")
      #expect(parsed.tokens[1].text == "hello world")
    }

    @Test func resetsCommandContextAfterPipe() {
      let parsed = parse("cat Cargo.toml | rg serde")

      #expect(parsed.pipelineIndex == 1)
      #expect(parsed.completion.command == "rg")
      #expect(parsed.completion.kind == .argument)
      #expect(parsed.completion.prefix == "serde")
    }

    @Test func resetsPipelineIndexAfterControlOperator() {
      let parsed = parse("cat Cargo.toml | rg serde && echo done")

      #expect(parsed.pipelineIndex == 0)
      #expect(parsed.completion.command == "echo")
      #expect(parsed.completion.prefix == "done")
    }

    @Test func parsesRedirectOperatorAndTarget() {
      let parsed = parse("cargo test > target/out.log")

      #expect(parsed.redirects.count == 1)
      #expect(parsed.redirects[0].operatorText == ">")
      #expect(parsed.redirects[0].target == "target/out.log")
      #expect(parsed.completion.kind == .redirectTarget)
      #expect(parsed.completion.prefix == "target/out.log")
    }

    @Test func parsesFdRedirectWithoutSpaces() {
      let parsed = parse("cargo test 2>/tmp/impulse.log")

      #expect(parsed.redirects.count == 1)
      #expect(parsed.redirects[0].operatorText == "2>")
      #expect(parsed.redirects[0].target == "/tmp/impulse.log")
      #expect(parsed.completion.kind == .redirectTarget)
    }

    @Test func parsesHereStringAsOneRedirectOperator() {
      let parsed = parse("cat <<< hello")

      #expect(parsed.redirects.count == 1)
      #expect(parsed.redirects[0].operatorText == "<<<")
      #expect(parsed.redirects[0].operatorSpan == TextSpan(start: 4, end: 7))
      #expect(parsed.redirects[0].target == "hello")
      #expect(parsed.completion.kind == .redirectTarget)
      #expect(parsed.completion.prefix == "hello")
    }

    @Test func expectsRedirectTargetAfterOperator() {
      let parsed = parse("cargo test 2>")

      #expect(parsed.redirects.count == 1)
      #expect(parsed.redirects[0].target == nil)
      #expect(parsed.completion.kind == .redirectTarget)
      #expect(parsed.completion.span == TextSpan(start: 13, end: 13))
    }

    @Test func reportsTrailingEscapeAsIncomplete() {
      let parsed = parse("echo foo\\")

      #expect(parsed.incomplete)
      #expect(parsed.completion.prefix == "foo")
      #expect(parsed.completion.kind == .argument)
    }
  }
#endif
