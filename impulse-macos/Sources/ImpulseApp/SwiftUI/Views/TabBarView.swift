import SwiftUI
import AppKit

/// Tab bar at the top of the detail area.
struct TabBarView: View {
    var windowModel: WindowModel
    @State private var hoveredIndex: Int? = nil

    var body: some View {
        tabStrip
            .frame(maxWidth: .infinity)
            .frame(height: 36)
            .background(.bar)
            .overlay(alignment: .bottom) { Divider() }
    }

    // MARK: - Tab Strip

    private var tabStrip: some View {
        ScrollViewReader { proxy in
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 1) {
                    ForEach(windowModel.tabDisplayInfos) { tab in
                        singleTab(tab)
                            .id(tab.id)
                    }
                }
                .padding(.horizontal, 4)
            }
            .onChange(of: windowModel.selectedTabIndex) { _, newValue in
                withAnimation { proxy.scrollTo(newValue, anchor: .center) }
            }
        }
    }

    // MARK: - Single Tab

    private func singleTab(_ tab: TabDisplayInfo) -> some View {
        let isSelected = tab.id == windowModel.selectedTabIndex
        let isHovered = hoveredIndex == tab.id

        // Use a plain ZStack — the tap gesture is on the whole area,
        // close button is an independent overlay.
        return ZStack(alignment: .trailing) {
            // Tab content — tapping anywhere selects the tab
            HStack(spacing: 5) {
                if let icon = tab.icon {
                    Image(nsImage: icon)
                        .resizable()
                        .interpolation(.high)
                        .frame(width: 14, height: 14)
                }

                Text(tab.title)
                    .font(.system(size: 12))
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            .frame(maxWidth: .infinity)
            .padding(.leading, 10)
            .padding(.trailing, tab.isPinned ? 10 : 26) // leave room for close

            // Close button — independent from the tab tap
            if !tab.isPinned && (isSelected || isHovered) {
                Button(action: { windowModel.onTabClosed?(tab.id) }) {
                    Image(systemName: "xmark")
                        .font(.system(size: 8, weight: .bold))
                        .foregroundStyle(.tertiary)
                        .frame(width: 16, height: 16)
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)
                .padding(.trailing, 6)
            }
        }
        .frame(minWidth: 120, maxWidth: 200, minHeight: 36, maxHeight: 36)
        .background(
            RoundedRectangle(cornerRadius: 5)
                .fill(isSelected ? Color.primary.opacity(0.08) : isHovered ? Color.primary.opacity(0.04) : .clear)
                .padding(.vertical, 3)
                .padding(.horizontal, 1)
        )
        .foregroundStyle(isSelected ? .primary : .secondary)
        .contentShape(Rectangle())
        .simultaneousGesture(TapGesture().onEnded {
            windowModel.onTabSelected?(tab.id)
        })
        .onHover { hovering in
            hoveredIndex = hovering ? tab.id : nil
        }
    }
}
