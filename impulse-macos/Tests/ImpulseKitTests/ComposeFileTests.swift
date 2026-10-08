#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct ComposeFileTests {
    /// Shaped like a Laravel project's dev stack.
    private let laravel = """
      # Local development
      services:
        app:
          build: .
          container_name: pulseboard-app
          ports:
            - "8000:8000"
            - "${VITE_PORT:-5173}:5173" # Vite
          volumes:
            - .:/var/www
            - /var/www/vendor
            - /var/www/node_modules
        mysql:
          image: mysql:8.4
          container_name: pulseboard-mysql
          ports: ["127.0.0.1:3306:3306"]
          volumes:
            - ./docker/data/mysql:/var/lib/mysql
        redis:
          image: "redis:8"
          ports:
            - target: 6379
              published: 6379

      volumes:
        cache: {}
      """

    @Test func servicesPortsAndVolumesAreRead() {
      let file = ComposeFile.parse(laravel)
      #expect(file.services.map(\.name) == ["app", "mysql", "redis"])
      let app = file.services[0]
      #expect(app.containerName == "pulseboard-app")
      #expect(app.ports.map(\.host) == [8000, nil])
      #expect(app.ports[1].variable == "VITE_PORT")
      #expect(app.ports[1].variableDefault == 5173)
      #expect(app.volumes.map(\.isAnonymous) == [false, true, true])
      #expect(app.volumes[2].target == "/var/www/node_modules")
      let mysql = file.services[1]
      #expect(mysql.image == "mysql:8.4")
      #expect(mysql.ports.map(\.host) == [3306])
      #expect(mysql.volumes.first?.hostPath == "./docker/data/mysql")
      #expect(mysql.volumes.first?.target == "/var/lib/mysql")
      #expect(file.services[2].image == "redis:8")
      #expect(file.services[2].ports.isEmpty, "the long syntax is left alone")
    }

    @Test func portVariablesWithoutADefault() {
      let port = ComposeFile.port("${PORT2}:80")
      #expect(port.variable == "PORT2")
      #expect(port.variableDefault == nil)
      #expect(ComposeFile.port("8080").host == nil, "a container port alone isn't published to a fixed port")
      #expect(ComposeFile.port("8000:8000/tcp").host == 8000)
    }

    @Test func aTasksOverrideRenamesAndMovesFixedPorts() throws {
      let override = try #require(ComposeFile.parse(laravel).override(task: "fix-elevation", slot: 1, offset: 100))
      #expect(
        override.hasSuffix(
          """
          services:
            app:
              container_name: pulseboard-app-fix-elevation
              ports: !override
                - "8100:8000"
                - "${VITE_PORT:-5173}:5173"
            mysql:
              container_name: pulseboard-mysql-fix-elevation
              ports: !override
                - "127.0.0.1:3406:3306"

          """))
      #expect(override.hasPrefix("# Written by Impulse for the task fix-elevation (slot 1)"))
    }

    @Test func nothingToOverrideWhenTheFileIsReadyAlready() {
      let ready = """
        services:
          web:
            ports:
              - "${WEB_PORT:-3000}:3000"
        """
      #expect(ComposeFile.parse(ready).override(task: "x", slot: 1, offset: 100) == nil)
    }
  }
#endif
