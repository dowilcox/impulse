import SwiftUI
import AppKit

/// Tab bar styled like Finder: full-width pill tabs dividing the strip equally.
/// Hidden when there is only one tab. Supports smooth drag-reorder.
struct TabBarView: View {
    var windowModel: WindowModel
    @State private var hoveredIndex: Int? = nil
    @State private var draggedTabId: Int? = nil
    @State private var dragOffset: CGFloat = 0
    @State private var tabFrames: [Int: CGRect] = [:]

    var body: some View {
        if windowModel.tabDisplayInfos.count > 1 {
            HStack(spacing: 2) {
                ForEach(windowModel.tabDisplayInfos) { tab in
                    singleTab(tab)
                        .id(tab.id)
                        .background(
                            GeometryReader { geo in
                                Color.clear.preference(
                                    key: TabFrameKey.self,
                                    value: [tab.id: geo.frame(in: .named("tabBar"))]
                                )
                            }
                        )
                        .offset(x: tabOffset(for: tab))
                        .animation(
                            draggedTabId == tab.id ? nil : .interactiveSpring(response: 0.25, dampingFraction: 0.8),
                            value: tabOffset(for: tab)
                        )
                        .zIndex(draggedTabId == tab.id ? 1 : 0)
                        .simultaneousGesture(
                            DragGesture(minimumDistance: 5, coordinateSpace: .named("tabBar"))
                                .onChanged { value in
                                    if draggedTabId == nil {
                                        draggedTabId = tab.id
                                        // Select the tab on drag start, like Finder.
                                        windowModel.onTabSelected?(tab.index)
                                    }
                                    dragOffset = value.translation.width
                                }
                                .onEnded { _ in
                                    commitDrag()
                                }
                        )
                }
            }
            .coordinateSpace(name: "tabBar")
            .onPreferenceChange(TabFrameKey.self) { tabFrames = $0 }
            .padding(.horizontal, 8)
            .padding(.vertical, 5)
            .frame(maxWidth: .infinity)
            .overlay(alignment: .bottom) { Divider() }
        }
    }

    // MARK: - Drag Logic

    /// Returns the visual x-offset for a tab during drag.
    /// The dragged tab follows the cursor; other tabs slide to make room.
    private func tabOffset(for tab: TabDisplayInfo) -> CGFloat {
        // Dragged tab follows cursor directly.
        if tab.id == draggedTabId {
            return dragOffset
        }

        guard let draggedId = draggedTabId,
              let draggedFrame = tabFrames[draggedId],
              let tabFrame = tabFrames[tab.id] else { return 0 }

        let tabs = windowModel.tabDisplayInfos
        guard let draggedIdx = tabs.firstIndex(where: { $0.id == draggedId }),
              let tabIdx = tabs.firstIndex(where: { $0.id == tab.id }) else { return 0 }

        let draggedCenter = draggedFrame.midX + dragOffset
        let shiftAmount = draggedFrame.width + 2 // width + spacing

        // Tab is to the right of the dragged tab and dragged has crossed its midpoint.
        if draggedIdx < tabIdx && draggedCenter > tabFrame.midX {
            return -shiftAmount
        }
        // Tab is to the left of the dragged tab and dragged has crossed its midpoint.
        if draggedIdx > tabIdx && draggedCenter < tabFrame.midX {
            return shiftAmount
        }

        return 0
    }

    /// On drag end, calculate the final target index and commit the move.
    private func commitDrag() {
        guard let draggedId = draggedTabId else { return }

        let tabs = windowModel.tabDisplayInfos
        guard let sourceIdx = tabs.firstIndex(where: { $0.id == draggedId }),
              let draggedFrame = tabFrames[draggedId] else {
            draggedTabId = nil
            dragOffset = 0
            return
        }

        let draggedCenter = draggedFrame.midX + dragOffset

        // Walk tabs to find the rightmost tab we've crossed to the right,
        // or the leftmost tab we've crossed to the left.
        var targetIdx = sourceIdx
        for (idx, tab) in tabs.enumerated() {
            if tab.id == draggedId { continue }
            guard let frame = tabFrames[tab.id] else { continue }
            if sourceIdx < idx && draggedCenter > frame.midX {
                targetIdx = idx
            } else if sourceIdx > idx && draggedCenter < frame.midX && idx < targetIdx {
                targetIdx = idx
            }
        }

        // Reset drag state first so the offset doesn't compound with
        // the new layout position after reorder.
        draggedTabId = nil
        dragOffset = 0

        if sourceIdx != targetIdx {
            windowModel.onTabMoved?(sourceIdx, targetIdx)
        }
    }

    // MARK: - Single Tab

    private func singleTab(_ tab: TabDisplayInfo) -> some View {
        let isSelected = tab.index == windowModel.selectedTabIndex
        let isHovered = hoveredIndex == tab.id
        let isDragging = draggedTabId == tab.id

        return HStack(spacing: 5) {
            if let icon = tab.icon {
                Image(nsImage: icon)
                    .resizable()
                    .interpolation(.high)
                    .frame(width: 14, height: 14)
                    .accessibilityHidden(true)
            }

            Text(tab.title)
                .font(.system(size: 11.5))
                .lineLimit(1)
                .truncationMode(.middle)

            Spacer(minLength: 0)

            if tab.needsAttention && !isSelected {
                Circle()
                    .fill(Color.accentColor)
                    .frame(width: 6, height: 6)
                    .accessibilityHidden(true)
            }

            if !tab.isPinned && isHovered {
                Button(action: { windowModel.onTabClosed?(tab.index) }) {
                    Image(systemName: "xmark")
                        .font(.system(size: 8, weight: .bold))
                        .foregroundStyle(.secondary)
                        .frame(width: 16, height: 16)
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Close \(tab.isTerminal ? "Terminal" : "Editor") Tab \(tab.title)")
            }
        }
        .padding(.horizontal, 12)
        .frame(maxWidth: .infinity)
        .frame(height: 30)
        .background(
            Group {
                if isSelected || isDragging {
                    Capsule().fill(.thickMaterial)
                } else if isHovered {
                    Capsule().fill(Color.white.opacity(0.04))
                }
            }
        )
        .overlay(
            Capsule()
                .strokeBorder(
                    isSelected || isDragging
                        ? Color.white.opacity(0.2)
                        : isHovered ? Color.white.opacity(0.08) : .clear,
                    lineWidth: 1
                )
        )
        .foregroundStyle(isSelected || isDragging ? .primary : .secondary)
        .contentShape(Capsule())
        .simultaneousGesture(TapGesture().onEnded {
            windowModel.onTabSelected?(tab.index)
        })
        .onHover { hovering in
            hoveredIndex = hovering ? tab.id : nil
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(tab.isTerminal ? "Terminal" : "Editor"): \(tab.title)")
        .accessibilityAddTraits(isSelected ? [.isSelected, .isButton] : [.isButton])
    }
}

// MARK: - Preference Key for Tab Frames

private struct TabFrameKey: PreferenceKey {
    static var defaultValue: [Int: CGRect] = [:]
    static func reduce(value: inout [Int: CGRect], nextValue: () -> [Int: CGRect]) {
        value.merge(nextValue()) { $1 }
    }
}
