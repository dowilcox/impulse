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

  // MARK: Session

  /// A turn as the session file keeps it: the refs, not the snapshots, so
  /// a restore can check they're still there.
  struct SavedTurn: Codable, Equatable {
    var agent: String
    var repo: String
    var start: String
    var end: String?
  }

  /// At most this many of a terminal's latest turns are saved.
  static let savedTurnLimit = 50

  /// A terminal's turns for the session file (oldest first).
  func savedTurns(terminalID: UUID) -> [SavedTurn] {
    turns(terminalID: terminalID).suffix(Self.savedTurnLimit).map {
      SavedTurn(agent: $0.agentName, repo: $0.repoRoot, start: $0.start.ref, end: $0.end?.ref)
    }
  }

  /// Give a restored terminal its turns back. Turns whose checkpoints were
  /// pruned, are old enough to be pruned next, or have no end (saved
  /// mid-turn), and turns whose repository is gone, are dropped.
  func restore(_ saved: [SavedTurn], terminalID: UUID) {
    guard !saved.isEmpty else { return }
    queue.async { [weak self] in
      var byRef: [String: SafetySnapshot] = [:]
      for repo in Set(saved.map(\.repo)) where FileManager.default.fileExists(atPath: repo) {
        for snapshot in SafetySnapshots.list(root: repo, prefix: SafetySnapshots.checkpointPrefix) {
          byRef[repo + "\n" + snapshot.ref] = snapshot
        }
      }
      let cutoff = Date().addingTimeInterval(-SafetySnapshots.defaultMaxAge)
      let restored: [Turn] = saved.compactMap { turn in
        // Without its end (pruned, or never recorded) the diff would run to
        // the working tree, which isn't that turn.
        guard let start = byRef[turn.repo + "\n" + turn.start], start.date >= cutoff,
          let end = turn.end.flatMap({ byRef[turn.repo + "\n" + $0] })
        else { return nil }
        return Turn(
          id: UUID(), terminalID: terminalID, agentName: turn.agent, repoRoot: turn.repo, start: start, end: end)
      }
      guard !restored.isEmpty else { return }
      DispatchQueue.main.async {
        // Ahead of any turn the terminal recorded since it was restored.
        self?.turns[terminalID, default: []].insert(contentsOf: restored, at: 0)
        NotificationCenter.default.post(name: .agentCheckpointsChanged, object: nil)
      }
    }
  }

  /// `queue` only.
  private func pruneOnce(root: String) {
    guard !prunedRoots.contains(root) else { return }
    prunedRoots.insert(root)
    pruneNow(root: root)
  }

  /// Prune the checkpoints of `root`'s repository (call from any thread).
  func prune(root: String) {
    queue.async { [weak self] in self?.pruneNow(root: root) }
  }

  /// `queue` only. Turns whose snapshots went are forgotten, so a review
  /// never asks for a deleted ref. By ref name alone: a repository's
  /// worktrees share its refs, and the names are unique.
  private func pruneNow(root: String) {
    let deleted = Set(SafetySnapshots.prune(root: root, prefix: SafetySnapshots.checkpointPrefix))
    guard !deleted.isEmpty else { return }
    DispatchQueue.main.async { [weak self] in
      guard let self else { return }
      var changed = false
      for (terminalID, list) in self.turns {
        let kept = list.filter { turn in
          !deleted.contains(turn.start.ref) && !(turn.end.map { deleted.contains($0.ref) } ?? false)
        }
        if kept.count != list.count {
          self.turns[terminalID] = kept
          changed = true
        }
      }
      if changed { NotificationCenter.default.post(name: .agentCheckpointsChanged, object: nil) }
    }
  }
}

extension Notification.Name {
  static let agentCheckpointsChanged = Notification.Name("impulse.agentCheckpointsChanged")
}
