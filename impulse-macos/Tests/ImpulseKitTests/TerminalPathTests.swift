#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct TerminalPathTests {
    private func only(_ text: String) -> TerminalPathMatch? {
      let all = TerminalPathDetector.matches(in: text)
      #expect(all.count == 1, "\(text): \(all)")
      return all.first
    }

    @Test func compilerStyleLocations() {
      let rust = only("  --> src/main.rs:12:5")
      #expect(rust?.path == "src/main.rs")
      #expect(rust?.line == 12)
      #expect(rust?.column == 5)
      #expect(rust?.range == 6..<22)

      let swift = only("/Users/me/app/Thing.swift:40:9: error: nope")
      #expect(swift?.path == "/Users/me/app/Thing.swift")
      #expect(swift?.line == 40)
      #expect(swift?.column == 9)

      let grep = only("lib/util.go:7:    return nil")
      #expect(grep?.path == "lib/util.go")
      #expect(grep?.line == 7)
      #expect(grep?.column == nil)
    }

    @Test func parenthesizedPositionsAndStackTraces() {
      let ts = only("src/app.ts(3,14): error TS2322")
      #expect(ts?.path == "src/app.ts")
      #expect(ts?.line == 3)
      #expect(ts?.column == 14)

      let node = only("    at run (/srv/app/index.js:10:15)")
      #expect(node?.path == "/srv/app/index.js")
      #expect(node?.line == 10)
      #expect(node?.column == 15)
    }

    @Test func pythonTracebacks() {
      let python = only(#"  File "/opt/tool/main.py", line 88, in <module>"#)
      #expect(python?.path == "/opt/tool/main.py")
      #expect(python?.line == 88)
    }

    @Test func barePathsAndPunctuation() {
      #expect(only("see docs/README.md.")?.path == "docs/README.md")
      #expect(only("edit ~/notes/todo.txt now")?.path == "~/notes/todo.txt")
      #expect(only("cd ../other/dir")?.path == "../other/dir")
      #expect(only("modified:   Package.swift")?.path == "Package.swift")
    }

    @Test func ignoresUrlsVersionsAndNoise() {
      #expect(TerminalPathDetector.matches(in: "open https://example.com/a/b.html").isEmpty)
      #expect(TerminalPathDetector.matches(in: "v1.2.3 took 0.5s").isEmpty)
      #expect(TerminalPathDetector.matches(in: "... / -- ./").isEmpty)
      #expect(TerminalPathDetector.matches(in: "plain words only").isEmpty)
    }

    @Test func lookupByOffsetAndResolution() {
      let text = "a.rs:1 and b/c.rs:2:3"
      #expect(TerminalPathDetector.match(in: text, at: 0)?.path == "a.rs")
      #expect(TerminalPathDetector.match(in: text, at: 12)?.path == "b/c.rs")
      #expect(TerminalPathDetector.match(in: text, at: 8) == nil)

      #expect(TerminalPathDetector.resolve("src/x.rs", in: "/repo") == "/repo/src/x.rs")
      #expect(TerminalPathDetector.resolve("../y", in: "/repo/sub") == "/repo/y")
      #expect(TerminalPathDetector.resolve("/abs/z", in: "/repo") == "/abs/z")
      #expect(TerminalPathDetector.resolve("~/q", in: "/repo") == NSHomeDirectory() + "/q")
    }
  }
#endif
