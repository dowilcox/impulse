#if canImport(Testing)
  import Foundation
  @testable import ImpulseApp
  @testable import ImpulseGit
  import ImpulseKit
  import Testing

  struct ReviewRowsTests {
    private func line(_ kind: DiffLineKind, _ old: UInt32?, _ new: UInt32?, _ text: String = "x") -> DiffLine {
      DiffLine(kind: kind, oldLineno: old, newLineno: new, content: text, spans: [])
    }

    private func file(_ path: String, hunks: [[DiffLine]]) -> ReviewFile {
      let file = ReviewFile(change: FileChange(path: path, status: .modified, added: 1, removed: 1))
      file.diff = FileDiff(
        path: path, oldPath: nil, status: .modified, language: "swift", isBinary: false, tooLarge: false,
        truncated: false, added: 1, removed: 1,
        hunks: hunks.map { DiffHunk(oldStart: 1, oldLines: 1, newStart: 1, newLines: 1, header: "@@", lines: $0) },
        hunkIds: hunks.indices.map { "h\($0)" }, oldMissingNewlineAtEnd: false, newMissingNewlineAtEnd: false)
      return file
    }

    @Test func unifiedRowsWithCommentsAndComposer() {
      let f = file(
        "a.swift",
        hunks: [[line(.context, 1, 1), line(.removed, 2, nil), line(.added, nil, 2), line(.context, 3, 3)]])
      f.comments = [
        ReviewComment(path: "a.swift", side: .new, line: 2, endLine: 2, snippet: "x", text: "on the new line"),
        ReviewComment(path: "a.swift", side: .old, line: 2, endLine: 2, snippet: "x", text: "on the old line"),
      ]
      f.composer = .init(hunk: 0, lineIndex: 3, side: .new, line: 3, endLine: 3, snippet: "x")
      let rows = ReviewRowBuilder.rows([f], layout: .unified)
      #expect(
        rows == [
          .fileHeader("a.swift"), .hunkHeader("a.swift", hunk: 0),
          .line("a.swift", hunk: 0, line: 0),
          .line("a.swift", hunk: 0, line: 1), .comment("a.swift", id: f.comments[1].id),
          .line("a.swift", hunk: 0, line: 2), .comment("a.swift", id: f.comments[0].id),
          .line("a.swift", hunk: 0, line: 3), .composer("a.swift"),
          .fileEnd("a.swift"),
        ])
    }

    @Test func splitPairsRemovedWithAdded() {
      let lines = [
        line(.context, 1, 1), line(.removed, 2, nil), line(.removed, 3, nil), line(.added, nil, 2),
        line(.context, 4, 3), line(.added, nil, 4),
      ]
      let pairs = ReviewRowBuilder.splitPairs(lines).map { [$0.old ?? -1, $0.new ?? -1] }
      #expect(pairs == [[0, 0], [1, 3], [2, -1], [4, 4], [-1, 5]])
    }

    @Test func collapsedLoadingAndOutdated() {
      let collapsed = file("b.txt", hunks: [[line(.added, nil, 1)]])
      collapsed.expanded = false
      let loading = ReviewFile(change: FileChange(path: "c.txt", status: .added))
      let old = file("d.txt", hunks: [[line(.added, nil, 1)]])
      let comment = ReviewComment(path: "d.txt", side: .new, line: 9, endLine: 9, snippet: "gone", text: "?")
      old.comments = [comment]
      old.outdated = [comment.id]
      let rows = ReviewRowBuilder.rows([collapsed, loading, old], layout: .unified)
      #expect(
        rows == [
          .fileHeader("b.txt"), .fileEnd("b.txt"),
          .fileHeader("c.txt"), .notice("c.txt", .loading), .fileEnd("c.txt"),
          .fileHeader("d.txt"), .outdatedTitle("d.txt"), .comment("d.txt", id: comment.id),
          .hunkHeader("d.txt", hunk: 0), .line("d.txt", hunk: 0, line: 0), .fileEnd("d.txt"),
        ])
    }

    @Test func eachFilesRowsMakeUpTheList() {
      let a = file("a.swift", hunks: [[line(.context, 1, 1), line(.added, nil, 2)]])
      let b = file("b.swift", hunks: [[line(.removed, 1, nil)], [line(.added, nil, 5)]])
      b.expanded = false
      let c = file("c.swift", hunks: [[line(.context, 1, 1), line(.removed, 2, nil), line(.added, nil, 2)]])
      for layout in [ReviewLayout.unified, .split] {
        let each = [a, b, c].flatMap { ReviewRowBuilder.rows(for: $0, layout: layout) }
        #expect(ReviewRowBuilder.rows([a, b, c], layout: layout) == each)
      }
    }

    /// A file's cached rows hold until something they show changes.
    @Test func rowKeyChangesWithWhatTheRowsShow() {
      let f = file("a.swift", hunks: [[line(.context, 1, 1), line(.added, nil, 2)]])
      let key = ReviewRowKey(f, layout: .unified)
      f.viewed = true
      f.selection = .init(hunk: 0, lines: [1], anchor: 1)
      f.syntax = [[[], []]]
      #expect(ReviewRowKey(f, layout: .unified) == key, "drawn in place, not rebuilt")
      #expect(ReviewRowKey(f, layout: .split) != key)

      let diff = f.diff
      f.diff = diff
      #expect(ReviewRowKey(f, layout: .unified) != key, "a replaced diff")
      let replaced = ReviewRowKey(f, layout: .unified)
      f.comments = [ReviewComment(path: "a.swift", side: .new, line: 2, endLine: 2, snippet: "x", text: "?")]
      #expect(ReviewRowKey(f, layout: .unified) != replaced)
      let commented = ReviewRowKey(f, layout: .unified)
      f.editingComment = f.comments[0].id
      #expect(ReviewRowKey(f, layout: .unified) != commented)
      f.editingComment = nil
      f.composer = .init(hunk: 0, lineIndex: 1, side: .new, line: 2, endLine: 2, snippet: "x")
      #expect(ReviewRowKey(f, layout: .unified) != commented)
      f.composer = nil
      f.expanded = false
      #expect(ReviewRowKey(f, layout: .unified) != commented)

      // A new file's first diff never matches another file's old key.
      let other = ReviewFile(change: FileChange(path: "a.swift", status: .modified))
      other.diff = diff
      #expect(ReviewRowKey(other, layout: .unified) != replaced)
    }

    /// The table's row count follows the rows through first, partial and
    /// emptying reloads, without ever being laid out.
    @MainActor @Test func tableRowCountFollowsReloads() {
      let context = ReviewDiffContext(
        theme: ThemeManager.theme(forName: "nord"), metrics: ReviewMetrics(fontFamily: "Menlo"))
      let list = ReviewDiffController(context: context)
      let a = file("a.swift", hunks: [[line(.context, 1, 1), line(.added, nil, 2)]])
      let b = file("b.swift", hunks: [[line(.removed, 1, nil)]])
      context.files = ["a.swift": a, "b.swift": b]
      list.reload([a, b])
      #expect(list.rows.count == 10)
      #expect(list.tableView.numberOfRows == list.rows.count)
      b.expanded = false
      list.reload([a, b])
      #expect(list.tableView.numberOfRows == list.rows.count)
      list.reload([b])
      #expect(list.tableView.numberOfRows == list.rows.count)
      list.reload([])
      #expect(list.tableView.numberOfRows == 0)
    }

    @Test func wrappedLineCounts() {
      let metrics = ReviewMetrics(fontFamily: "Menlo", size: 12)
      let width = metrics.charWidth * 10
      #expect(metrics.wrappedLineCount("", width: width) == 1)
      #expect(metrics.wrappedLineCount(String(repeating: "a", count: 10), width: width) == 1)
      #expect(metrics.wrappedLineCount(String(repeating: "a", count: 11), width: width) == 2)
      #expect(ReviewMetrics.displayColumns("\tab", tabWidth: 4) == 6)
      #expect(ReviewMetrics.displayColumns("e\u{301}日本", tabWidth: 4) == 5)
    }
  }
#endif
