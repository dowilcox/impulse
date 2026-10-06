import AppKit
import ImpulseGit
import ImpulseKit
import SwiftUI
import os.log

extension MainWindowController {

  // MARK: - Terminal Search Bar

  func setupTerminalSearchBar() {
    let host = tabManager.contentView

    termSearchBar.translatesAutoresizingMaskIntoConstraints = false
    // Clip contents so the bar wipes into view as it grows instead of
    // overflowing fully formed while the height animates.
    termSearchBar.wantsLayer = true
    termSearchBar.layer?.masksToBounds = true
    termSearchBar.isHidden = true

    termFind.onChange = { [weak self] query in self?.runTerminalFind(query) }
    termFind.onNext = { [weak self] in self?.stepTerminalFind(forward: true) }
    termFind.onPrevious = { [weak self] in self?.stepTerminalFind(forward: false) }
    termFind.onClose = { [weak self] in self?.hideTerminalSearch() }
    let bar = WorkbenchHosting.make(TerminalFindBar(model: termFind))
    bar.translatesAutoresizingMaskIntoConstraints = false
    termSearchBar.addSubview(bar)
    host.addSubview(termSearchBar)

    let heightConstraint = termSearchBar.heightAnchor.constraint(equalToConstant: 0)
    termSearchHeightConstraint = heightConstraint

    NSLayoutConstraint.activate([
      termSearchBar.topAnchor.constraint(equalTo: host.topAnchor),
      termSearchBar.leadingAnchor.constraint(equalTo: host.leadingAnchor),
      termSearchBar.trailingAnchor.constraint(equalTo: host.trailingAnchor),
      heightConstraint,
      bar.topAnchor.constraint(equalTo: termSearchBar.topAnchor),
      bar.leadingAnchor.constraint(equalTo: termSearchBar.leadingAnchor),
      bar.trailingAnchor.constraint(equalTo: termSearchBar.trailingAnchor),
      bar.heightAnchor.constraint(equalToConstant: 32),
    ])
  }

  private func runTerminalFind(_ query: TerminalFindQuery) {
    guard let terminal = tabManager.selectedTerminal?.activeTerminal else { return }
    if let pattern = query.pattern {
      terminal.search(pattern)
    } else {
      terminal.searchClear()
    }
    termFind.update(terminal.searchStats())
  }

  private func stepTerminalFind(forward: Bool) {
    guard let terminal = tabManager.selectedTerminal?.activeTerminal, termFind.query.pattern != nil else { return }
    if forward { terminal.searchNext() } else { terminal.searchPrev() }
    termFind.update(terminal.searchStats())
  }

  /// Toggles the terminal search bar visibility.
  func toggleTerminalSearch() {
    guard tabManager.selectedTerminal != nil else {
      NSSound.beep()
      return
    }

    if termSearchBarVisible {
      hideTerminalSearch()
    } else {
      showTerminalSearch()
    }
  }

  private func showTerminalSearch() {
    termSearchBarVisible = true
    termSearchBar.isHidden = false
    // The active tab's view is added to `contentView` on every tab switch
    // (TabManager), so it sits above the search bar (added once at setup) and
    // occludes the field + buttons — only the field's focus ring, which draws
    // outside the view bounds, escapes. Re-raise the bar to the front so its
    // contents are visible.
    termSearchBar.superview?.addSubview(termSearchBar, positioned: .above, relativeTo: nil)
    termSearchBar.alphaValue = 0
    NSAnimationContext.runAnimationGroup(
      { context in
        context.duration = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion ? 0 : 0.2
        context.timingFunction = CAMediaTimingFunction(name: .easeOut)
        context.allowsImplicitAnimation = true
        termSearchHeightConstraint?.constant = 32
        termSearchBar.alphaValue = 1
        tabManager.contentView.layoutSubtreeIfNeeded()
      },
      completionHandler: { [weak self] in
        self?.termFind.focusToken += 1
      })

    // Seed from a one-line selection in the grid, else search again for
    // what was there.
    termFind.palette = windowModel.palette
    if let selection = tabManager.selectedTerminal?.activeTerminal?.singleLineSelection {
      termFind.query.text = selection
    } else {
      runTerminalFind(termFind.query)
    }
  }

