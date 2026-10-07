#if canImport(Testing)
  import Foundation
  @testable import ImpulseApp
  import Testing

  struct FileTypeIndentationTests {
    @Test func overridesSetTabWidthAndIndentationPerFile() {
      var settings = Settings.default
      settings.tabWidth = 4
      settings.useSpaces = true
      settings.fileTypeOverrides = [
        FileTypeOverride(pattern: "*.go", useSpaces: false),
        FileTypeOverride(pattern: "*.go", tabWidth: 8),
        FileTypeOverride(pattern: "Makefile", tabWidth: 8, useSpaces: false),
        FileTypeOverride(pattern: "*.md", tabWidth: 2),
      ]
      // Each value from the first matching row that sets it.
      #expect(settings.indentation(forPath: "/p/main.go") == (8, false))
      #expect(settings.indentation(forPath: "/p/Makefile") == (8, false))
      #expect(settings.indentation(forPath: "/p/README.MD") == (2, true))
      // No match: the editor's settings.
      #expect(settings.indentation(forPath: "/p/main.rs") == (4, true))
    }

    @Test func nonPositiveTabWidthsAreIgnored() {
      var settings = Settings.default
      settings.tabWidth = 4
      settings.fileTypeOverrides = [
        FileTypeOverride(pattern: "*.py", tabWidth: 0),
        FileTypeOverride(pattern: "*", tabWidth: 3),
      ]
      #expect(settings.indentation(forPath: "/p/a.py").tabWidth == 3)
    }
  }
#endif
