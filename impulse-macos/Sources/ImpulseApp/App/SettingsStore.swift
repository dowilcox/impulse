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

  private init() {}

  /// Load from disk (app launch). Does not trigger a save.
  func load() {
    isLoading = true
    settings = Settings.load()
    isLoading = false
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
    settings.save()
  }

  /// Coalesce rapid changes (stepper clicks, typing) into one write.
  private func scheduleSave() {
    saveWorkItem?.cancel()
    let work = DispatchWorkItem { [weak self] in
      self?.saveWorkItem = nil
      self?.settings.save()
    }
    saveWorkItem = work
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.3, execute: work)
  }
}
