import Foundation
import Observation

/// Re-runs `apply` on the main queue whenever any `@Observable` property it
/// reads changes — the AppKit counterpart of a SwiftUI view body. Tracking
/// stops when `owner` is deallocated or `cancel()` is called.
///
///     let token = ObservationLoop(owner: self) { [weak self] in
///       self?.layoutDocks(visible: model.leftDockVisible)
///     }
final class ObservationLoop {
  private weak var owner: AnyObject?
  private let apply: () -> Void
  private var cancelled = false

  @discardableResult
  init(owner: AnyObject, _ apply: @escaping () -> Void) {
    self.owner = owner
    self.apply = apply
    run()
  }

  func cancel() { cancelled = true }

  private func run() {
    guard !cancelled, owner != nil else { return }
    withObservationTracking {
      apply()
    } onChange: { [weak self] in
      // onChange fires before the new value is set; apply on the next turn.
      DispatchQueue.main.async { self?.run() }
    }
  }
}
