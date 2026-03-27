import SwiftUI
import AppKit

/// Tab bar styled like Finder: full-width pill tabs dividing the strip equally.
/// Hidden when there is only one tab.
struct TabBarView: View {
    var windowModel: WindowModel
    @State private var hoveredIndex: Int? = nil

    var body: some View {
        if windowModel.tabDisplayInfos.count > 1 {
            HStack(spacing: 2) {
                ForEach(windowModel.tabDisplayInfos) { tab in
                    singleTab(tab)
                        .id(tab.id)
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 5)
            .frame(maxWidth: .infinity)
            .overlay(alignment: .bottom) { Divider() }
        }
    }

    // MARK: - Single Tab

    private func singleTab(_ tab: TabDisplayInfo) -> some View {
        let isSelected = tab.id == windowModel.selectedTabIndex
        let isHovered = hoveredIndex == tab.id

        return HStack(spacing: 5) {
            if let icon = tab.icon {
                Image(nsImage: icon)
                    .resizable()
                    .interpolation(.high)
                    .frame(width: 14, height: 14)
            }

            Text(tab.title)
                .font(.system(size: 11.5))
                .lineLimit(1)
                .truncationMode(.middle)

            Spacer(minLength: 0)

            // Close button — only on hover
            if !tab.isPinned && isHovered {
                Button(action: { windowModel.onTabClosed?(tab.id) }) {
                    Image(systemName: "xmark")
                        .font(.system(size: 8, weight: .bold))
                        .foregroundStyle(.secondary)
                        .frame(width: 16, height: 16)
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 12)
        .frame(maxWidth: .infinity)
        .frame(height: 28)
        .background(
            RoundedRectangle(cornerRadius: 10)
                .fill(isSelected
                    ? Color.primary.opacity(0.12)
                    : isHovered ? Color.primary.opacity(0.05) : .clear)
        )
        .foregroundStyle(isSelected ? .primary : .secondary)
        .contentShape(RoundedRectangle(cornerRadius: 10))
        .simultaneousGesture(TapGesture().onEnded {
            windowModel.onTabSelected?(tab.id)
        })
        .onHover { hovering in
            hoveredIndex = hovering ? tab.id : nil
        }
    }
}
