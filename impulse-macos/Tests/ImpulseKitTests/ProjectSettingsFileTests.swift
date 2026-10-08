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

    @Test func mergingIntoAFileWithoutManagedSectionsAppends() {
      let merged = ProjectSettingsFile.merging(ProjectConfig(setupScript: "npm ci"), into: "[lsp]\nx = 1\n")
      #expect(merged == "[lsp]\nx = 1\n\n[scripts]\nsetup = \"npm ci\"\n")
    }
  }
#endif
