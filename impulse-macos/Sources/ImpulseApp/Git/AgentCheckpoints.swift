import Foundation
import ImpulseGit
import ImpulseKit

/// Snapshots of the repository around each agent turn, so "what did the
/// agent just do?" is one review away (and undoable). A turn's start is
/// recorded when the agent starts working and its end when it stops, under
/// `refs/impulse/checkpoints/<terminal>/` — private refs a normal push never
/// sends, pruned like the safety oplog.
final class AgentCheckpoints {
  static let shared = AgentCheckpoints()

  struct Turn {
    let id: UUID
    let terminalID: UUID
    let agentName: String
    let repoRoot: String
    let start: SafetySnapshot
    var end: SafetySnapshot?

    /// The review scope for this turn (to the working tree while it runs).
    var scope: DiffScope { .snapshot(from: start.ref, to: end?.ref) }
  }

  private let queue = DispatchQueue(label: "impulse.checkpoints", qos: .utility)
  /// Turns per terminal, oldest first. Main thread only.
  private var turns: [UUID: [Turn]] = [:]
  /// The turn each terminal's agent is working on. Main thread only.
  private var openTurns: [UUID: UUID] = [:]
  /// Each turn's repository, once its start snapshot is taken. `queue` only.
  private var turnRoots: [UUID: String] = [:]
  private var prunedRoots: Set<String> = []

  /// The agent started working in `cwd`.
  func turnStarted(terminalID: UUID, agentName: String, cwd: String) {
    guard !cwd.isEmpty else { return }
    let id = UUID()
    openTurns[terminalID] = id
    queue.async { [weak self] in
      guard let self, let root = GitClient.repoRoot(forPath: cwd) else { return }
      let prefix = SafetySnapshots.checkpointPrefix + terminalID.uuidString + "/"
      guard
        case .success(let snapshot) = SafetySnapshots.create(
          reason: "turn start", root: root, prefix: prefix)
      else { return }
      self.turnRoots[id] = root
      self.pruneOnce(root: root)
      DispatchQueue.main.async {
        self.turns[terminalID, default: []].append(
          Turn(id: id, terminalID: terminalID, agentName: agentName, repoRoot: root, start: snapshot))
        NotificationCenter.default.post(name: .agentCheckpointsChanged, object: nil)
      }
    }
  }

  /// The agent stopped (finished, or waits for the user). The end snapshot
  /// queues behind the start's, so a turn that ends before its start
  /// snapshot is recorded still gets its end.
  func turnEnded(terminalID: UUID) {
    guard let id = openTurns.removeValue(forKey: terminalID) else { return }
    let prefix = SafetySnapshots.checkpointPrefix + terminalID.uuidString + "/"
    queue.async { [weak self] in
      // No root: the start snapshot failed or wasn't in a repository.
      guard let self, let root = self.turnRoots.removeValue(forKey: id),
        case .success(let snapshot) = SafetySnapshots.create(
          reason: "turn end", root: root, prefix: prefix)
      else { return }
      DispatchQueue.main.async {
        guard var list = self.turns[terminalID], let index = list.lastIndex(where: { $0.id == id }) else {
          return
        }
        list[index].end = snapshot
        self.turns[terminalID] = list
        NotificationCenter.default.post(name: .agentCheckpointsChanged, object: nil)
      }
    }
  }

  /// The most recent turn in a repository, by any agent.
  func lastTurn(inRepo root: String) -> Turn? {
    turns.values.compactMap { $0.last(where: { $0.repoRoot == root }) }
      .max { $0.start.date < $1.start.date }
  }

  func lastTurn(terminalID: UUID) -> Turn? {
    turns[terminalID]?.last
  }

  /// Every recorded turn in a terminal, oldest first.
  func turns(terminalID: UUID) -> [Turn] {
    turns[terminalID] ?? []
  }

  func turnCount(terminalID: UUID) -> Int {
    turns[terminalID]?.count ?? 0
  }

  private func pruneOnce(root: String) {
    guard !prunedRoots.contains(root) else { return }
    prunedRoots.insert(root)
    SafetySnapshots.prune(root: root, prefix: SafetySnapshots.checkpointPrefix)
  }
}

extension Notification.Name {
  static let agentCheckpointsChanged = Notification.Name("impulse.agentCheckpointsChanged")
}
