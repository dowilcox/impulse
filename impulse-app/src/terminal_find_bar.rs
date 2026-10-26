//! Find in a terminal (the Swift `TerminalFindBar` and
//! `MainWindowController+FindBar`): the shared find bar (`find_bar`) over
//! the grid, searching the scrollback. The query becomes the core's regex
//! through `impulse_kit::terminal_find`.

use gpui_kit::{AnyElement, Context, Window};
use impulse_kit::terminal_find::TerminalFindQuery;

use crate::find_bar::{FindBar, FindHost};
use crate::terminal_view::TerminalView;

impl FindHost for TerminalView {
    fn find_bar_mut(&mut self) -> Option<&mut FindBar> {
        self.find.as_mut()
    }

    fn find_changed(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.run_find(cx);
    }

    fn find_step(&mut self, forward: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.step_find(forward, cx);
    }

    fn find_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_find(window, cx);
    }
}

impl TerminalView {
    /// Find: show the bar (seeded from a one-line selection), or hide it.
    pub fn toggle_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.is_some() {
            return self.close_find(window, cx);
        }
        let selection = self
            .backend
            .selected_text()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty() && !t.contains('\n'));
        let mut query = TerminalFindQuery::default();
        if let Some(text) = &self.last_find {
            query = text.clone();
        }
        if let Some(selection) = selection {
            query.text = selection;
        }
        let theme = self.theme.clone();
        let find = FindBar::new(&theme, "Find in terminal", query, false, window, cx);
        find.focus(window, cx);
        self.find = Some(find);
        self.run_find(cx);
    }

    /// Find `text` (the snapshot's `find=`), showing the bar if it's hidden.
    #[cfg(debug_assertions)]
    pub(crate) fn find_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.is_none() {
            self.toggle_find(window, cx);
        }
        if let Some(find) = self.find.as_mut() {
            find.set_query_text(text, cx);
        }
        self.run_find(cx);
    }

    /// Whole words only (the snapshot's `find-word`).
    #[cfg(debug_assertions)]
    pub(crate) fn find_whole_word(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.is_none() {
            self.toggle_find(window, cx);
        }
        if let Some(find) = self.find.as_mut() {
            find.query.whole_word = true;
        }
        self.run_find(cx);
    }

    /// Hide the bar, clear the matches and give the terminal the keyboard.
    pub(crate) fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(find) = self.find.take() {
            self.last_find = Some(find.query);
        }
        self.backend.search_clear();
        self.focus(window, cx);
        cx.notify();
    }

    fn run_find(&mut self, cx: &mut Context<Self>) {
        let Some(find) = self.find.as_mut() else {
            return;
        };
        match find.query.pattern() {
            Some(pattern) => {
                self.backend.search(&pattern);
            }
            None => self.backend.search_clear(),
        }
        find.stats = self.backend.search_stats();
        cx.notify();
    }

    fn step_find(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(find) = self.find.as_mut() else {
            return;
        };
        if find.query.pattern().is_none() {
            return;
        }
        if forward {
            self.backend.search_next();
        } else {
            self.backend.search_prev();
        }
        find.stats = self.backend.search_stats();
        cx.notify();
    }

    pub(crate) fn find_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        Some(self.find.as_ref()?.render(&self.palette, cx))
    }
}
