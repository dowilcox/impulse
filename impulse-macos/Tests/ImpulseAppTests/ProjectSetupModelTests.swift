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
  }
#endif
