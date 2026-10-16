#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct ShellQuotingTests {
    /// Words that break naive quoting: quotes, backslashes (one at the end
    /// leaves a fish quote open), substitutions, newlines, option-looking
    /// names, globs and expansions, text beyond ASCII, and control
    /// characters (`Icon\r` is the file behind a custom folder icon).
    static let words = [
      "plain", "it's", "a\\b", "trailing\\", "\\'", "a\\';echo INJECTED;#", "$(touch PWNED)", "`id`",
      "line1\nline2", "tab\there", "-rf", "日本語 café 🚀", "a b", "!event", "~user", "~/x", "*?[a]",
      "{a,b}", "#comment", "semi;colon&|<>", "\"double\"", "%self", "=cmd", "a^b", "$HOME", "",
      "Icon\r", "x\u{15}touch MARKER;echo '", "\u{1b}[2Jcls", "del\u{7f}", "c1\u{9b}x", "\r\r",
    ]

    /// The shells to try, with the flags that skip the user's startup files.
    static let shells: [(type: ShellType, path: String, flags: [String])] = {
      var shells: [(ShellType, String, [String])] = [
        (.bash, "/bin/sh", []), (.bash, "/bin/bash", ["--norc", "--noprofile"]), (.zsh, "/bin/zsh", ["-f"]),
      ]
      // fish isn't part of macOS; skip it where it isn't installed.
      for path in ["/opt/homebrew/bin/fish", "/usr/local/bin/fish", "/usr/bin/fish"]
      where FileManager.default.isExecutableFile(atPath: path) {
        shells.append((.fish, path, ["--no-config"]))
        break
      }
      return shells
    }()

    /// What `shell` passes to a program for `word` (each argument printed
    /// followed by a NUL), run in an empty folder.
    private func arguments(_ word: String, shell: (type: ShellType, path: String, flags: [String])) throws -> [String] {
      let base = FileManager.default.temporaryDirectory.appendingPathComponent("impulse-quoting-\(UUID().uuidString)")
      let folder = base.appendingPathComponent("work")
      let home = base.appendingPathComponent("home")
      try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
      try FileManager.default.createDirectory(at: home, withIntermediateDirectories: true)
      defer { try? FileManager.default.removeItem(at: base) }
      let result = try ChildProcess.run(
        shell.path, shell.flags + ["-c", "printf '%s\\0' " + word], in: folder.path,
        environment: ["LANG": "en_US.UTF-8", "PATH": "/usr/bin:/bin", "HOME": home.path], timeout: 10)
      #expect(try FileManager.default.contentsOfDirectory(atPath: folder.path).isEmpty, "\(word) ran something")
      var output = String(decoding: result.stdout, as: UTF8.self)
      guard output.hasSuffix("\0") else { return [] }
      output.removeLast()
      return output.components(separatedBy: "\0")
    }

    @Test func quotedWordsReachTheProgramIntact() throws {
      for shell in Self.shells {
        for word in Self.words {
          let quoted = ShellQuoting.quoted(word, for: shell.type)
          #expect(try arguments(quoted, shell: shell) == [word], "\(shell.path): \(word) as \(quoted)")
        }
      }
    }

    @Test func escapedWordsReachTheProgramIntact() throws {
      for shell in Self.shells {
        for word in Self.words where !word.isEmpty {
          let escaped = ShellQuoting.escaped(word, for: shell.type)
          #expect(try arguments(escaped, shell: shell) == [word], "\(shell.path): \(word) as \(escaped)")
        }
      }
    }

    @Test func quotingIsPerShell() {
      #expect(ShellQuoting.quoted("it's", for: .zsh) == "'it'\\''s'")
      #expect(ShellQuoting.quoted("it's", for: .fish) == "'it\\'s'")
      #expect(ShellQuoting.quoted("a\\b", for: .bash) == "'a\\b'")
      #expect(ShellQuoting.quoted("a\\b", for: .fish) == "'a\\\\b'")
      // Nothing to quote.
      #expect(ShellQuoting.quoted("src/main.swift", for: .fish) == "src/main.swift")
      #expect(ShellQuoting.quoted("café", for: .bash) == "café")
      #expect(ShellQuoting.quoted("", for: .bash) == "''")
      #expect(ShellQuoting.escaped("my file (1).txt", for: .zsh) == "my\\ file\\ \\(1\\).txt")
    }

    @Test func controlCharactersAreWrittenAsEscapes() {
      // Never typed raw (^U would clear the line and run the rest), but kept.
      #expect(ShellQuoting.quoted("Icon\r", for: .bash) == "'Icon'$'\\x0d'")
      #expect(ShellQuoting.quoted("Icon\r", for: .fish) == "'Icon'\\u000d")
      #expect(ShellQuoting.quoted("a\u{9b}b", for: .zsh) == "'a'$'\\xc2\\x9b''b'")
      #expect(ShellQuoting.escaped("a\u{3}b", for: .bash) == "a$'\\x03'b")
      #expect(ShellQuoting.escaped("a\u{3}b", for: .fish) == "a\\u0003b")
      for shell in [ShellType.bash, .zsh, .fish] {
        let text = ShellQuoting.quoted("x\u{15}touch MARKER;echo '", for: shell)
        #expect(!text.unicodeScalars.contains(where: ShellQuoting.isControl))
      }
      #expect(ShellQuoting.withoutControlCharacters("a\tb\nc\r\u{0}") == "a\tb\nc")
    }

    @Test func shellCompletionsKeepTheirHead() {
      // fish answers as typed: a leading `~/` or `$NAME/` still expands.
      #expect(ShellQuoting.completionText("~/Library/", typed: "~/Lib", for: .fish) == "~/Library/")
      #expect(ShellQuoting.completionText("~/Library/", typed: "~/lib", for: .fish) == "~/Library/")
      #expect(ShellQuoting.completionText("$HOME/my dir/", typed: "$HOME/my", for: .fish) == "$HOME/my\\ dir/")
      #expect(ShellQuoting.completionText("$HOME", typed: "$HO", for: .fish) == "$HOME")
      // Everything else is a name.
      #expect(ShellQuoting.completionText("x$(touch PWNED)/", typed: "x", for: .fish) == "x\\$\\(touch\\ PWNED\\)/")
      #expect(ShellQuoting.completionText("my dir/", typed: "my\\ d", for: .fish) == "my\\ dir/")
      #expect(ShellQuoting.completionText("src/a b.txt", typed: "src/a\\ ", for: .fish) == "src/a\\ b.txt")
      #expect(ShellQuoting.completionText("feature/it's", typed: "", for: .fish) == "feature/it\\'s")
      #expect(ShellQuoting.completionText("~$Report.docx", typed: "~$", for: .fish) == "\\~\\$Report.docx")
      #expect(ShellQuoting.completionText("~", typed: "~", for: .fish) == "\\~")
      #expect(ShellQuoting.completionText("a\\\\x", typed: "a\\\\", for: .fish) == "a\\\\\\\\x")
    }

    @Test func impulsesOwnCandidatesAreLiteral() {
      func path(_ value: String, kept: String = "") -> CompletionCandidate {
        CompletionCandidate(value: value, display: value, kind: "path", isDir: false, gitStatus: nil, keptPrefix: kept)
      }
      // A folder named `~` or `$HOME` is a name, not the home folder.
      #expect(path("~/").insertion(typed: "~", shell: .bash) == "\\~/")
      #expect(path("$HOME/").insertion(typed: "$H", shell: .bash) == "\\$HOME/")
      #expect(path("~$Report.docx").insertion(typed: "~$", shell: .zsh) == "\\~\\$Report.docx")
      // A home prefix the completer followed stays as typed.
      #expect(path("~/Library/", kept: "~/").insertion(typed: "~/Lib", shell: .bash) == "~/Library/")
      #expect(path("$HOME/my dir/", kept: "$HOME/").insertion(typed: "$HOME/my", shell: .fish) == "$HOME/my\\ dir/")
      // Branches and other values too.
      let branch = CompletionCandidate(value: "feat$x", display: "feat$x", kind: "branch", isDir: false, gitStatus: nil)
      #expect(branch.insertion(typed: "feat$", shell: .bash) == "feat\\$x")
    }
  }
#endif
