import Foundation

/// Polls terminals whose views are off screen — background tabs, other
/// workspaces, restored tabs not yet shown — so their events (titles, cwd,
/// finished commands, bells, progress, exits) keep arriving while hidden.
/// On-screen terminals poll themselves at display rate; these share one
/// low-rate timer that speeds up while any of them is busy.
final class TerminalSessionHub {
  static let shared = TerminalSessionHub()

  private let renderers = NSHashTable<TerminalRenderer>.weakObjects()
  private var timer: DispatchSourceTimer?
  private let busyInterval: TimeInterval = 0.1
  private let idleInterval: TimeInterval = 0.25

  func add(_ renderer: TerminalRenderer) {
    renderers.add(renderer)
    scheduleIfNeeded(after: 0)
  }

  func remove(_ renderer: TerminalRenderer) {
    renderers.remove(renderer)
  }

  var count: Int { renderers.allObjects.count }

  private func scheduleIfNeeded(after interval: TimeInterval) {
    guard timer == nil, renderers.allObjects.isEmpty == false else { return }
    let timer = DispatchSource.makeTimerSource(queue: .main)
    timer.schedule(deadline: .now() + interval)
    timer.setEventHandler { [weak self] in
      guard let self else { return }
      self.timer = nil
      let busy = self.pollAll()
      self.scheduleIfNeeded(after: busy ? self.busyInterval : self.idleInterval)
    }
    self.timer = timer
    timer.resume()
  }

  private func pollAll() -> Bool {
    var busy = false
    for renderer in renderers.allObjects {
      guard renderer.window == nil else {
        renderers.remove(renderer)
        continue
      }
      switch renderer.pollInBackground() {
      case nil: renderers.remove(renderer)
      case true?: busy = true
      case false?: break
      }
    }
    return busy
  }
}
