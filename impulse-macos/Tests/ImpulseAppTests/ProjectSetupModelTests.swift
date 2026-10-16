#if canImport(Testing)
  import AppKit
  @testable import ImpulseApp
  @testable import ImpulseKit
  import Testing

  struct ProjectSetupModelTests {
    private func model() -> ProjectSetupModel {
      let model = ProjectSetupModel(palette: ChromePalette(theme: ThemeManager.theme(forName: "nord")))
      model.dumpCommand = "DUMP"
      model.emptyCommand = "MIGRATE"
      return model
    }

    @Test func choosingADatabaseLeavesTheRestOfSetupWhole() {
      let model = model()
      model.choose(.dump)
      #expect(model.setup == "DUMP")
      model.setup += " && composer install"
      model.choose(.none)
      #expect(model.setup == "composer install", "not \"&& composer install\"")

      model.setup = "npm ci && DUMP && composer install"
      model.choose(.empty)
      #expect(model.setup == "npm ci && composer install && MIGRATE")
      model.choose(.none)
      #expect(model.setup == "npm ci && composer install")
    }

    @Test func changesAreToldFromWhatWasSaved() {
      let model = model()
      model.isLoading = false
      model.setup = "npm ci"
      model.ports = [.init(name: "APP_PORT", value: "8000")]
      #expect(!model.isModified, "nothing read or saved yet")
      model.savedConfig = model.config()
      #expect(!model.isModified)
      model.check = "npm test"
      #expect(model.isModified)
      model.check = "  "
      #expect(!model.isModified, "blank is no script, as saved")
      model.ports.append(.init(name: "", value: ""))
      #expect(!model.isModified, "an empty row isn't saved")
      model.markSaved(model.config(), text: "", local: nil)
      model.ports[0].value = "8001"
      #expect(model.isModified)
    }

    @Test func entriesThatCantBeSavedAreNamed() {
      let model = model()
      model.ports = [
        .init(name: "APP_PORT", value: "8000"), .init(name: "VITE_PORT", value: "51a3"), .init(name: "", value: "9000"),
        .init(name: "WEB PORT", value: "80"), .init(name: "", value: " "),
      ]
      #expect(model.ports.map(model.problem(port:)) == [
        nil, "Not a port number (1–65535)", "Needs a name", "Not a variable name (letters, digits and _)", nil,
      ])
      model.values = [
        .init(name: "APP_URL", value: "http://localhost:{APP_PORT}"), .init(name: "DB", value: "app_{tsk}"),
        .init(name: "JSON", value: #"{"a": 1}"#), .init(name: "APP_PORT", value: "1"),
      ]
      #expect(model.values.map(model.problem(value:)) == [
        nil, "No {tsk}: use {task}, {task_}, {slot} or a port's name", nil, "Also a port",
      ])
      #expect(model.problem(port: model.ports[0]) == "Also a value for each task")
      model.rules = [.init(name: "composer.lock", value: ""), .init(name: "a", value: "x"), .init(name: "a", value: "y")]
      #expect(model.rules.map(model.problem(rule:)) == ["Needs a command", "a is listed twice", "a is listed twice"])
      model.actions = [
        .init(isOn: true, action: .init(name: "test", command: "")), .init(isOn: false, action: .init(name: "", command: "x")),
        .init(isOn: true, action: .init(name: "", command: "")),
      ]
      #expect(model.actions.map(model.problem(action:)) == ["Needs a command", nil, nil], "unticked and empty rows are skipped")
      model.portOffset = "0"
      #expect(model.offsetProblem != nil)
      model.portOffset = ""
      #expect(model.offsetProblem == nil, "empty means the default")
      model.database = .clone
      #expect(model.databaseProblem != nil)
      #expect(model.problemCount == 11)
    }

    /// Typed or read from a file, numbers that aren't ports never crash
    /// the screen, and can't be saved.
    @Test func offsetsAndPortsPastTheLastPortAreProblems() {
      let model = model()
      model.ports = [.init(name: "APP_PORT", value: "8000")]
      for offset in ["0", "-5", "10001", "9223372036854775807", "99999999999999999999", "x"] {
        model.portOffset = offset
        #expect(model.offsetProblem == "A whole number from 1 to 10000", "\(offset)")
        #expect(model.offset == 100)
      }
      model.portOffset = "6000"
      #expect(model.offsetProblem == "Too big: task 10's APP_PORT would be past 65535")
      model.portOffset = "5754"
      #expect(model.offsetProblem == "Too big: task 10's APP_PORT would be past 65535")
      model.portOffset = "5753"
      #expect(model.offsetProblem == nil, "8000 + 10 × 5753 = 65530")
      model.portOffset = ""
      model.ports = [.init(name: "HIGH", value: "65000"), .init(name: "BIG", value: "9223372036854775807")]
      #expect(model.offsetProblem == "Too big: task 10's HIGH would be past 65535", "the default offset too")
      #expect(model.problem(port: model.ports[1]) == "Not a port number (1–65535)")
      _ = model.config()

      // With the override on, the Compose file's fixed ports count too.
      model.ports = []
      model.fixedPorts = [.init(service: "web", port: ComposeFile.port("65400:80"))]
      #expect(model.offsetProblem == nil)
      model.composeOverride = true
      #expect(model.offsetProblem == "Too big: task 10's web 65400 would be past 65535")
    }

    @Test func valuesCantHoldALineBreak() {
      let model = model()
      model.values = [
        .init(name: "APP_NAME", value: "Bob's {task}"), .init(name: "NOTE", value: "a\nb"), .init(name: "DB", value: "app_{task_}"),
      ]
      #expect(model.values.map(model.problem(value:)) == [nil, "Can't hold a line break", nil])
    }

    @Test func theTrustPromptListsCommandsAsTheyAre() {
      #expect(MainWindowController.listedCommand("npm ci") == "$ npm ci")
      #expect(MainWindowController.listedCommand("npx tsc --noEmit\nnpm test") == "$ npx tsc --noEmit\n  npm test")
      #expect(MainWindowController.listedCommand("a\r\nb\rc") == "$ a\n  b\n  c", "no line break hides one")
      #expect(MainWindowController.listedCommand("echo hi\u{1B}[2K\u{202E}x\u{200B}") == "$ echo hi\\u{1B}[2K\\u{202E}x\\u{200B}")
      #expect(MainWindowController.listedCommand("a\tb") == "$ a\tb")
    }
  }
#endif
