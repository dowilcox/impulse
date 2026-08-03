// Parity tests for the theme system port, asserted against golden fixtures
// generated from the Rust implementation (theme.rs / protocol.rs / markdown.rs).
#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  private let themeIDs = [
    "kanagawa", "rose-pine", "nord", "gruvbox", "tokyo-night", "tokyo-night-storm",
    "catppuccin-mocha", "dracula", "solarized-dark", "one-dark", "ayu-dark",
    "everforest-dark", "github-dark", "monokai-pro", "palenight", "solarized-light",
    "catppuccin-latte", "github-light", "harbor",
  ]

  /// Returns the path and both values of the first structural difference
  /// between two parsed JSON values, or nil if they are equal.
  private func firstJSONDifference(_ lhs: Any, _ rhs: Any, path: String = "$") -> String? {
    switch (lhs, rhs) {
    case let (l, r) as (NSDictionary, NSDictionary):
      let allKeys = Set(l.allKeys.compactMap { $0 as? String })
        .union(r.allKeys.compactMap { $0 as? String })
      for key in allKeys.sorted() {
        let lv = l[key]
        let rv = r[key]
        switch (lv, rv) {
        case (nil, nil):
          continue
        case (nil, .some(let rvv)):
          return "\(path).\(key): missing in Swift output, fixture has \(rvv)"
        case (.some(let lvv), nil):
          return "\(path).\(key): Swift output has \(lvv), missing in fixture"
        case (.some(let lvv), .some(let rvv)):
          if let diff = firstJSONDifference(lvv, rvv, path: "\(path).\(key)") {
            return diff
          }
        }
      }
      return nil
    case let (l, r) as (NSArray, NSArray):
      if l.count != r.count {
        return "\(path): array count \(l.count) != fixture count \(r.count)"
      }
      for i in 0..<l.count {
        if let diff = firstJSONDifference(l[i], r[i], path: "\(path)[\(i)]") {
          return diff
        }
      }
      return nil
    default:
      if (lhs as? NSObject) != (rhs as? NSObject) {
        return "\(path): Swift output \(lhs) != fixture \(rhs)"
      }
      return nil
    }
  }

  /// Encodes an Encodable value with JSONEncoder and re-parses it with
  /// JSONSerialization for order-insensitive structural comparison.
  private func reparsed<T: Encodable>(_ value: T) throws -> Any {
    let data = try JSONEncoder().encode(value)
    return try JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed])
  }

  private func expectMatchesFixture<T: Encodable>(
    _ value: T, fixture relativePath: String
  ) throws {
    let got = try reparsed(value)
    let want = try Fixtures.json(relativePath)
    let diff = firstJSONDifference(got, want)
    #expect(
      diff == nil,
      "\(relativePath): first difference at \(diff ?? "")")
  }

  struct ThemeParityTests {
    @Test func builtinThemeNamesMatchFixtureSet() throws {
      let displayNames = try #require(
        try Fixtures.json("themes/display_names.json") as? [String: String])
      let names = ThemeStore.builtinThemeNames()
      #expect(names.count == 19)
      #expect(Set(names) == Set(displayNames.keys))
      // Order mirrors the Rust BUILTIN_THEMES registry.
      #expect(names == themeIDs)
    }

    @Test func displayNamesMatchFixture() throws {
      let expected = try #require(
        try Fixtures.json("themes/display_names.json") as? [String: String])
      for (id, want) in expected {
        #expect(ThemeStore.themeDisplayName(id) == want, "display name for \(id)")
      }
    }

    @Test(arguments: themeIDs)
    func resolvedThemeMatchesFixture(id: String) throws {
      let theme = ThemeStore.getTheme(id)
      try expectMatchesFixture(theme, fixture: "themes/\(id).theme.json")
    }

    @Test(arguments: themeIDs)
    func monacoThemeMatchesFixture(id: String) throws {
      let monaco = themeToMonaco(ThemeStore.getTheme(id))
      try expectMatchesFixture(monaco, fixture: "themes/\(id).monaco.json")
    }

    @Test(arguments: themeIDs)
    func markdownColorsMatchFixture(id: String) throws {
      let colors = themeToMarkdownColors(ThemeStore.getTheme(id))
      try expectMatchesFixture(colors, fixture: "themes/\(id).markdown.json")
    }

    // Fallback behavior mirrors Rust's get_theme: unknown names resolve to Nord.
    @Test func unknownThemeFallsBackToNord() {
      let theme = ThemeStore.getTheme("nonexistent-theme-xyz")
      #expect(theme.name == "Nord")
      #expect(theme.id == "nord")
    }

    // Alternative-format IDs normalize to the canonical kebab-case theme.
    @Test func normalizedIDLookup() {
      #expect(ThemeStore.getTheme("tokyo_night").id == "tokyo-night")
      #expect(ThemeStore.getTheme("tokyonight").id == "tokyo-night")
      #expect(ThemeStore.getTheme("catppuccin_mocha").id == "catppuccin-mocha")
      #expect(ThemeStore.getTheme("Nord").id == "nord")
    }

    // Older serialized themes lack surface_style — decoding must default to flat.
    @Test func resolvedThemeDecodeDefaultsSurfaceStyle() throws {
      let data = try Fixtures.data("themes/kanagawa.theme.json")
      var obj = try #require(
        try JSONSerialization.jsonObject(with: data) as? [String: Any])
      obj.removeValue(forKey: "surface_style")
      let stripped = try JSONSerialization.data(withJSONObject: obj)
      let theme = try JSONDecoder().decode(ResolvedTheme.self, from: stripped)
      #expect(theme.surfaceStyle == "flat")
    }
  }
#endif
