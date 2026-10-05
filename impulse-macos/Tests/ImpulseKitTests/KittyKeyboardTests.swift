#if canImport(Testing)
  import Testing

  @testable import ImpulseKit

  struct KittyKeyboardTests {
    typealias K = KittyKeyboard

    private func encode(
      _ key: K.Key, _ modifiers: K.Modifiers = [], _ event: K.Event = .press, text: String? = nil,
      flags: K.Flags = .disambiguate
    ) -> String? {
      K.encode(key, modifiers: modifiers, event: event, text: text, flags: flags)
        .map { String(decoding: $0, as: UTF8.self).replacingOccurrences(of: "\u{1B}", with: "^[") }
    }

    private let a = K.Key.text(base: "a", shifted: "A")

    @Test func disambiguateKeepsTypingAsText() {
      #expect(encode(a) == nil)
      #expect(encode(a, .shift) == nil, "shift alone still types")
      #expect(encode(.enter) == nil)
      #expect(encode(.tab) == nil)
      #expect(encode(.backspace) == nil)
      #expect(encode(.up) == nil)
      #expect(encode(.function(5)) == nil)
    }

    @Test func disambiguatedKeys() {
      #expect(encode(.escape) == "^[[27u")
      #expect(encode(a, .control) == "^[[97;5u")
      #expect(encode(a, [.control, .shift]) == "^[[97;6u")
      #expect(encode(K.Key.text(base: "i", shifted: "I"), .control) == "^[[105;5u", "not the same as Tab")
      #expect(encode(a, .alt) == "^[[97;3u")
      #expect(encode(.enter, .control) == "^[[13;5u")
      #expect(encode(.tab, .shift) == "^[[9;2u")
      #expect(encode(.backspace, .alt) == "^[[127;3u")
    }

    @Test func functionalKeysWithModifiers() {
      #expect(encode(.up, .control) == "^[[1;5A")
      #expect(encode(.end, .shift) == "^[[1;2F")
      #expect(encode(.delete, .alt) == "^[[3;3~")
      #expect(encode(.pageDown, .control) == "^[[6;5~")
      #expect(encode(.function(1), .shift) == "^[[1;2P")
      #expect(encode(.function(3), .shift) == "^[[13;2~", "F3 avoids CSI R")
      #expect(encode(.function(12), .control) == "^[[24;5~")
    }

    @Test func eventTypes() {
      let flags: K.Flags = [.disambiguate, .reportEventTypes]
      #expect(encode(a, .control, .repeat, flags: flags) == "^[[97;5:2u")
      #expect(encode(a, .control, .release, flags: flags) == "^[[97;5:3u")
      #expect(encode(.up, [], .release, flags: flags) == "^[[1;1:3A")
      #expect(encode(a, [], .release, flags: flags) == "", "typing isn't reported on release")
      #expect(encode(.enter, [], .release, flags: flags) == "", "nor are Enter, Tab and Backspace")
      #expect(encode(a, .control, .release) == "", "no releases without the flag")
      #expect(encode(a, .control, .repeat) == "^[[97;5u", "repeats are presses without the flag")
    }

    @Test func allKeysAlternatesAndText() {
      let all: K.Flags = [.disambiguate, .reportAllKeys]
      #expect(encode(a, flags: all) == "^[[97u")
      #expect(encode(.enter, flags: all) == "^[[13u")
      #expect(encode(a, .shift, flags: [.disambiguate, .reportAllKeys, .reportAlternateKeys]) == "^[[97:65;2u")
      #expect(encode(a, .shift, flags: all) == "^[[97;2u", "alternates only when asked")
      #expect(
        encode(a, .shift, text: "A", flags: [.disambiguate, .reportAllKeys, .reportText]) == "^[[97;2;65u")
      #expect(encode(a, text: "a", flags: [.reportAllKeys, .reportText]) == "^[[97;1;97u")
    }
  }
#endif
