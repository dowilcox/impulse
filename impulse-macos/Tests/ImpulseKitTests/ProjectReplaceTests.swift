#if canImport(Testing)
  import Testing

  @testable import ImpulseKit

  struct ProjectReplaceTests {
    @Test func replacesEveryOccurrenceKeepingLineEndings() {
      let text = "let Foo = foo()\r\nfoo.bar\n"
      let insensitive = ProjectReplace.replace(in: text, query: "foo", with: "baz", caseSensitive: false)
      #expect(insensitive.text == "let baz = baz()\r\nbaz.bar\n")
      #expect(insensitive.count == 3)
      let sensitive = ProjectReplace.replace(in: text, query: "foo", with: "baz", caseSensitive: true)
      #expect(sensitive.text == "let Foo = baz()\r\nbaz.bar\n")
      #expect(sensitive.count == 2)
    }

    @Test func emptyAndMultilineQueriesReplaceNothing() {
      #expect(ProjectReplace.replace(in: "abc", query: "", with: "x", caseSensitive: false).count == 0)
      #expect(ProjectReplace.replace(in: "a\nb", query: "a\nb", with: "x", caseSensitive: false).text == "a\nb")
    }

    @Test func replacementCanContainTheQuery() {
      let result = ProjectReplace.replace(in: "aa", query: "a", with: "aa", caseSensitive: true)
      #expect(result.text == "aaaa")
      #expect(result.count == 2)
    }

    @Test func previewMarksRemovedAndAdded() {
      #expect(
        ProjectReplace.preview(line: "x = Foo(foo)", query: "foo", replacement: "bar", caseSensitive: false) == [
          .same("x = "), .removed("Foo"), .added("bar"), .same("("), .removed("foo"), .added("bar"), .same(")"),
        ])
      #expect(
        ProjectReplace.preview(line: "remove me", query: " me", replacement: "", caseSensitive: true)
          == [.same("remove"), .removed(" me")])
    }
  }
#endif
