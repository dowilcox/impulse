#if canImport(Testing)
  import Foundation
  @testable import ImpulseApp
  import Testing

  struct SettingsCatalogTests {
    /// Every key Impulse writes to settings.json.
    private func encodedKeys() throws -> Set<String> {
      let data = try JSONEncoder().encode(Settings.default)
      let object = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
      return Set(object.keys)
    }

    @Test func everySettingIsInTheSchema() throws {
      let schema = SettingsCatalog.jsonSchema()
      let properties = try #require(schema["properties"] as? [String: Any])
      let missing = try encodedKeys().subtracting(properties.keys)
      #expect(missing.isEmpty, "add these to SettingsCatalog: \(missing.sorted())")
      #expect(JSONSerialization.isValidJSONObject(schema))
    }

    @Test func catalogKeysAreRealAndUnique() throws {
      let keys = SettingsCatalog.items.map(\.key)
      #expect(Set(keys).count == keys.count, "duplicate catalog keys")
      let unknown = Set(keys).subtracting(try encodedKeys())
      #expect(unknown.isEmpty, "catalog keys settings.json doesn't have: \(unknown.sorted())")
    }

    @Test func modifiedAndResetFollowTheDefault() throws {
      let item = try #require(SettingsCatalog.items.first { $0.key == "font_size" })
      var settings = Settings.default
      #expect(!item.isModified(settings))
      settings.fontSize = 20
      #expect(item.isModified(settings))
      item.reset(&settings)
      #expect(settings.fontSize == Settings.default.fontSize)
      #expect(item.matches("FONT"))
      #expect(item.matches("font_size"))
      #expect(!item.matches("scrollback"))
    }
  }
#endif
