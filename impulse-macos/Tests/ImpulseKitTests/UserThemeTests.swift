// User theme files: case-insensitive ids, the menu name from the file, and
// a readable reason when a file can't be used.
#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  private let campfire = """
    name = "Campfire Glow"
    variant = "dark"

    [palette]
    bg = "#1f1d1a"
    fg = "#e8dfd0"
    accent = "#e0a458"
    red = "#e06c5b"
    orange = "#e0915b"
    yellow = "#e0c25b"
    green = "#9cc27a"
    cyan = "#6cc2b8"
    blue = "#7aa6d6"
    magenta = "#c58fc4"
    """

  /// A fresh themes folder holding `files` (name → contents).
  private func themesFolder(_ files: [String: String]) throws -> URL {
    let dir = FileManager.default.temporaryDirectory
      .appendingPathComponent("impulse-themes-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    for (name, contents) in files {
      try contents.write(to: dir.appendingPathComponent(name), atomically: true, encoding: .utf8)
    }
    return dir
  }

  struct UserThemeTests {
    @Test func fileNameCaseDoesNotMatter() throws {
      let dir = try themesFolder(["Campfire.toml": campfire])
      defer { try? FileManager.default.removeItem(at: dir) }

      #expect(ThemeStore.availableThemes(userThemesIn: dir).last == "campfire")
      for name in ["campfire", "Campfire", "CAMPFIRE"] {
        let theme = ThemeStore.getTheme(name, userThemesIn: dir)
        #expect(theme.id == "campfire")
        #expect(theme.name == "Campfire Glow")
      }
      #expect(ThemeStore.loadProblem(for: "Campfire", userThemesIn: dir) == nil)
    }

    @Test func menuShowsTheNameInTheFile() throws {
      let dir = try themesFolder(["campfire.toml": campfire, "broken.toml": "name = 3"])
      defer { try? FileManager.default.removeItem(at: dir) }

      #expect(ThemeStore.themeMenuName("campfire", userThemesIn: dir) == "Campfire Glow")
      #expect(ThemeStore.themeMenuName("rose-pine", userThemesIn: dir) == "Rosé Pine")
      // A file that doesn't load falls back to a name made from its id.
      #expect(ThemeStore.themeMenuName("broken", userThemesIn: dir) == "Broken")
    }

    @Test func userFileReplacesBuiltinWithoutListingItTwice() throws {
      let dir = try themesFolder(["Nord.toml": campfire])
      defer { try? FileManager.default.removeItem(at: dir) }

      let names = ThemeStore.availableThemes(userThemesIn: dir)
      #expect(names == ThemeStore.builtinThemeNames())
      #expect(ThemeStore.getTheme("nord", userThemesIn: dir).name == "Campfire Glow")
      #expect(ThemeStore.themeMenuName("nord", userThemesIn: dir) == "Campfire Glow")
    }

    @Test func missingKeyIsNamed() throws {
      let missingAccent = campfire.replacingOccurrences(of: "accent = \"#e0a458\"\n", with: "")
      let dir = try themesFolder(["ember.toml": missingAccent])
      defer { try? FileManager.default.removeItem(at: dir) }

      let problem = try #require(ThemeStore.loadProblem(for: "ember", userThemesIn: dir))
      #expect(problem.path.lastPathComponent == "ember.toml")
      #expect(problem.message == "palette.accent is missing or isn't a string")
      // The theme itself falls back to Nord.
      #expect(ThemeStore.getTheme("ember", userThemesIn: dir).id == "nord")
    }

    @Test func missingTableIsNamed() throws {
      let dir = try themesFolder(["ember.toml": "name = \"Ember\"\nvariant = \"dark\"\n"])
      defer { try? FileManager.default.removeItem(at: dir) }

      let problem = try #require(ThemeStore.loadProblem(for: "ember", userThemesIn: dir))
      #expect(problem.message == "palette is missing")
    }

    @Test func syntaxErrorGivesItsPosition() throws {
      let dir = try themesFolder(["ember.toml": "name = \"Ember\"\nvariant = \n"])
      defer { try? FileManager.default.removeItem(at: dir) }

      let problem = try #require(ThemeStore.loadProblem(for: "ember", userThemesIn: dir))
      #expect(problem.message.contains("line 2"))
    }

    // TOMLKit can't tell a wrong type from a missing value; the message
    // covers both.
    @Test func wrongTypeIsNamed() throws {
      let dir = try themesFolder(["ember.toml": campfire.replacingOccurrences(of: "bg = \"#1f1d1a\"", with: "bg = 5")])
      defer { try? FileManager.default.removeItem(at: dir) }

      let problem = try #require(ThemeStore.loadProblem(for: "ember", userThemesIn: dir))
      #expect(problem.message == "palette.bg is missing or isn't a string")
    }

    @Test func noProblemWithoutAFile() throws {
      let dir = try themesFolder([:])
      defer { try? FileManager.default.removeItem(at: dir) }

      #expect(ThemeStore.loadProblem(for: "nord", userThemesIn: dir) == nil)
      #expect(ThemeStore.loadProblem(for: "nonexistent", userThemesIn: dir) == nil)
    }

    @Test func canonicalIDFollowsAliasesAndCase() {
      #expect(ThemeStore.canonicalID("Tokyo_Night") == "tokyo-night")
      #expect(ThemeStore.canonicalID("Campfire") == "campfire")
    }
  }
#endif
