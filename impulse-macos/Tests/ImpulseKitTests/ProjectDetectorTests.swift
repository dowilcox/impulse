#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct ProjectDetectorTests {
    /// A Laravel project with a Docker dev stack, like PulseBoard.
    private func laravelProject() throws -> String {
      let root = FileManager.default.temporaryDirectory.appendingPathComponent("detect-\(UUID().uuidString)").path
      try FileManager.default.createDirectory(atPath: root, withIntermediateDirectories: true)
      let files: [String: String] = [
        "docker-compose.yml": """
          services:
            app:
              container_name: pulseboard-app
              ports:
                - "8000:8000"
                - "${VITE_PORT:-5173}:5173"
              volumes:
                - .:/var/www
                - /var/www/vendor
                - /var/www/node_modules
            mysql:
              image: mysql:8.4
              ports:
                - "${FORWARD_DB_PORT:-3306}:3306"
              volumes:
                - ./docker/data/mysql:/var/lib/mysql
          """,
        "dotenv": "APP_URL=http://localhost:8000\nAPP_PORT=8000\nDB_DATABASE=pulseboard\nREDIS_PORT=nope\n",
        "composer.lock": "{}",
        "package-lock.json": "{}",
        "package.json": #"{"scripts": {"dev": "vite", "build": "vite build", "test": "vitest", "typecheck": "tsc --noEmit"}}"#,
        "composer.json": #"{"scripts": {"test": "phpunit", "post-install-cmd": "x"}}"#,
        "Makefile": ".PHONY: up\nup:\n\tdocker compose up -d\nVAR := 1\n%.o: %.c\nfresh: up\n",
      ]
      for (name, text) in files {
        let path = (root as NSString).appendingPathComponent(name == "dotenv" ? "." + "env" : name)
        try text.write(toFile: path, atomically: true, encoding: .utf8)
      }
      return root
    }

    @Test func aDockerLaravelProject() throws {
      let root = try laravelProject()
      defer { try? FileManager.default.removeItem(atPath: root) }
      let env = "." + "env"
      let found = ProjectDetector.suggest(
        root: root,
        ignored: [env, env + ".testing", "vendor/", "node_modules/", "public/build/", "storage/logs/laravel.log"])

      #expect(found.copies == [.init(env, suggested: true), .init(env + ".testing", suggested: false)])
      #expect(found.clones.map(\.path) == ["vendor", "node_modules", "public/build"])
      #expect(found.clones.map(\.suggested) == [false, false, true], "the containers can't see vendor or node_modules")
      #expect(found.clones[0].note?.contains("anonymous volume") == true)
      #expect(found.ports == ["VITE_PORT": 5173, "FORWARD_DB_PORT": 3306, "APP_PORT": 8000])
      #expect(found.values == ["APP_URL": "http://localhost:{APP_PORT}"], "each task runs its own database")
      #expect(found.composeWarnings == ["app: container_name pulseboard-app", "app: fixed port 8000:8000"])
      #expect(found.setup == "docker compose up -d && npm ci && composer install")
      #expect(found.archive == "docker compose down -v")
      #expect(found.check == "npm run typecheck && npm test && composer test")
      #expect(found.actions.map(\.name) == ["build", "dev", "test", "typecheck", "composer test", "make up", "make fresh"])
      #expect(found.databaseFolder == "docker/data/mysql")
      #expect(found.databaseService == "mysql")
      #expect(found.composeFileName == "docker-compose.yml")
    }

    @Test func aSharedDatabaseServerGetsADatabasePerTask() throws {
      let root = FileManager.default.temporaryDirectory.appendingPathComponent("detect-\(UUID().uuidString)").path
      try FileManager.default.createDirectory(atPath: root, withIntermediateDirectories: true)
      defer { try? FileManager.default.removeItem(atPath: root) }
      try "DB_DATABASE=trailhead\nPORT=3000\nBASE_URL=http://localhost:3000\n".write(
        toFile: (root as NSString).appendingPathComponent("." + "env"), atomically: true, encoding: .utf8)
      let found = ProjectDetector.suggest(root: root, ignored: [])
      #expect(found.values == ["DB_DATABASE": "trailhead_{task_}"], "PORT isn't read: only _PORT names")
      #expect(found.setup == nil)
      #expect(found.archive == nil)
      #expect(found.databaseFolder == nil)
    }

    @Test func databaseCommandsForSetup() {
      let mysql = ProjectDetector.dumpAndLoad(service: "mysql", image: "mysql:8.4")
      #expect(mysql?.hasPrefix("until docker compose exec -T mysql sh -c 'mysqladmin ping") == true)
      #expect(mysql?.contains(#"(cd "$IMPULSE_REPO_ROOT" && docker compose exec -T mysql sh -c 'mysqldump"#) == true)
      #expect(mysql?.hasSuffix(#"| docker compose exec -T mysql sh -c 'mysql -uroot -p"$MYSQL_ROOT_PASSWORD"'"#) == true)
      #expect(ProjectDetector.dumpAndLoad(service: "db", image: "postgres:17")?.contains("pg_dumpall") == true)
      #expect(ProjectDetector.dumpAndLoad(service: "cache", image: "redis:8") == nil)
    }

    @Test func makeTargetsAndEnvValues() {
      #expect(ProjectDetector.makeTargets("all: build\nbuild:\n\tcc\n.PHONY: all\nX = 1\nY := 2\n") == ["all", "build"])
      #expect(ProjectDetector.parseEnv("A=1\nexport B=\"two words\"\n# C=3\n") == ["A": "1", "B": "two words"])
    }
  }
#endif
