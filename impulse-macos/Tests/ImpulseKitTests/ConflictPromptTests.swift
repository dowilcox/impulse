#if canImport(Testing)
  import Testing

  @testable import ImpulseKit

  struct ConflictPromptTests {
    private let file = """
      import A
      <<<<<<< HEAD
      let x = 1
      =======
      let x = 2
      >>>>>>> topic
      print(x)
      <<<<<<< HEAD
      a()
      ||||||| base
      b()
      =======
      c()
      >>>>>>> topic
      """

    @Test func findsBlocksWithTheirLines() {
      let blocks = ConflictPrompt.blocks(in: file)
      #expect(blocks.map(\.startLine) == [2, 8])
      #expect(blocks.map(\.endLine) == [6, 14])
      #expect(blocks[0].text == "<<<<<<< HEAD\nlet x = 1\n=======\nlet x = 2\n>>>>>>> topic")
      #expect(ConflictPrompt.blocks(in: "no markers\n").isEmpty)
      #expect(ConflictPrompt.blocks(in: "<<<<<<< HEAD\nunterminated").isEmpty)
    }

    @Test func promptListsEveryConflict() {
      let prompt = ConflictPrompt.make(
        files: [("src/a.swift", file), ("clean.txt", "fine\n")], operation: "rebase")
      #expect(prompt.hasPrefix("Please resolve the merge conflicts from this rebase"))
      #expect(prompt.contains("## src/a.swift (2 conflicts)"))
      #expect(prompt.contains("Lines 2–6:\n```\n<<<<<<< HEAD\nlet x = 1"))
      #expect(prompt.contains("Lines 8–14:"))
      #expect(!prompt.contains("clean.txt"), "files without markers are skipped")
    }

    @Test func oversizedBlocksAreListedByLine() {
      let prompt = ConflictPrompt.make(files: [("big.txt", file)], limit: 330)
      #expect(prompt.contains("Lines 2–6:"))
      #expect(prompt.contains("not shown, open the files): big.txt:8-14"))
    }
  }
#endif
