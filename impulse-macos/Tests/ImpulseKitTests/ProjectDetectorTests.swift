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
        "dotenv": """
          APP_URL=http://localhost:8000
          APP_PORT=8000
          DB_HOST=mysql
          DB_PORT=3306
          DB_DATABASE=pulseboard
          REDIS_HOST=127.0.0.1
          REDIS_PORT=nope
          MAIL_HOST=smtp.example.com
          MAIL_PORT=2525
          REVERB_HOST=localhost
          REVERB_PORT=8080

          """,
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
      #expect(
        found.ports == ["VITE_PORT": 5173, "FORWARD_DB_PORT": 3306, "APP_PORT": 8000, "REVERB_PORT": 8080],
        "DB_PORT is inside the Compose network and MAIL_PORT on another machine: neither moves")
      #expect(found.values == ["APP_URL": "http://localhost:{APP_PORT}"], "each task runs its own database")
      #expect(found.fixedPorts == [.init(service: "app", port: ComposeFile.port("8000:8000"))])
      #expect(found.containerNames == [.init(service: "app", name: "pulseboard-app")])
      #expect(found.setup == "docker compose up -d && npm ci && composer install")
      #expect(found.archive == "docker compose down -v")
      #expect(found.onChange == ["package-lock.json": "npm ci", "composer.lock": "composer install"])
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

    @Test func serversEveryCheckoutSharesKeepTheirPorts() throws {
      let root = FileManager.default.temporaryDirectory.appendingPathComponent("detect-\(UUID().uuidString)").path
      try FileManager.default.createDirectory(atPath: root, withIntermediateDirectories: true)
      defer { try? FileManager.default.removeItem(atPath: root) }
      // Laravel's own: MySQL, Redis and a mail catcher on this Mac.
      try """
        APP_URL=http://localhost
        DB_HOST=127.0.0.1
        DB_PORT=3306
        DB_DATABASE=laravel
        REDIS_HOST=127.0.0.1
        REDIS_PORT=6379
        MAIL_HOST=127.0.0.1
        MAIL_PORT=2525
        REVERB_HOST=localhost
        REVERB_PORT=8080
        """.write(toFile: (root as NSString).appendingPathComponent("." + "env"), atomically: true, encoding: .utf8)
      var found = ProjectDetector.suggest(root: root, ignored: [])
      #expect(found.ports == ["REVERB_PORT": 8080], "the servers are shared; Reverb is the app's own")
      #expect(found.values == ["DB_DATABASE": "laravel_{task_}"])

      // Published by the Compose file, the database is the project's own.
      try "services:\n  db:\n    image: mysql:8.4\n    ports:\n      - \"3306:3306\"\n".write(
        toFile: (root as NSString).appendingPathComponent("compose.yaml"), atomically: true, encoding: .utf8)
      found = ProjectDetector.suggest(root: root, ignored: [])
      #expect(found.ports == ["DB_PORT": 3306, "REVERB_PORT": 8080])
    }

    @Test func scriptsRunWithTheProjectsPackageManager() throws {
      let root = FileManager.default.temporaryDirectory.appendingPathComponent("detect-\(UUID().uuidString)").path
      try FileManager.default.createDirectory(atPath: root, withIntermediateDirectories: true)
      defer { try? FileManager.default.removeItem(atPath: root) }
      func write(_ name: String, _ text: String) throws {
        try text.write(toFile: (root as NSString).appendingPathComponent(name), atomically: true, encoding: .utf8)
      }
      try write("package.json", #"{"scripts": {"dev": "vite", "lint": "eslint .", "test": "vitest"}}"#)
      try write("pnpm-lock.yaml", "")
      var found = ProjectDetector.suggest(root: root, ignored: [])
      #expect(found.check == "pnpm run lint && pnpm test")
      #expect(found.actions.first == .init(name: "dev", command: "pnpm run dev"))

      try FileManager.default.removeItem(atPath: (root as NSString).appendingPathComponent("pnpm-lock.yaml"))
      try write("bun.lock", "")
      found = ProjectDetector.suggest(root: root, ignored: [])
      #expect(found.check == "bun run lint && bun run test", "bun test is Bun's own runner")

      try write("package.json", #"{"packageManager": "yarn@4.5.0", "scripts": {"test": "vitest"}}"#)
      found = ProjectDetector.suggest(root: root, ignored: [])
      #expect(found.check == "yarn test", "packageManager wins over the lock file")
    }

    @Test func databaseCommandsForSetup() {
      let mysql = ProjectDetector.dumpAndLoad(service: "mysql", image: "mysql:8.4")
      #expect(
        mysql
          == #"sh -c 'docker compose up -d mysql && n=0 && until docker compose exec -T mysql sh -c "mysqladmin ping -uroot -p\"\$MYSQL_ROOT_PASSWORD\" --silent"; do n=$((n+1)); if [ $n -ge 120 ]; then echo "mysql did not start" >&2; exit 1; fi; sleep 1; done && (cd "$IMPULSE_REPO_ROOT" && docker compose exec -T mysql sh -c "mysqldump -uroot -p\"\$MYSQL_ROOT_PASSWORD\" --all-databases --single-transaction") | docker compose exec -T mysql sh -c "mysql -uroot -p\"\$MYSQL_ROOT_PASSWORD\""'"#
      )
      #expect(ProjectDetector.dumpAndLoad(service: "db", image: "postgres:17")?.contains(#""pg_dumpall -U \"\$POSTGRES_USER\"""#) == true)
      #expect(ProjectDetector.dumpAndLoad(service: "cache", image: "redis:8") == nil)
      #expect(
        ProjectDetector.earlierDumpAndLoad(service: "mysql", image: "mysql:8.4")?
          .hasPrefix("until docker compose exec -T mysql sh -c 'mysqladmin ping") == true)
      #expect(ProjectDetector.earlierDumpAndLoad(service: "db", image: "postgres:17")?.contains("pg_dumpall") == true)
    }

    /// The service name goes into the script's single quotes, which the
    /// user's shell (bash, zsh or fish) reads: only names Compose allows.
    @Test func onlyPlainServiceNamesGetDumpAndLoad() {
      for name in ["db'; touch PWNED; '", "db$(id)", "my db", "db\\", "-p", "db\"x", "db\nx", ""] {
        #expect(ProjectDetector.dumpAndLoad(service: name, image: "mysql:8.4") == nil, "\(name)")
        #expect(ProjectDetector.earlierDumpAndLoad(service: name, image: "mysql:8.4") == nil, "\(name)")
      }
      #expect(ProjectDetector.dumpAndLoad(service: "mysql_1.db-x", image: "mysql:8.4")?.contains("exec -T mysql_1.db-x sh") == true)
    }

    /// The command as the user's shell hands it to sh: the database's own
    /// variables are left for the container, and the loop gives up.
    @Test func dumpAndLoadRunsUnderSh() throws {
      let command = try #require(ProjectDetector.dumpAndLoad(service: "mysql", image: "mysql:8.4"))
      let folder = FileManager.default.temporaryDirectory.appendingPathComponent("dump-\(UUID().uuidString)").path
      try FileManager.default.createDirectory(atPath: folder, withIntermediateDirectories: true)
      defer { try? FileManager.default.removeItem(atPath: folder) }
      // A docker that records its arguments: up succeeds, ping fails twice.
      let docker = """
        #!/bin/sh
        echo "$@" >> "\(folder)/calls"
        case "$*" in
          *"up -d"*) exit 0 ;;
          *ping*) [ "$(grep -c ping "\(folder)/calls")" -ge 3 ] && exit 0 || exit 1 ;;
          *) exit 0 ;;
        esac
        """
      let dockerPath = (folder as NSString).appendingPathComponent("docker")
      try docker.write(toFile: dockerPath, atomically: true, encoding: .utf8)
      try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: dockerPath)
      let process = Process()
      process.executableURL = URL(fileURLWithPath: "/bin/sh")
      process.arguments = ["-c", command.replacingOccurrences(of: "sleep 1", with: "true")]
      process.environment = ["PATH": "\(folder):/usr/bin:/bin", "IMPULSE_REPO_ROOT": folder]
      try process.run()
      process.waitUntilExit()
      #expect(process.terminationStatus == 0)
      let calls = try String(contentsOfFile: (folder as NSString).appendingPathComponent("calls"), encoding: .utf8)
      #expect(calls.hasPrefix("compose up -d mysql\n"))
      #expect(calls.contains(#"compose exec -T mysql sh -c mysqladmin ping -uroot -p"$MYSQL_ROOT_PASSWORD" --silent"#))
      #expect(calls.contains(#"sh -c mysqldump -uroot -p"$MYSQL_ROOT_PASSWORD" --all-databases"#))
    }

    @Test func dumpAndLoadGivesUpOnADatabaseThatNeverStarts() throws {
      let command = try #require(ProjectDetector.dumpAndLoad(service: "mysql", image: "mysql:8.4"))
      let folder = FileManager.default.temporaryDirectory.appendingPathComponent("dump-\(UUID().uuidString)").path
      try FileManager.default.createDirectory(atPath: folder, withIntermediateDirectories: true)
      defer { try? FileManager.default.removeItem(atPath: folder) }
      let dockerPath = (folder as NSString).appendingPathComponent("docker")
      try "#!/bin/sh\ncase \"$*\" in *ping*) exit 1 ;; esac\n".write(toFile: dockerPath, atomically: true, encoding: .utf8)
      try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: dockerPath)
      let process = Process()
      process.executableURL = URL(fileURLWithPath: "/bin/sh")
      process.arguments = ["-c", command.replacingOccurrences(of: "sleep 1", with: "true")]
      process.environment = ["PATH": "\(folder):/usr/bin:/bin", "IMPULSE_REPO_ROOT": folder]
      process.standardError = FileHandle.nullDevice
      try process.run()
      process.waitUntilExit()
      #expect(process.terminationStatus == 1)
    }

    @Test func makeTargetsAndEnvValues() {
      #expect(ProjectDetector.makeTargets("all: build\nbuild:\n\tcc\n.PHONY: all\nX = 1\nY := 2\n") == ["all", "build"])
      #expect(ProjectDetector.parseEnv("A=1\nexport B=\"two words\"\n# C=3\n") == ["A": "1", "B": "two words"])
    }
  }
#endif