  /// Hides the terminal search bar, clears search state, and returns focus
  /// to the active terminal.
  func hideTerminalSearch() {
    termSearchBarVisible = false
    if let terminal = tabManager.selectedTerminal?.activeTerminal {
      terminal.searchClear()
    }

    NSAnimationContext.runAnimationGroup(
      { context in
        context.duration = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion ? 0 : 0.16
        context.timingFunction = CAMediaTimingFunction(name: .easeIn)
        context.allowsImplicitAnimation = true
        termSearchHeightConstraint?.constant = 0
        termSearchBar.alphaValue = 0
        tabManager.contentView.layoutSubtreeIfNeeded()
      },
      completionHandler: { [weak self] in
        guard let self else { return }
        self.termSearchBar.isHidden = true
        self.termSearchBar.alphaValue = 1
        if let terminal = self.tabManager.selectedTerminal?.activeTerminal {
          terminal.focus()
        }
      })
  }

  /// Updates the status bar with information from the currently active tab.
  func updateStatusBar() {
    guard let tabInfo = tabManager.activeTabInfo else { return }

    if let shellName = tabInfo.shellName {
      let cwd = tabInfo.cwd ?? NSHomeDirectory()
      // Sync to SwiftUI
      windowModel.shellName = shellName
      windowModel.currentCwd = cwd
      bindRepository(forDirectory: cwd)
      windowModel.cursorLine = nil
      windowModel.cursorCol = nil
      windowModel.currentLanguage = nil
      windowModel.currentIndent = nil
      windowModel.isPreviewable = false
      windowModel.isPreviewing = false
      let active = tabManager.selectedTerminal?.activeTerminal
      windowModel.commandRunning = active?.isCommandRunning ?? false
      windowModel.passwordInputActive = active?.isPasswordInput ?? false
      windowModel.lastCommandExitCode = active?.lastCommandExitCode
      windowModel.lastCommandDurationMs = active?.lastCommandDurationMs
      windowModel.terminalDirectInteraction = active?.isDirectInteraction ?? false
      windowModel.focusedTerminalID = active?.id
    } else if let language = tabInfo.language {
      let cwd = tabInfo.cwd ?? ""
      // Sync to SwiftUI
      windowModel.shellName = ""
      // An editor tab never owns a TUI — keep the status-bar pills interactive.
      windowModel.terminalDirectInteraction = false
      windowModel.currentCwd = cwd
      bindRepository(forDirectory: cwd)
      windowModel.cursorLine = tabInfo.cursorLine
      windowModel.cursorCol = tabInfo.cursorCol
      windowModel.currentLanguage = language
      windowModel.currentIndent =
        settings.useSpaces
        ? "Spaces: \(settings.tabWidth)" : "Tab Size: \(settings.tabWidth)"
      // Show/hide preview button based on file type
      if let editor = tabManager.selectedEditor,
        let fp = editor.filePath,
        EditorTab.isPreviewableFile(fp)
      {
        windowModel.isPreviewable = true
        windowModel.isPreviewing = editor.isPreviewing
      } else {
        windowModel.isPreviewable = false
        windowModel.isPreviewing = false
      }
    }
  }

  /// Re-applies theme colors to all child views.
  func handleThemeChange(_ newTheme: Theme) {
    theme = newTheme
    windowModel.theme = newTheme
    windowModel.iconCache = tabManager.iconCache

    // Switch window chrome between light and dark appearance
    window?.appearance = NSAppearance(named: newTheme.isLight ? .aqua : .darkAqua)

    // Toolbar buttons: flat icons for card-surface themes, bordered otherwise.
    let bordered = newTheme.surfaceStyle != "card"
    for item in window?.toolbar?.items ?? []
    where !(item is NSSearchToolbarItem) && !(item is NSTrackingSeparatorToolbarItem) {
      item.isBordered = bordered
    }

    // Window background — use bgSurface so the titlebar blends with the tab bar
    window?.backgroundColor = newTheme.bgSurfaceColor

    // Tab manager (propagates to all tabs)
    tabManager.applyTheme(newTheme)

    // Re-render previews with updated theme
    let themeJSON = ThemeManager.markdownThemeJSON(forName: newTheme.id)
    for tab in tabManager.allSurfaces {
      if case .editor(let editor) = tab {
        editor.refreshPreview(themeJSON: themeJSON, bgColor: newTheme.bg)
      }
    }
  }
}
