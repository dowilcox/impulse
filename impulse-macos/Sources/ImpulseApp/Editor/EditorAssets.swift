import Foundation

/// Locates the Monaco editor assets bundled into the app's SwiftPM resource
/// bundle (populated by build.sh from vendor/monaco + web/), replacing the
/// old Rust `include_dir!` extraction to Application Support.
enum EditorAssets {
  /// The bundled Monaco directory containing vs/, fonts/, highlight/,
  /// editor.html/js, and review.html/js.
  static let monacoDirectory: URL? = {
    let url = Bundle.appResources.resourceURL?
      .appendingPathComponent("monaco", isDirectory: true)
    guard let url, FileManager.default.fileExists(atPath: url.path) else {
      NSLog("Bundled Monaco directory missing — run build.sh to populate Resources/monaco")
      return nil
    }
    return url
  }()

  /// Install the bundled JetBrains Mono and Inter fonts into ~/Library/Fonts
  /// so the terminal renderer can use them, mirroring the Rust
  /// `install_user_fonts`. Existing files are never overwritten. Safe to call
  /// from a background queue.
  static func installUserFontsIfNeeded() {
    guard let monacoDir = monacoDirectory else { return }
    let fontsRoot = monacoDir.appendingPathComponent("fonts", isDirectory: true)
    let targetDir = FileManager.default.homeDirectoryForCurrentUser
      .appendingPathComponent("Library/Fonts", isDirectory: true)
    try? FileManager.default.createDirectory(at: targetDir, withIntermediateDirectories: true)

    for family in ["jetbrains-mono", "inter"] {
      let familyDir = fontsRoot.appendingPathComponent(family, isDirectory: true)
      guard
        let files = try? FileManager.default.contentsOfDirectory(
          at: familyDir, includingPropertiesForKeys: nil)
      else { continue }
      for file in files where file.pathExtension == "ttf" {
        let dest = targetDir.appendingPathComponent(file.lastPathComponent)
        guard !FileManager.default.fileExists(atPath: dest.path) else { continue }
        do {
          try FileManager.default.copyItem(at: file, to: dest)
          NSLog("Installed font: %@", dest.path)
        } catch {
          NSLog("Failed to install font %@: %@", dest.path, error.localizedDescription)
        }
      }
    }
  }
}
