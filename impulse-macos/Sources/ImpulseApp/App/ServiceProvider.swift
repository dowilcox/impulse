import AppKit

/// macOS Services Impulse offers to other apps: Finder's "New Impulse
/// Workspace Here" on a folder (see NSServices in build.sh's Info.plist).
final class ServiceProvider: NSObject {
  @objc func openWorkspace(
    _ pasteboard: NSPasteboard, userData: String?, error: AutoreleasingUnsafeMutablePointer<NSString?>
  ) {
    let urls =
      pasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL] ?? []
    let folders = urls.filter { url in
      var isDirectory: ObjCBool = false
      return FileManager.default.fileExists(atPath: url.path, isDirectory: &isDirectory) && isDirectory.boolValue
    }
    guard !folders.isEmpty else {
      error.pointee = "Choose a folder to open as a workspace." as NSString
      return
    }
    NSApp.activate(ignoringOtherApps: true)
    for folder in folders {
      (NSApp.delegate as? AppDelegate)?.openWorkspaceFromService(folder.path)
    }
  }
}
