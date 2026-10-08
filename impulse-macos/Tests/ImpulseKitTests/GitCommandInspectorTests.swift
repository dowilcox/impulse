#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct GitCommandInspectorTests {
    @Test func mergesRebasesAndPulls() {
      #expect(GitCommandInspector.mergedRefs(in: "git merge interia-upgrade") == ["interia-upgrade"])
      #expect(GitCommandInspector.mergedRefs(in: #"git merge --no-ff -m "Merge it" interia-upgrade"#) == ["interia-upgrade"])
      #expect(GitCommandInspector.mergedRefs(in: "git -C ../pulse merge origin/interia-upgrade && npm test")
        == ["origin/interia-upgrade"])
      #expect(GitCommandInspector.mergedRefs(in: "git rebase main") == ["main"])
      #expect(GitCommandInspector.mergedRefs(in: "git rebase --onto upgrade main topic") == ["upgrade", "main"])
      #expect(GitCommandInspector.mergedRefs(in: "git pull origin feature-x") == ["origin/feature-x"])
      #expect(GitCommandInspector.mergedRefs(in: "git pull . feature-x") == ["feature-x"])
      #expect(GitCommandInspector.mergedRefs(in: "git pull") == [])
      #expect(GitCommandInspector.mergedRefs(in: "echo 'git merge x'; git status") == [], "quoted text isn't a command")
      #expect(GitCommandInspector.mergedRefs(in: "git log --oneline main..upgrade") == [])
    }

    @Test func headMoves() {
      #expect(GitCommandInspector.movesHead("git pull --rebase"))
      #expect(GitCommandInspector.movesHead("cd x && git switch main"))
      #expect(!GitCommandInspector.movesHead("git status && git diff"))
      #expect(!GitCommandInspector.movesHead("npm install"))
    }
  }
#endif
