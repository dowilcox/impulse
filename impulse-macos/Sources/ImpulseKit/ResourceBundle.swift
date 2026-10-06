import Foundation

extension Bundle {
  /// ImpulseKit's resources (themes, shell integration scripts). In the app
  /// they're in Contents/Resources — SwiftPM's generated `Bundle.module` only
  /// looks beside the executable (where codesign rejects bundles) and in the
  /// build folder of the machine that built it, so a shipped app would crash
  /// without this. Development builds and tests use `Bundle.module`.
  static let kitResources: Bundle = {
    if let url = Bundle.main.resourceURL?.appendingPathComponent("ImpulseApp_ImpulseKit.bundle"),
      let bundle = Bundle(url: url)
    {
      return bundle
    }
    return Bundle.module
  }()
}
