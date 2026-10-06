#if canImport(Testing)
  @testable import ImpulseApp
  import Testing

  struct PaletteModelTests {
    @Test func reassigningTheSameQueryKeepsTheHighlightedRow() {
      let model = PaletteModel()
      model.commands = ["Alpha", "Beta", "Gamma"].map { title in
        AppCommand(id: title.lowercased(), title: title, category: "Test") {}
      }
      model.query = ">"
      #expect(model.rows.count == 3)
      model.moveSelection(1)
      model.moveSelection(1)
      #expect(model.selectedIndex == 2)
      // What the text field does on Return, just before the submit runs.
      model.query = ">"
      #expect(model.selectedIndex == 2)
      // A different query starts again from the top.
      model.query = ">a"
      #expect(model.selectedIndex == 0)
    }
  }
#endif
