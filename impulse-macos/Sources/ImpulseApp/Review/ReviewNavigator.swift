import ImpulseGit
import ImpulseKit
import SwiftUI

/// The review's file list: filter, how much is viewed, and the files by
/// folder. The file at the top of the diff list is highlighted.
@Observable
final class ReviewNavigatorModel {
  struct Item: Identifiable, Equatable {
    var id: String { path }
    let path: String
    let status: ChangeStatus
    let viewed: Bool
    let changedSinceViewed: Bool
    let commentCount: Int
    let added: Int?
    let removed: Int?
    let binary: Bool
  }

  var items: [Item] = []
  var filter = ""
  var current: String?
  var palette: ChromePalette
  /// Bump to put the cursor in the filter field (T or /).
  var filterFocusRequest = 0
  @ObservationIgnored var onSelect: ((String) -> Void)?
  @ObservationIgnored var onToggleViewed: ((String) -> Void)?

  init(palette: ChromePalette) {
    self.palette = palette
  }

  var visibleItems: [Item] {
    let query = filter.trimmingCharacters(in: .whitespaces).lowercased()
    return query.isEmpty ? items : items.filter { $0.path.lowercased().contains(query) }
  }

  var viewedFraction: Double {
    items.isEmpty ? 0 : Double(items.filter(\.viewed).count) / Double(items.count)
  }
}

struct ReviewNavigatorView: View {
  var model: ReviewNavigatorModel

  var body: some View {
    let chrome = model.palette
    VStack(spacing: 0) {
      ChromeTextField(
        placeholder: "Filter files (T)", text: Binding(get: { model.filter }, set: { model.filter = $0 }),
        icon: .search, focusRequest: model.filterFocusRequest,
        onSubmit: { if let first = model.visibleItems.first { model.onSelect?(first.path) } })
        .padding(8)
      GeometryReader { geo in
        ZStack(alignment: .leading) {
          Capsule().fill(chrome.hairline)
          Capsule().fill(chrome.gitAdded).frame(width: geo.size.width * model.viewedFraction)
        }
      }
      .frame(height: 3)
      .padding(.horizontal, 10)
      .padding(.bottom, 6)
      .animation(.easeOut(duration: 0.2), value: model.viewedFraction)
      .accessibilityLabel("Viewed \(Int(model.viewedFraction * 100)) percent")

      ScrollViewReader { proxy in
        ScrollView {
          LazyVStack(alignment: .leading, spacing: 0) {
            let items = model.visibleItems
            ForEach(Array(items.enumerated()), id: \.element.id) { index, item in
              let dir = Self.directory(item.path)
              if index == 0 || Self.directory(items[index - 1].path) != dir, !dir.isEmpty {
                Text(dir)
                  .font(ChromeFont.ui(10.5))
                  .foregroundStyle(chrome.textTertiary)
                  .lineLimit(1)
                  .truncationMode(.head)
                  .padding(.horizontal, 12)
                  .padding(.top, 6)
                  .padding(.bottom, 2)
                  .help(dir)
              }
              ReviewNavigatorRow(item: item, current: item.path == model.current, model: model)
                .id(item.path)
            }
          }
          .padding(.bottom, 12)
        }
        .onChange(of: model.current) { _, path in
          if let path { proxy.scrollTo(path) }
        }
      }
    }
    .frame(maxHeight: .infinity, alignment: .top)
    .background(chrome.panel)
    .environment(\.chrome, chrome)
  }

  static func directory(_ path: String) -> String {
    (path as NSString).deletingLastPathComponent
  }
}

private struct ReviewNavigatorRow: View {
  @Environment(\.chrome) private var chrome
  let item: ReviewNavigatorModel.Item
  let current: Bool
  var model: ReviewNavigatorModel
  @State private var hovering = false

  var body: some View {
    HStack(spacing: 6) {
      Button {
        model.onToggleViewed?(item.path)
      } label: {
        ReviewCheckbox(on: item.viewed)
      }
      .buttonStyle(.plain)
      .help(item.viewed ? "Viewed" : "Mark as viewed")
      ReviewStatusLetter(status: item.status)
      Text((item.path as NSString).lastPathComponent)
        .font(ChromeFont.ui(12))
        .foregroundStyle(item.viewed ? chrome.textTertiary : chrome.text)
        .lineLimit(1)
        .truncationMode(.middle)
        .frame(maxWidth: .infinity, alignment: .leading)
      if item.changedSinceViewed {
        Circle().fill(chrome.warning).frame(width: 6, height: 6).help("Changed since viewed")
      }
      if item.commentCount > 0 {
        HStack(spacing: 2) {
          Icon(.messageSquare, size: 10)
          Text("\(item.commentCount)").font(ChromeFont.mono(10))
        }
        .foregroundStyle(chrome.accent)
      }
      if !item.binary {
        ReviewStat(added: item.added, removed: item.removed)
      }
    }
    .padding(.leading, 8)
    .padding(.trailing, 6)
    .frame(height: 24)
    .background(
      RoundedRectangle(cornerRadius: 5, style: .continuous)
        .fill(current ? chrome.accentSoft : hovering ? chrome.hover : .clear)
    )
    .padding(.horizontal, 6)
    .contentShape(Rectangle())
    .onTapGesture { model.onSelect?(item.path) }
    .onHover { hovering = $0 }
    .help(item.path)
    .accessibilityElement(children: .combine)
    .accessibilityLabel("\(item.path)\(item.viewed ? ", viewed" : "")")
    .accessibilityAddTraits(.isButton)
  }
}
