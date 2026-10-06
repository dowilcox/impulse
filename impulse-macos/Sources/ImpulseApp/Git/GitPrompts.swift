import AppKit
import ImpulseGit
import ImpulseKit

/// Sheets that collect what a git action needs before it runs.
enum GitPrompts {
  struct NewTag {
    let name: String
    /// Nil or empty: a lightweight tag.
    let message: String?
    let push: Bool
  }

  /// Ask for a tag at `revision`: a name (the next version is suggested),
  /// an optional message (annotated tag), and whether to push it — the
  /// checkbox starts from the "Push new tags" setting.
  static func askForTag(
    in window: NSWindow, root: String, revision: String, subject: String?, host: GitPanelHost?,
    then: @escaping (NewTag) -> Void
  ) {
    DispatchQueue.global(qos: .userInitiated).async {
      let remote = GitOperations.defaultRemote(root: root)
      let suggestion = TagNameSuggestion.next(after: GitOperations.tags(root: root)) ?? ""
      let subject = subject ?? GitOperations.headMessage(root: root)?.split(separator: "\n").first.map(String.init)
      DispatchQueue.main.async {
        present(
          in: window, revision: revision, subject: subject, remote: remote, suggestion: suggestion, host: host,
          then: then)
      }
    }
  }

  private static func present(
    in window: NSWindow, revision: String, subject: String?, remote: String?, suggestion: String,
    host: GitPanelHost?, then: @escaping (NewTag) -> Void
  ) {
    let alert = NSAlert()
    alert.messageText = revision == "HEAD" ? "New tag on the current commit" : "New tag at \(revision.prefix(7))"
    alert.informativeText = subject ?? ""
    alert.addButton(withTitle: "Create Tag")
    alert.addButton(withTitle: "Cancel")

    let name = NSTextField(string: suggestion)
    name.placeholderString = "v1.0.0"
    let message = NSTextField(string: "")
    message.placeholderString = "Message (optional: makes an annotated tag)"
    let push = NSButton(checkboxWithTitle: "Push to \(remote ?? "origin")", target: nil, action: nil)
    push.state = SettingsStore.shared.settings.gitPushTagsOnCreate ? .on : .off
    push.isHidden = remote == nil

    let stack = NSStackView(views: [name, message, push])
    stack.orientation = .vertical
    stack.alignment = .leading
    stack.spacing = 8
    for field in [name, message] {
      field.translatesAutoresizingMaskIntoConstraints = false
      field.widthAnchor.constraint(equalToConstant: 300).isActive = true
    }
    stack.frame = NSRect(x: 0, y: 0, width: 300, height: remote == nil ? 56 : 80)
    alert.accessoryView = stack
    alert.window.initialFirstResponder = name

    alert.beginSheetModal(for: window) { response in
      guard response == .alertFirstButtonReturn else { return }
      let tagName = name.stringValue.trimmingCharacters(in: .whitespaces)
      guard GitRefName.isValid(tagName) else {
        host?.toasts.show(Toast(kind: .warning, message: "“\(tagName)” isn't a valid tag name."))
        return
      }
      let text = message.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
      then(NewTag(name: tagName, message: text.isEmpty ? nil : text, push: remote != nil && push.state == .on))
    }
  }
}
