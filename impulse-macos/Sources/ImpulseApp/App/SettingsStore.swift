import Foundation
import Observation

/// The single in-memory copy of the user's settings.
///
/// Every window, tab manager and the settings UI read and write through
/// `SettingsStore.shared.settings` instead of keeping their own copies (which
/// had to be hand-synced and drifted). Writes schedule a debounced save to
/// `settings.json`; `commit()` additionally broadcasts
/// `.impulseSettingsDidChange` so live UI re-applies the new values.
@Observable
final class SettingsStore {
  static let shared = SettingsStore()

  /// Current settings. Assigning schedules a debounced save.
  var settings: Settings = .default {
    didSet {
      guard !isLoading else { return }
      scheduleSave()
    }
  }

  @ObservationIgnored private var isLoading = false
  @ObservationIgnored private var saveWorkItem: DispatchWorkItem?
  @ObservationIgnored private var directoryWatch: DispatchSourceFileSystemObject?
  @ObservationIgnored private var reloadWork: DispatchWorkItem?

  private init() {}

  /// Load from disk (app launch). Does not trigger a save.
  func load() {
    isLoading = true
    settings = Settings.load()
    isLoading = false
  }

  /// Follow edits to settings.json made by hand (in Impulse's editor or
  /// another app): without this they'd apply only after a restart, and every
  /// in-app change before then would be refused (the file no longer matches
  /// what was loaded).
  func watchFile() {
    guard AppState.persistenceEnabled, directoryWatch == nil else { return }
    // The folder, not the file: saves replace the file (a new inode).
    let fd = open(Settings.settingsPath().deletingLastPathComponent().path, O_EVTONLY)
    guard fd >= 0 else { return }
    let source = DispatchSource.makeFileSystemObjectSource(fileDescriptor: fd, eventMask: [.write], queue: .main)
    source.setEventHandler { [weak self] in self?.scheduleReload() }
    source.setCancelHandler { close(fd) }
    source.resume()
    directoryWatch = source
  }

  private func scheduleReload() {
    reloadWork?.cancel()
    let work = DispatchWorkItem { [weak self] in self?.reloadIfChangedOnDisk() }
    reloadWork = work
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.3, execute: work)
  }

  /// Read settings.json again if something other than Impulse changed it.
  /// A file that doesn't parse leaves the current settings in place (saves
  /// stay paused and the banner says why) until it's fixed.
  func reloadIfChangedOnDisk() {
    guard Settings.fileChangedOnDisk() else { return }
    saveWorkItem?.cancel()
    saveWorkItem = nil
    let loaded = Settings.load(backupInvalid: false)
    if Settings.loadWarning == nil {
      isLoading = true
      settings = loaded
      isLoading = false
    }
    NotificationCenter.default.post(name: .impulseSettingsDidChange, object: settings)
  }

  /// Apply a change, then broadcast it so every window re-applies settings.
  func update(_ change: (inout Settings) -> Void) {
    change(&settings)
    commit()
  }

  /// Broadcast the current settings to observers of `.impulseSettingsDidChange`
  /// and schedule a save.
  func commit() {
    NotificationCenter.default.post(name: .impulseSettingsDidChange, object: settings)
    scheduleSave()
  }

  /// Write to disk immediately (app termination).
  func saveNow() {
    saveWorkItem?.cancel()
    saveWorkItem = nil
    guard AppState.persistenceEnabled else { return }
    settings.save(synchronously: true)
  }

  /// Coalesce rapid changes (stepper clicks, typing) into one write.
  private func scheduleSave() {
    guard AppState.persistenceEnabled else { return }
    saveWorkItem?.cancel()
    let work = DispatchWorkItem { [weak self] in
      self?.saveWorkItem = nil
      self?.settings.save()
    }
    saveWorkItem = work
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.3, execute: work)
  }
}
