#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  /// Impulse's zsh startup files, run by the real zsh: the user's files are
  /// read from where zsh would read them, and the integration still loads.
  struct ZshStartupTests {
    private let zsh = "/bin/zsh"

    /// A scratch home with the given files, Impulse's startup files in their
    /// own folder, and the lines a login shell printed while starting.
    /// `systemZshrc` runs just before Impulse's .zshrc, where macOS's
    /// /etc/zshrc runs (the test can't rely on that file's contents).
    private func start(
      files: [String: String], inheritedZdotdir: ((String) -> String)? = nil, systemZshrc: String = "",
      command: String = "print -r -- \"end ZDOTDIR=${ZDOTDIR-unset}\""
    ) throws -> (home: String, lines: [String]) {
      let root = FileManager.default.temporaryDirectory.appendingPathComponent("zsh-startup-\(UUID().uuidString)")
      defer { try? FileManager.default.removeItem(at: root) }
      let home = root.appendingPathComponent("home").path
      let impulse = root.appendingPathComponent("impulse").path
      for (path, contents) in files {
        let url = URL(fileURLWithPath: home).appendingPathComponent(path)
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try contents.write(to: url, atomically: true, encoding: .utf8)
      }
      try FileManager.default.createDirectory(atPath: impulse, withIntermediateDirectories: true)
      let startup = ShellIntegration.zshStartupFiles(
        integration: "print -r -- \"integration ZDOTDIR=${ZDOTDIR-unset}\"\n",
        userZdotdir: inheritedZdotdir?(home))
      for (name, contents) in startup {
        let prefix = name == ".zshrc" ? systemZshrc : ""
        try (prefix + contents).write(toFile: impulse + "/" + name, atomically: true, encoding: .utf8)
      }
      let output = try ChildProcess.run(
        zsh, ["-i", "-l", "-c", command],
        environment: ["HOME": home, "ZDOTDIR": impulse, "PATH": "/usr/bin:/bin", "TERM": "dumb"],
        timeout: 20)
      let lines = String(decoding: output.stdout, as: UTF8.self).split(separator: "\n").map(String.init)
      return (home, lines)
    }

    private func announce(_ name: String) -> String {
      "print -r -- \"\(name) ZDOTDIR=${ZDOTDIR-unset}\"\n"
    }

    @Test func readsTheUsualFilesFromHome() throws {
      guard FileManager.default.isExecutableFile(atPath: zsh) else { return }
      let (_, lines) = try start(files: [
        ".zshenv": announce("zshenv"), ".zprofile": announce("zprofile"),
        ".zshrc": announce("zshrc"), ".zlogin": announce("zlogin"),
      ])
      #expect(
        lines == [
          "zshenv ZDOTDIR=unset", "zprofile ZDOTDIR=unset", "zshrc ZDOTDIR=unset",
          "integration ZDOTDIR=unset", "zlogin ZDOTDIR=unset", "end ZDOTDIR=unset",
        ])
    }

    @Test func followsAZdotdirSetInTheUsersZshenv() throws {
      guard FileManager.default.isExecutableFile(atPath: zsh) else { return }
      let (home, lines) = try start(files: [
        ".zshenv": announce("zshenv") + "export ZDOTDIR=$HOME/.config/zsh\n",
        ".zshrc": announce("home-zshrc"),
        ".config/zsh/.zprofile": announce("zprofile"),
        ".config/zsh/.zshrc": announce("zshrc") + "typeset -g FROM_ZSHRC=yes\n",
        ".config/zsh/.zlogin": announce("zlogin") + "print -r -- \"zshrc variable $FROM_ZSHRC\"\n",
      ])
      let dots = home + "/.config/zsh"
      #expect(
        lines == [
          "zshenv ZDOTDIR=unset", "zprofile ZDOTDIR=\(dots)", "zshrc ZDOTDIR=\(dots)",
          "integration ZDOTDIR=\(dots)", "zlogin ZDOTDIR=\(dots)", "zshrc variable yes", "end ZDOTDIR=\(dots)",
        ])
    }

    @Test func usesAZdotdirImpulseInherited() throws {
      guard FileManager.default.isExecutableFile(atPath: zsh) else { return }
      let (home, lines) = try start(
        files: [
          ".zshrc": announce("home-zshrc"),
          "dots/.zshenv": announce("zshenv"), "dots/.zshrc": announce("zshrc"),
        ], inheritedZdotdir: { $0 + "/dots" })
      let dots = home + "/dots"
      #expect(
        lines == [
          "zshenv ZDOTDIR=\(dots)", "zshrc ZDOTDIR=\(dots)", "integration ZDOTDIR=\(dots)", "end ZDOTDIR=\(dots)",
        ])
    }

    @Test func historyIsKeptWhereZshWouldKeepIt() throws {
      guard FileManager.default.isExecutableFile(atPath: zsh) else { return }
      // What /etc/zshrc does, while ZDOTDIR is still Impulse's folder.
      let systemZshrc = "HISTFILE=${ZDOTDIR:-$HOME}/.zsh_history\n"
      let printHistfile = "print -r -- \"HISTFILE=$HISTFILE\""
      let (home, lines) = try start(files: [:], systemZshrc: systemZshrc, command: printHistfile)
      #expect(lines.last == "HISTFILE=\(home)/.zsh_history")

      let (dotsHome, dotsLines) = try start(
        files: [".zshenv": "export ZDOTDIR=$HOME/.config/zsh\n"], systemZshrc: systemZshrc, command: printHistfile)
      #expect(dotsLines.last == "HISTFILE=\(dotsHome)/.config/zsh/.zsh_history")

      // A HISTFILE the user's .zshrc sets is theirs.
      let (ownHome, ownLines) = try start(
        files: [".zshrc": "HISTFILE=$HOME/.history/zsh\n"], systemZshrc: systemZshrc, command: printHistfile)
      #expect(ownLines.last == "HISTFILE=\(ownHome)/.history/zsh")
    }

    @Test func quotesTheInheritedFolder() {
      let files = ShellIntegration.zshStartupFiles(integration: "", userZdotdir: "/tmp/it's here")
      #expect(files[".zshenv"]?.contains("__impulse_user_zdotdir='/tmp/it'\\''s here'") == true)
      #expect(Set(files.keys) == [".zshenv", ".zprofile", ".zshrc"])
    }
  }
#endif
