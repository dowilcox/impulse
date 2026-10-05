import AppKit
import ImpulseKit
import SwiftUI

/// The right dock: the focused file's symbols, nested, with the one the
/// cursor is in highlighted. Click to jump.
struct OutlineView: View {
  var model: WindowModel

  var body: some View {
    let chrome = model.palette
    VStack(spacing: 0) {
      HStack(spacing: 6) {
        Text("OUTLINE").font(ChromeFont.ui(10.5, weight: .semibold)).tracking(0.6)
          .foregroundStyle(chrome.textTertiary)
        if let file = model.outlineFile {
          Text((file as NSString).lastPathComponent).font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textSecondary)
            .lineLimit(1).truncationMode(.middle)
        }
        Spacer()
        ChromeIconButton(icon: .refreshCw, help: "Refresh", size: 20, iconSize: 11) { model.onRefreshOutline?() }
      }
      .padding(.horizontal, 12)
      .frame(height: 32)
      Rectangle().fill(chrome.hairline).frame(height: 1)
      content
    }
    .background(chrome.panel)
    .environment(\.chrome, chrome)
  }

  @ViewBuilder private var content: some View {
    let chrome = model.palette
    switch model.outlineState {
    case .idle, .noEditor:
      placeholder("Open a file to see its outline.")
    case .loading:
      ProgressRing(progress: nil, color: chrome.textTertiary, size: 14, lineWidth: 1.5)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    case .noServer:
      placeholder("No language server for this file.")
    case .ready:
      if model.outlineSymbols.isEmpty {
        placeholder("No symbols in this file.")
      } else {
        let current = currentIndex
        ScrollViewReader { proxy in
          ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
              ForEach(Array(model.outlineSymbols.enumerated()), id: \.offset) { index, symbol in
                row(symbol, selected: index == current).id(index)
              }
            }
            .padding(.vertical, 4)
          }
          .onChange(of: current) { _, index in
            if let index { withAnimation(.easeOut(duration: 0.15)) { proxy.scrollTo(index, anchor: .center) } }
          }
        }
      }
    }
  }

  /// The last symbol starting at or above the cursor (deepest wins on ties).
  private var currentIndex: Int? {
    guard let line = model.cursorLine else { return nil }
    return model.outlineSymbols.lastIndex { $0.line <= line }
  }

  private func row(_ symbol: OutlineSymbol, selected: Bool) -> some View {
    let chrome = model.palette
    return Button {
      model.onOutlineSelect?(symbol)
    } label: {
      HStack(spacing: 6) {
        Icon(icon(for: symbol.kind), size: 11).foregroundStyle(color(for: symbol.kind))
        Text(symbol.name).font(ChromeFont.ui(12)).foregroundStyle(chrome.text).lineLimit(1)
        if let detail = symbol.detail {
          Text(detail).font(ChromeFont.ui(11)).foregroundStyle(chrome.textTertiary).lineLimit(1)
            .truncationMode(.tail)
        }
        Spacer(minLength: 4)
      }
      .padding(.leading, 12 + CGFloat(min(symbol.depth, 8)) * 14)
      .padding(.trailing, 10)
      .frame(height: 24)
      .background(selected ? chrome.selection : .clear)
      .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
    .help("\(symbol.kindName) · line \(symbol.line)")
  }

  private func placeholder(_ text: String) -> some View {
    Text(text).font(ChromeFont.ui(12)).foregroundStyle(model.palette.textTertiary)
      .frame(maxWidth: .infinity, maxHeight: .infinity)
      .padding()
  }

  private func icon(for kind: Int) -> LucideIcon {
    switch kind {
    case 5, 10, 11, 23, 26: return .layers
    case 6, 9, 12, 25: return .code
    case 2, 3, 4: return .package
    default: return .tag
    }
  }

  private func color(for kind: Int) -> Color {
    let chrome = model.palette
    switch kind {
    case 5, 10, 11, 23, 26: return chrome.warning
    case 6, 9, 12, 25: return chrome.accent
    default: return chrome.textSecondary
    }
  }
}
