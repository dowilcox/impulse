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

    @Test func scriptsAreReadCommandByCommand() {
      let refs = GitCommandInspector.mergedRefs(in:)
      #expect(refs("npm test\ngit merge feature") == ["feature"])
      #expect(refs("git fetch\n  git merge feature\nnpm test") == ["feature"])
      #expect(refs("(cd ../app && git merge feature)") == ["feature"])
      #expect(refs("{ git merge feature; npm test; }") == ["feature"])
      #expect(refs("! git merge feature") == ["feature"])
      #expect(refs("if git merge feature; then npm test; fi") == ["feature"])
      #expect(refs("for b in a; do git pull origin feature; done") == ["origin/feature"])
      #expect(refs("echo $(git merge feature)") == ["feature"])
      #expect(refs("echo `git rebase main`") == ["main"])
      #expect(refs("git merge \\\n  feature") == ["feature"], "a continued line is one command")
      #expect(refs("echo 'é' && git merge feature") == ["feature"], "non-ASCII text before it")
      // Quoted, it's text.
      #expect(refs("git commit -m \"fix (merge it)\ngit merge x\"") == [])
      #expect(refs("echo 'a\ngit merge x'") == [])
      #expect(!GitCommandInspector.movesHead("git commit -m \"subject\n\ngit checkout body\""))
    }

    @Test func wrappersAreSeenThrough() {
      let refs = GitCommandInspector.mergedRefs(in:)
      #expect(refs("env GIT_EDITOR=true git merge feature") == ["feature"])
      #expect(refs("env -i -u HOME A=b /usr/bin/git merge feature") == ["feature"])
      #expect(refs("GIT_EDITOR=true git merge feature") == ["feature"])
      #expect(refs("command git merge feature") == ["feature"])
      #expect(refs("time -p git merge feature") == ["feature"])
      #expect(refs("sudo -u sam git merge feature") == ["feature"])
      #expect(refs("nice -n 10 git merge feature") == ["feature"])
      #expect(refs("nohup git pull origin feature") == ["origin/feature"])
      #expect(refs("if FOO=1 nohup env git rebase main; then :; fi") == ["main"])
      #expect(refs("command -v git") == [])
      #expect(refs("env git") == [])
    }

    @Test func globalOptionsWithValues() {
      #expect(GitCommandInspector.mergedRefs(in: "git --config-env core.editor=EDITOR merge feature") == ["feature"])
      #expect(GitCommandInspector.mergedRefs(in: "git --config-env=core.editor=EDITOR merge feature") == ["feature"])
      #expect(GitCommandInspector.mergedRefs(in: "git -c merge.ff=false -C ../app merge feature") == ["feature"])
    }
  }
#endif
