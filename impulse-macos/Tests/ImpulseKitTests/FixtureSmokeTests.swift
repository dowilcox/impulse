// Phase 0 smoke tests: the golden fixtures are present in the test bundle
// and every one of them is valid JSON. The real parity assertions land with
// each ported module in later phases.
//
// Note: run tests with the full Xcode toolchain, e.g.
// `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer swift test`
// — under bare CommandLineTools this file compiles to nothing.
#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct FixtureSmokeTests {
    @Test func fixturesArePresentAndParse() throws {
      let paths = try Fixtures.allJSONPaths()
      #expect(paths.count >= 60, "expected the full fixture corpus, found \(paths.count)")

      // Every category of fixture must be represented.
      let required = [
        "shell_parser.json",
        "close_risk.json",
        "glob.json",
        "palette_items.json",
        "palette_filter.json",
        "file_tree_patch.json",
        "path_to_uri.json",
        "language_from_uri.json",
        "themes/nord.monaco.json",
        "themes/display_names.json",
      ]
      for name in required {
        #expect(paths.contains(name), "missing fixture \(name)")
      }

      for path in paths {
        _ = try Fixtures.json(path)
      }
    }

    @Test func allNineteenThemesCaptured() throws {
      let themeFiles = try Fixtures.allJSONPaths().filter {
        $0.hasPrefix("themes/") && $0.hasSuffix(".monaco.json")
      }
      #expect(themeFiles.count == 19, "expected 19 built-in themes, found \(themeFiles.count)")
    }
  }
#endif
