import AppKit

/// Hosts the tab manager's content view (the active tab's NSView) in the
/// workbench's center column, keeping it sized to the container.
final class ContentContainer: NSView {
  init(content: NSView) {
    super.init(frame: .zero)
    wantsLayer = true
    layer?.masksToBounds = true
    content.translatesAutoresizingMaskIntoConstraints = false
    addSubview(content)
    NSLayoutConstraint.activate([
      content.topAnchor.constraint(equalTo: topAnchor),
      content.leadingAnchor.constraint(equalTo: leadingAnchor),
      content.trailingAnchor.constraint(equalTo: trailingAnchor),
      content.bottomAnchor.constraint(equalTo: bottomAnchor),
    ])
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }
}
