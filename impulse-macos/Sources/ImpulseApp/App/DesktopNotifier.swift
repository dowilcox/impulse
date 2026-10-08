import AppKit
import UserNotifications
import os.log

/// macOS notifications for terminals that want the user while Impulse is in
/// the background: a program's own notification (OSC 9/777/99), a long
/// command finishing, a bell. One thread per workspace; clicking one brings
/// its window forward and focuses the pane. Delivered notifications for a
/// terminal are removed once it's looked at.
///
/// Only bundled builds can use the notification center (a bare `swift build`
/// binary has no bundle identity), so elsewhere this does nothing.
final class DesktopNotifier: NSObject, UNUserNotificationCenterDelegate {
  static let shared = DesktopNotifier()

  /// Posted on the main queue when the user clicks a notification;
  /// userInfo["terminal"] is the terminal's id string.
  static let revealTerminal = Notification.Name("impulse.revealTerminal")

  private let isAvailable =
    Bundle.main.bundleURL.pathExtension == "app" && Bundle.main.bundleIdentifier != nil
    && AppState.persistenceEnabled
  private var center: UNUserNotificationCenter? {
    isAvailable ? UNUserNotificationCenter.current() : nil
  }
  private var authorization: Bool?
  private var pending: [UNNotificationRequest] = []
  /// Delivered notification ids per terminal id.
  private var delivered: [String: [String]] = [:]

  func activate() {
    center?.delegate = self
  }

  /// Show a notification for a terminal. `thread` groups a workspace's
  /// notifications.
  func post(title: String, subtitle: String?, body: String, terminalID: UUID, thread: String) {
    let id = UUID().uuidString
    delivered[terminalID.uuidString, default: []].append(id)
    post(
      id: id, title: title, subtitle: subtitle, body: body,
      userInfo: ["terminal": terminalID.uuidString], thread: thread)
  }

  /// A notification that brings Impulse forward when clicked.
  func post(title: String, subtitle: String?, body: String, thread: String) {
    post(id: UUID().uuidString, title: title, subtitle: subtitle, body: body, userInfo: [:], thread: thread)
  }

  /// A notification that opens `url` when clicked (e.g. a pull request).
  func post(title: String, subtitle: String?, body: String, url: String, thread: String) {
    post(id: UUID().uuidString, title: title, subtitle: subtitle, body: body, userInfo: ["url": url], thread: thread)
  }

  private func post(
    id: String, title: String, subtitle: String?, body: String, userInfo: [String: String], thread: String
  ) {
    guard let center else { return }
    let content = UNMutableNotificationContent()
    content.title = title
    if let subtitle, !subtitle.isEmpty { content.subtitle = subtitle }
    content.body = body
    content.threadIdentifier = thread
    content.sound = .default
    content.userInfo = userInfo
    let request = UNNotificationRequest(identifier: id, content: content, trigger: nil)

    switch authorization {
    case true?:
      add(request)
    case false?:
      break
    case nil:
      pending.append(request)
      guard pending.count == 1 else { return }
      center.requestAuthorization(options: [.alert, .sound, .badge]) { [weak self] granted, error in
        if let error {
          os_log(.info, "Notification authorization failed: %{public}@", error.localizedDescription)
        }
        DispatchQueue.main.async {
          guard let self else { return }
          self.authorization = granted
          let queued = self.pending
          self.pending = []
          if granted { queued.forEach(self.add) }
        }
      }
    }
  }

  /// Drop a terminal's delivered notifications (it has been seen).
  func clear(terminalID: UUID) {
    guard let ids = delivered.removeValue(forKey: terminalID.uuidString), let center else { return }
    center.removeDeliveredNotifications(withIdentifiers: ids)
  }

  /// The Dock badge: how many terminals need attention ("" hides it).
  func setBadge(count: Int) {
    NSApp.dockTile.badgeLabel = count > 0 ? "\(count)" : nil
  }

  private func add(_ request: UNNotificationRequest) {
    center?.add(request) { error in
      if let error {
        os_log(.info, "Notification failed: %{public}@", error.localizedDescription)
      }
    }
  }

  // MARK: UNUserNotificationCenterDelegate

  func userNotificationCenter(
    _ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
    withCompletionHandler completionHandler: @escaping () -> Void
  ) {
    let terminal = response.notification.request.content.userInfo["terminal"] as? String
    let url = (response.notification.request.content.userInfo["url"] as? String).flatMap(URL.init(string:))
    DispatchQueue.main.async {
      if let url, url.scheme == "https" {
        NSWorkspace.shared.open(url)
        return completionHandler()
      }
      NSApp.activate(ignoringOtherApps: true)
      if let terminal {
        NotificationCenter.default.post(
          name: Self.revealTerminal, object: nil, userInfo: ["terminal": terminal])
      }
      completionHandler()
    }
  }

  func userNotificationCenter(
    _ center: UNUserNotificationCenter, willPresent notification: UNNotification,
    withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
  ) {
    // Impulse only posts while it's in the background; if it came forward
    // in the meantime the tab's attention dot is enough.
    completionHandler([])
  }
}
