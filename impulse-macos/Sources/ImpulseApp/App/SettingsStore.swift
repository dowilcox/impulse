import AppKit
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

  /// Current settings. Assigning a different value schedules a debounced
  /// save.
  var settings: Settings = .default {
    didSet {
      guard !isLoading, settings != oldValue else { return }
      scheduleSave()
    }
  }

  @ObservationIgnored private var isLoading = false
  @ObservationIgnored private var saveWorkItem: DispatchWorkItem?
  @ObservationIgnored private var directoryWatch: DispatchSourceFileSystemObject?
  @ObservationIgnored private var fileWatches: [DispatchSourceFileSystemObject] = []
  @ObservationIgnored private var reloadWork: DispatchWorkItem?
  /// The settings as settings.json held them when Impulse last read or
  /// wrote it (nil while it doesn't parse): what this session's changes are
  /// told apart by when the file is changed elsewhere.
  @ObservationIgnored private var onDisk: Settings?

  private init() {}

  /// Load from disk (app launch). Does not trigger a save.
  func load() {
    isLoading = true
    settings = Settings.load()
    isLoading = false
    onDisk = Settings.loadWarning == nil ? settings : nil
  }

  /// Follow edits to settings.json made by hand (in Impulse's editor or
  /// another app): without this they'd apply only after a restart, and every
  /// in-app change before then would be refused (the file no longer matches
  /// what was loaded).
  func watchFile() {
    guard AppState.persistenceEnabled, directoryWatch == nil else { return }
    // The folder sees the file replaced (saves write a new inode) or
    // created; the file itself sees edits made in place.
    directoryWatch = Self.watch(Settings.settingsPath().deletingLastPathComponent().path, events: [.write]) {
      [weak self] in self?.scheduleReload()
    }
    watchTargetFile()
  }

  /// Watch the file settings.json resolves to (through a symlink, a dotfile
  /// manager's copy), and its folder when that's somewhere else. Opened
  /// again after every change: a replaced file leaves the old descriptor
  /// watching nothing.
  private func watchTargetFile() {
    fileWatches.forEach { $0.cancel() }
    fileWatches = []
    let settingsPath = Settings.settingsPath()
    let target = TextFile.target(of: settingsPath.path)
    if let watch = Self.watch(target, events: [.write, .extend, .delete, .rename], handler: { [weak self] in
      self?.scheduleReload()
    }) {
      fileWatches.append(watch)
    }
    let targetFolder = (target as NSString).deletingLastPathComponent
    if targetFolder != TextFile.target(of: settingsPath.deletingLastPathComponent().path),
      let watch = Self.watch(targetFolder, events: [.write], handler: { [weak self] in self?.scheduleReload() })
    {
      fileWatches.append(watch)
    }
  }

  private static func watch(
    _ path: String, events: DispatchSource.FileSystemEvent, handler: @escaping () -> Void
  ) -> DispatchSourceFileSystemObject? {
    let fd = open(path, O_EVTONLY)
    guard fd >= 0 else { return nil }
    let source = DispatchSource.makeFileSystemObjectSource(fileDescriptor: fd, eventMask: events, queue: .main)
    source.setEventHandler(handler: handler)
    source.setCancelHandler { close(fd) }
    source.resume()
    return source
  }

  private func scheduleReload() {
    reloadWork?.cancel()
    let work = DispatchWorkItem { [weak self] in
      self?.watchTargetFile()
      self?.reloadIfChangedOnDisk()
    }
    reloadWork = work
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.3, execute: work)
  }

  /// Read settings.json again if something other than Impulse changed it.
  /// A file that doesn't parse leaves the current settings in place (saves
  /// stay paused and the banner says why) until it's fixed.
  func reloadIfChangedOnDisk() {
    guard Settings.fileChangedOnDisk() else { return }
    if mergeFromDisk(announce: true) {
      scheduleSave()
    }
  }

  /// Take settings.json as it is now, with the changes made in Impulse since
  /// it was last read or written on top (a change made here just before
  /// the file was edited elsewhere isn't lost). True when that left
  /// something to write.
  private func mergeFromDisk(announce: Bool) -> Bool {
    saveWorkItem?.cancel()
    saveWorkItem = nil
    let loaded = Settings.load(reloading: true)
    var unsaved = false
    if Settings.loadWarning == nil {
      let merged = onDisk.map { Settings.merged(settings, onto: loaded, since: $0) } ?? loaded
      onDisk = loaded
      isLoading = true
      settings = merged
      isLoading = false
      unsaved = merged != loaded
      if unsaved, announce { announceMerge() }
    }
    NotificationCenter.default.post(name: .impulseSettingsDidChange, object: settings)
    return unsaved
  }

  private func announceMerge() {
    let controller =
      NSApp.keyWindow?.windowController as? MainWindowController
      ?? NSApp.windows.lazy.compactMap { $0.windowController as? MainWindowController }.first
    let path = Settings.filePath.path
    controller?.toasts.show(
      Toast(
        kind: .info, message: "settings.json was changed outside Impulse",
        detail: "Your changes here were kept on top of it.",
        actionTitle: "Open File", action: { [weak controller] in controller?.openFile(path: path) },
        lifetime: 10))
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
    persist(synchronously: true)
  }

  /// Coalesce rapid changes (stepper clicks, typing) into one write.
  private func scheduleSave() {
    guard AppState.persistenceEnabled else { return }
    saveWorkItem?.cancel()
    let work = DispatchWorkItem { [weak self] in
      self?.saveWorkItem = nil
      self?.persist()
    }
    saveWorkItem = work
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.3, execute: work)
  }

  /// Save, and when settings.json was changed elsewhere first, merge with
  /// that version and save the result instead of dropping this session's
  /// changes.
  private func persist(synchronously: Bool = false, merged: Bool = false) {
    guard AppState.persistenceEnabled else { return }
    let saving = settings
    saving.save(synchronously: synchronously) { [weak self] outcome in
      guard let self else { return }
      switch outcome {
      case .saved:
        self.onDisk = saving
      case .changedOnDisk:
        // Once: a file that keeps changing is left to the watcher.
        guard !merged else { return }
        // At quit there's no window left to say so in.
        if self.mergeFromDisk(announce: !synchronously) {
          self.persist(synchronously: synchronously, merged: true)
        }
      case .failed:
        break
      }
    }
  }
}
