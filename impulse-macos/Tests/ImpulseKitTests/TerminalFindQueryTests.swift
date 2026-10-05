#if canImport(Testing)
  import Testing

  @testable import ImpulseKit

  struct TerminalFindQueryTests {
    @Test func literalTextIsEscapedAndCaseIsExplicit() {
      #expect(TerminalFindQuery(text: "").pattern == nil)
      #expect(TerminalFindQuery(text: "a.b(c)").pattern == "(?i)a\\.b\\(c\\)")
      #expect(TerminalFindQuery(text: "Error", caseSensitive: true).pattern == "(?-i)Error")
      #expect(TerminalFindQuery(text: "Error").pattern == "(?i)Error", "insensitive even with capitals")
    }

    @Test func regexAndWholeWord() {
      #expect(TerminalFindQuery(text: "err(or)?", regex: true).pattern == "(?i)err(or)?")
      #expect(TerminalFindQuery(text: "id", wholeWord: true).pattern == "(?i)(?-u:\\b)(?:id)(?-u:\\b)")
      #expect(
        TerminalFindQuery(text: "a|b", regex: true, wholeWord: true).pattern
          == "(?i)(?-u:\\b)(?:a|b)(?-u:\\b)", "alternation stays inside the boundaries")
    }

    @Test func escapeCoversEveryMetacharacter() {
      #expect(TerminalFindQuery.escape("\\.+*?()|[]{}^$#&-~") == "\\\\\\.\\+\\*\\?\\(\\)\\|\\[\\]\\{\\}\\^\\$\\#\\&\\-\\~")
      #expect(TerminalFindQuery.escape("plain words") == "plain words")
    }
  }
#endif
