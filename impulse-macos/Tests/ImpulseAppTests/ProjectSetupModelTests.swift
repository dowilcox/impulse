#if canImport(Testing)
  import AppKit
  @testable import ImpulseApp
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
  }
#endif
