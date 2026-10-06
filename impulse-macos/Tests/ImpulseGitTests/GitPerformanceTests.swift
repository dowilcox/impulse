#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseGit

  /// Timing for the §11 budget "git snapshot refresh, 50k-file repo, warm
  /// < 150 ms". Runs only with IMPULSE_PERF_REPO set to a large repository:
  ///
  ///   IMPULSE_PERF_REPO=/path/to/repo impulse-macos/swiftw test --filter GitPerformanceTests
  struct GitPerformanceTests {
    @Test func warmSnapshotOfALargeRepository() throws {
      guard let root = ProcessInfo.processInfo.environment["IMPULSE_PERF_REPO"] else { return }
      _ = GitClient.snapshot(forPath: root)  // cold: index and caches
      var samples: [Double] = []
      for _ in 0..<5 {
        let start = DispatchTime.now().uptimeNanoseconds
        let snap = GitClient.snapshot(forPath: root)
        samples.append(Double(DispatchTime.now().uptimeNanoseconds - start) / 1_000_000)
        #expect(snap != nil)
      }
      let median = samples.sorted()[samples.count / 2]
      print("git snapshot warm: median \(String(format: "%.1f", median)) ms, samples \(samples.map { Int($0) })")
      #expect(median < 150, "budget: < 150 ms warm")
    }

    /// The data behind the review's first paint (budget: first paint
    /// < 300 ms for 200 files / 20k changed lines): the file list, then the
    /// first screenful of diffs.
    @Test func reviewDataForManyChangedFiles() throws {
      guard let root = ProcessInfo.processInfo.environment["IMPULSE_PERF_REPO"] else { return }
      let start = DispatchTime.now().uptimeNanoseconds
      let files = try GitClient.changedFiles(repoPath: root, scope: .unstaged)
      let listed = Double(DispatchTime.now().uptimeNanoseconds - start) / 1_000_000
      for file in files.prefix(5) {
        _ = try GitClient.fileDiff(repoPath: root, path: file.path, scope: .unstaged)
      }
      let total = Double(DispatchTime.now().uptimeNanoseconds - start) / 1_000_000
      let lines = files.compactMap(\.added).reduce(0, +)
      print("review data: \(files.count) files, \(lines) added lines; list \(Int(listed)) ms, list + 5 diffs \(Int(total)) ms")
      #expect(total < 300)
    }
  }
#endif
