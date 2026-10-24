//! The Vim layer on plain text: `|` marks the cursor. A small stand-in
//! editor applies each outcome, types Insert mode's keys itself, and keeps
//! an undo history, as the real editor does.

use super::*;

struct Ed {
    vim: Vim,
    text: String,
    cursor: usize,
    selection: Range<usize>,
    clipboard: Option<String>,
    actions: Vec<Action>,
    undo: Vec<(String, usize)>,
    redo: Vec<(String, usize)>,
    view: Range<usize>,
    typing: bool,
}

impl Ed {
    fn new(s: &str) -> Self {
        let cursor = s.find('|').expect("a cursor");
        let text = s.replacen('|', "", 1);
        let mut ed = Ed {
            vim: Vim::new(),
            text,
            cursor,
            selection: cursor..cursor,
            clipboard: None,
            actions: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            view: 0..0,
            typing: false,
        };
        ed.sync();
        ed
    }

    fn sync(&mut self) {
        if let Some(sel) = self
            .vim
            .sync(&self.text, self.cursor, self.selection.clone())
        {
            self.cursor = sel.end;
            self.selection = sel;
        }
    }

    fn keys(&mut self, keys: &str) -> &mut Self {
        for key in Key::parse_all(keys) {
            self.key(key);
        }
        self
    }

    fn key(&mut self, key: Key) {
        let clipboard = self.clipboard.clone();
        let mut read = move || clipboard.clone();
        let mut cx = Context {
            text: &self.text,
            cursor: self.cursor,
            visible_lines: self.view.clone(),
            indent: "    ",
            tab_size: 4,
            clipboard: &mut read,
        };
        let outcome = self.vim.key(key, &mut cx);
        if !outcome.handled {
            self.type_key(key);
            return;
        }
        self.typing = false;
        if let Some((range, text)) = outcome.edit {
            self.undo.push((self.text.clone(), self.cursor));
            self.redo.clear();
            self.text.replace_range(range, &text);
        }
        if let Some(sel) = outcome.selection {
            self.cursor = if sel.is_empty() { sel.start } else { sel.end };
            self.selection = sel;
        }
        for action in outcome.actions {
            match action {
                Action::Undo(n) => {
                    for _ in 0..n {
                        if let Some((text, cursor)) = self.undo.pop() {
                            self.redo
                                .push((std::mem::replace(&mut self.text, text), self.cursor));
                            self.cursor = cursor;
                        }
                    }
                    self.selection = self.cursor..self.cursor;
                    self.sync();
                }
                Action::Redo(n) => {
                    for _ in 0..n {
                        if let Some((text, cursor)) = self.redo.pop() {
                            self.undo
                                .push((std::mem::replace(&mut self.text, text), self.cursor));
                            self.cursor = cursor;
                        }
                    }
                    self.selection = self.cursor..self.cursor;
                    self.sync();
                }
                Action::CopyToClipboard(text) => {
                    self.clipboard = Some(text.clone());
                    self.actions.push(Action::CopyToClipboard(text));
                }
                other => self.actions.push(other),
            }
        }
    }

    /// The editor's own typing, in Insert mode.
    fn type_key(&mut self, key: Key) {
        if !self.typing {
            self.undo.push((self.text.clone(), self.cursor));
            self.typing = true;
        }
        match key {
            Key::Char(c) => {
                self.text.insert(self.cursor, c);
                self.cursor += c.len_utf8();
            }
            Key::Enter => {
                self.text.insert(self.cursor, '\n');
                self.cursor += 1;
            }
            Key::Backspace if self.cursor > 0 => {
                let at = text::prev(&self.text, self.cursor);
                self.text.replace_range(at..self.cursor, "");
                self.cursor = at;
            }
            Key::Left if self.cursor > 0 => self.cursor = text::prev(&self.text, self.cursor),
            Key::Right if self.cursor < self.text.len() => {
                self.cursor = text::next(&self.text, self.cursor)
            }
            _ => {}
        }
        self.selection = self.cursor..self.cursor;
    }

    fn show(&self) -> String {
        let mut s = self.text.clone();
        s.insert(self.cursor, '|');
        s
    }

    fn selected(&self) -> &str {
        &self.text[self.selection.clone()]
    }
}

/// Run `keys` on `start` and show the text and cursor afterwards.
fn run(start: &str, keys: &str) -> String {
    Ed::new(start).keys(keys).show()
}

// MARK: Motions

#[test]
fn moves_by_characters_and_lines() {
    assert_eq!(run("a|bcd", "l"), "ab|cd");
    assert_eq!(run("a|bcd", "5l"), "abc|d");
    assert_eq!(run("abc|d", "h"), "ab|cd");
    assert_eq!(run("abc|d", "9h"), "|abcd");
    assert_eq!(run("ab|c\ndef\nghi", "j"), "abc\nde|f\nghi");
    assert_eq!(run("ab|c\ndef\nghi", "2j"), "abc\ndef\ngh|i");
    assert_eq!(run("abc\ndef\ngh|i", "k"), "abc\nde|f\nghi");
    // The column is kept across a shorter line.
    assert_eq!(run("abc|d\nx\nabcd", "jj"), "abcd\nx\nabc|d");
    assert_eq!(run("abc|d\nx\nabcd", "j"), "abcd\n|x\nabcd");
    // `$` sticks to line ends.
    assert_eq!(run("a|b\nabcdef\nabc", "$jj"), "ab\nabcdef\nab|c");
    // No line break to stand on in Normal mode.
    assert_eq!(run("ab|c", "l"), "ab|c");
    assert_eq!(run("abc|", ""), "ab|c");
}

#[test]
fn moves_by_words() {
    let t = "|foo.bar baz  qux\nnext";
    assert_eq!(run(t, "w"), "foo|.bar baz  qux\nnext");
    assert_eq!(run(t, "2w"), "foo.|bar baz  qux\nnext");
    assert_eq!(run(t, "W"), "foo.bar |baz  qux\nnext");
    assert_eq!(run(t, "4w"), "foo.bar baz  |qux\nnext");
    assert_eq!(run(t, "5w"), "foo.bar baz  qux\n|next");
    assert_eq!(run(t, "e"), "fo|o.bar baz  qux\nnext");
    assert_eq!(run(t, "E"), "foo.ba|r baz  qux\nnext");
    assert_eq!(run("foo.bar baz|", "b"), "foo.bar |baz");
    assert_eq!(run("foo.bar ba|z", "b"), "foo.bar |baz");
    assert_eq!(run("foo.bar ba|z", "2b"), "foo.|bar baz");
    assert_eq!(run("foo.bar ba|z", "B"), "foo.bar |baz");
    assert_eq!(run("foo.bar ba|z", "2B"), "|foo.bar baz");
    assert_eq!(run("foo bar ba|z", "ge"), "foo ba|r baz");
    // An empty line is a word.
    assert_eq!(run("|a\n\nb", "w"), "a\n|\nb");
    assert_eq!(run("a\n\n|b", "b"), "a\n|\nb");
}

#[test]
fn moves_within_lines() {
    assert_eq!(run("  foo b|ar", "0"), "|  foo bar");
    assert_eq!(run("  foo b|ar", "^"), "  |foo bar");
    assert_eq!(run("  f|oo bar", "$"), "  foo ba|r");
    assert_eq!(run("a|bc\ndef", "2$"), "abc\nde|f");
    assert_eq!(run("|a,b,c,d", "f,"), "a|,b,c,d");
    assert_eq!(run("|a,b,c,d", "2f,"), "a,b|,c,d");
    assert_eq!(run("|a,b,c,d", "t,"), "|a,b,c,d");
    assert_eq!(run("|a,b,c,d", "lt,"), "a,|b,c,d");
    assert_eq!(run("a,b,c,|d", "F,"), "a,b,c|,d");
    assert_eq!(run("a,b,c,|d", "T,"), "a,b,c,|d");
    assert_eq!(run("|a,b,c,d", "f,;"), "a,b|,c,d");
    assert_eq!(run("|a,b,c,d", "f,;;,"), "a,b|,c,d");
    // `;` after `t` moves on to the next one.
    assert_eq!(run("|a,b,c,d", "t,;"), "a,|b,c,d");
    assert_eq!(run("|a,b", "fz"), "|a,b");
    assert_eq!(run("ab|cdef", "4|"), "abc|def");
}

#[test]
fn moves_through_the_file() {
    let t = "one\n  two\nth|ree\nfour";
    assert_eq!(run(t, "gg"), "|one\n  two\nthree\nfour");
    assert_eq!(run(t, "G"), "one\n  two\nthree\n|four");
    assert_eq!(run(t, "2G"), "one\n  |two\nthree\nfour");
    assert_eq!(run(t, "2gg"), "one\n  |two\nthree\nfour");
    assert_eq!(run(t, "-"), "one\n  |two\nthree\nfour");
    assert_eq!(run(t, "+"), "one\n  two\nthree\n|four");
    assert_eq!(run(t, "<CR>"), "one\n  two\nthree\n|four");
    assert_eq!(run(t, "50%"), "one\n  |two\nthree\nfour");
}

#[test]
fn matches_brackets() {
    assert_eq!(run("|f(a, [b]) {}", "%"), "f(a, [b]|) {}");
    assert_eq!(run("f(a, [b]|) {}", "%"), "f|(a, [b]) {}");
    assert_eq!(run("f(a, |[b]) {}", "%"), "f(a, [b|]) {}");
    assert_eq!(run("fn x() {\n  |a\n}", "%"), "fn x() {\n  |a\n}");
    assert_eq!(run("fn x() |{\n  a\n}", "%"), "fn x() {\n  a\n|}");
}

#[test]
fn moves_by_paragraphs() {
    let t = "|a\nb\n\nc\nd\n\n\ne";
    assert_eq!(run(t, "}"), "a\nb\n|\nc\nd\n\n\ne");
    assert_eq!(run(t, "2}"), "a\nb\n\nc\nd\n|\n\ne");
    assert_eq!(run(t, "3}"), "a\nb\n\nc\nd\n\n\n|e");
    assert_eq!(run("a\nb\n\nc\n|d", "{"), "a\nb\n|\nc\nd");
    assert_eq!(run("a\nb\n\nc\n|d", "2{"), "|a\nb\n\nc\nd");
}

#[test]
fn moves_on_screen() {
    let mut ed = Ed::new("|0\n1\n2\n3\n4\n5\n6\n7\n8\n9");
    ed.view = 2..7;
    assert_eq!(ed.keys("H").show(), "0\n1\n|2\n3\n4\n5\n6\n7\n8\n9");
    assert_eq!(ed.keys("L").show(), "0\n1\n2\n3\n4\n5\n|6\n7\n8\n9");
    assert_eq!(ed.keys("M").show(), "0\n1\n2\n3\n|4\n5\n6\n7\n8\n9");
    assert_eq!(ed.keys("2H").show(), "0\n1\n2\n|3\n4\n5\n6\n7\n8\n9");
    ed.keys("<C-d>");
    assert_eq!(ed.actions.pop(), Some(Action::Scroll(2)));
    assert_eq!(ed.show(), "0\n1\n2\n3\n4\n|5\n6\n7\n8\n9");
    ed.keys("<C-u>");
    assert_eq!(ed.actions.pop(), Some(Action::Scroll(-2)));
    ed.keys("zz");
    assert_eq!(ed.actions.pop(), Some(Action::Reveal(Reveal::Center)));
    ed.keys("zt");
    assert_eq!(ed.actions.pop(), Some(Action::Reveal(Reveal::Top)));
}

#[test]
fn searches() {
    let t = "|foo bar\nbaz foo\nfoo";
    assert_eq!(run(t, "/foo<CR>"), "foo bar\nbaz |foo\nfoo");
    assert_eq!(run(t, "/foo<CR>n"), "foo bar\nbaz foo\n|foo");
    assert_eq!(run(t, "/foo<CR>nn"), "|foo bar\nbaz foo\nfoo");
    assert_eq!(run(t, "/foo<CR>N"), "|foo bar\nbaz foo\nfoo");
    assert_eq!(run(t, "?foo<CR>"), "foo bar\nbaz foo\n|foo");
    assert_eq!(run(t, "?ba<CR>n"), "foo |bar\nbaz foo\nfoo");
    assert_eq!(run(t, "*"), "foo bar\nbaz |foo\nfoo");
    assert_eq!(run(t, "#"), "foo bar\nbaz foo\n|foo");
    assert_eq!(run("|foobar foo", "*"), "|foobar foo");
    assert_eq!(run(t, "2/o<CR>"), "fo|o bar\nbaz foo\nfoo");
    // Regexes, and `\<`/`\>` as word boundaries.
    assert_eq!(run(t, "/\\<ba.<CR>"), "foo |bar\nbaz foo\nfoo");
    let mut ed = Ed::new(t);
    ed.keys("/nope<CR>");
    assert_eq!(
        ed.vim.status().message.as_deref(),
        Some("Pattern not found: nope")
    );
    assert_eq!(ed.show(), t);
    ed.keys("G/bar<CR>");
    assert_eq!(
        ed.vim.status().message.as_deref(),
        Some("Search hit BOTTOM, continuing at TOP")
    );
    assert_eq!(ed.vim.search_highlights(&ed.text), vec![4..7]);
    ed.keys(":noh<CR>");
    assert!(ed.vim.search_highlights(&ed.text).is_empty());
}

// MARK: Operators

#[test]
fn deletes() {
    assert_eq!(run("foo |bar baz", "dw"), "foo |baz");
    assert_eq!(run("foo |bar baz qux", "d2w"), "foo |qux");
    assert_eq!(run("foo |bar baz qux", "2dw"), "foo |qux");
    assert_eq!(run("foo |bar baz qux", "2d2w"), "foo| ");
    // The last word of a line goes without the line break.
    assert_eq!(run("foo |bar\nbaz", "dw"), "foo| \nbaz");
    assert_eq!(run("foo |bar baz", "de"), "foo | baz");
    assert_eq!(run("foo bar |baz", "db"), "foo |baz");
    assert_eq!(run("foo b|ar baz", "d$"), "foo |b");
    assert_eq!(run("foo b|ar baz", "D"), "foo |b");
    assert_eq!(run("  foo b|ar", "d0"), "|ar");
    assert_eq!(run("  foo b|ar", "d^"), "  |ar");
    assert_eq!(run("a,b,|c,d", "dt,"), "a,b,|,d");
    assert_eq!(run("a,b,|c,d", "df,"), "a,b,|d");
    assert_eq!(run("a,b,|c,d", "dF,"), "a,b|c,d");
    assert_eq!(run("f(|a, b) x", "d%"), "f|, b) x");
    assert_eq!(run("f|(a, b) x", "d%"), "f| x");
    assert_eq!(run("ab|cd", "x"), "ab|d");
    assert_eq!(run("ab|cd", "3x"), "a|b");
    assert_eq!(run("abc|d", "x"), "ab|c");
    assert_eq!(run("ab|cd", "X"), "a|cd");
    assert_eq!(run("ab|cd", "5X"), "|cd");
}

#[test]
fn deletes_lines() {
    let t = "one\n  t|wo\nthree\nfour";
    assert_eq!(run(t, "dd"), "one\n|three\nfour");
    assert_eq!(run(t, "2dd"), "one\n|four");
    assert_eq!(run(t, "d2d"), "one\n|four");
    assert_eq!(run(t, "9dd"), "|one");
    assert_eq!(run(t, "dj"), "one\n|four");
    assert_eq!(run(t, "dk"), "|three\nfour");
    assert_eq!(run(t, "dG"), "|one");
    assert_eq!(run(t, "dgg"), "|three\nfour");
    assert_eq!(run("one\n|two", "dd"), "|one");
    assert_eq!(run("|one", "dd"), "|");
    assert_eq!(run("one\ntwo\n|", "dd"), "one\n|two");
    // `dj` on the last line has no line to take.
    assert_eq!(run("one\n|two", "dj"), "one\n|two");
}

#[test]
fn deletes_paragraph_motions() {
    // From the start of a line: whole lines.
    assert_eq!(run("|a\nb\n\nc", "d}"), "|\nc");
    // From inside a line: up to that line's end.
    assert_eq!(run("a|x\nb\n\nc", "d}"), "|a\n\nc");
}

#[test]
fn changes() {
    let mut ed = Ed::new("foo |bar baz");
    ed.keys("cw");
    assert_eq!(ed.vim.mode(), Mode::Insert);
    assert_eq!(ed.show(), "foo | baz");
    ed.keys("qux<Esc>");
    assert_eq!(ed.vim.mode(), Mode::Normal);
    assert_eq!(ed.show(), "foo qu|x baz");
    // `cw` on a word's last character changes only that.
    assert_eq!(run("fo|o bar", "cwx<Esc>"), "fo|x bar");
    assert_eq!(run("foo |bar baz", "c2wx<Esc>"), "foo |x");
    assert_eq!(run("foo |bar baz", "cex<Esc>"), "foo |x baz");
    assert_eq!(run("foo b|ar baz", "Cx<Esc>"), "foo b|x");
    assert_eq!(run("foo b|ar baz", "c$x<Esc>"), "foo b|x");
    assert_eq!(run("ab|cd", "sx<Esc>"), "ab|xd");
    assert_eq!(run("ab|cd", "2sx<Esc>"), "ab|x");
    // `cc` and `S` keep the indentation.
    assert_eq!(run("a\n    b|c\nd", "ccx<Esc>"), "a\n    |x\nd");
    assert_eq!(run("a\n    b|c\nd", "Sx<Esc>"), "a\n    |x\nd");
    assert_eq!(run("a\n  b|c\nd\ne", "2ccx<Esc>"), "a\n  |x\ne");
}

#[test]
fn yanks_and_puts() {
    assert_eq!(run("|foo bar", "ywP"), "foo| foo bar");
    assert_eq!(run("|foo bar", "ywp"), "ffoo| oo bar");
    assert_eq!(run("|foo bar", "yw$p"), "foo barfoo| ");
    assert_eq!(run("|foo bar", "yw3P"), "foo foo foo| foo bar");
    assert_eq!(run("o|ne\ntwo", "yyp"), "one\n|one\ntwo");
    assert_eq!(run("o|ne\ntwo", "yyP"), "|one\none\ntwo");
    assert_eq!(run("one\nt|wo", "yyp"), "one\ntwo\n|two");
    assert_eq!(run("one\nt|wo", "Yp"), "one\ntwo\n|two");
    assert_eq!(run("o|ne\ntwo", "yj2p"), "one\n|one\ntwo\none\ntwo\ntwo");
    assert_eq!(run("one\nt|wo", "ykP"), "|one\ntwo\none\ntwo");
    // `yb` leaves the cursor where the yank started.
    assert_eq!(run("foo b|ar", "yb"), "foo |bar");
    // Delete, then put: a swap of characters.
    assert_eq!(run("|ab", "xp"), "b|a");
    assert_eq!(run("one\n|two\nthree", "ddp"), "one\nthree\n|two");
}

#[test]
fn indents() {
    assert_eq!(run("a|b\ncd", ">>"), "    |ab\ncd");
    assert_eq!(run("a|b\ncd", "2>>"), "    |ab\n    cd");
    assert_eq!(run("a|b\n\ncd", ">2j"), "    |ab\n\n    cd");
    assert_eq!(run("      a|b", "<<"), "  |ab");
    assert_eq!(run("\ta|b", "<<"), "|ab");
    assert_eq!(run("  a|b", "<<"), "|ab");
    assert_eq!(run("a|b\ncd", "Vj>"), "    |ab\n    cd");
    assert_eq!(run("a|b\ncd", "V2>"), "        |ab\ncd");
}

#[test]
fn changes_case() {
    assert_eq!(run("|abc def", "~"), "A|bc def");
    assert_eq!(run("|abc def", "4~"), "ABC |def");
    assert_eq!(run("|abc def", "gUw"), "|ABC def");
    assert_eq!(run("ab|c DEF", "guu"), "|abc def");
    assert_eq!(run("ab|c DEF", "g~~"), "|ABC def");
    assert_eq!(run("|abc def", "veU"), "|ABC def");
    assert_eq!(run("|ABC def", "veu"), "|abc def");
}

#[test]
fn replaces_and_joins() {
    assert_eq!(run("|abc", "rx"), "|xbc");
    assert_eq!(run("|abc", "3rx"), "xx|x");
    assert_eq!(run("|abc", "4rx"), "|abc");
    assert_eq!(run("a|bc", "r<CR>"), "a\n|c");
    assert_eq!(run("|abc def", "verx"), "|xxx def");
    assert_eq!(run("a|b\n   cd\nef", "J"), "ab| cd\nef");
    assert_eq!(run("a|b\n   cd\nef", "3J"), "ab cd| ef");
    assert_eq!(run("a|b \ncd", "J"), "ab |cd");
    assert_eq!(run("f(|\n)", "J"), "f(|)");
    assert_eq!(run("a|b\ncd\nef", "VjJ"), "ab| cd\nef");
    assert_eq!(run("a|b", "J"), "a|b");
}

#[test]
fn replace_mode() {
    let mut ed = Ed::new("|abcd");
    ed.keys("Rxy");
    assert_eq!(ed.vim.mode(), Mode::Replace);
    assert_eq!(ed.vim.status().mode, "--REPLACE--");
    assert_eq!(ed.show(), "xy|cd");
    ed.keys("<BS>");
    assert_eq!(ed.show(), "x|bcd");
    ed.keys("zzzz<Esc>");
    assert_eq!(ed.show(), "xzzz|z");
    assert_eq!(ed.vim.mode(), Mode::Normal);
    // Backspace over what went past the end takes it away.
    assert_eq!(run("a|b", "Rxyz<BS><BS><Esc>"), "a|x");
}

// MARK: Text objects

#[test]
fn word_objects() {
    assert_eq!(run("foo b|ar baz", "diw"), "foo | baz");
    assert_eq!(run("foo b|ar baz", "daw"), "foo |baz");
    assert_eq!(run("foo baz b|ar", "daw"), "foo ba|z");
    assert_eq!(run("foo b|ar baz", "ciwx<Esc>"), "foo |x baz");
    assert_eq!(run("foo b|ar-x baz", "diW"), "foo | baz");
    assert_eq!(run("foo b|ar baz qux", "d2aw"), "foo |qux");
    assert_eq!(run("foo b|ar baz qux", "d3iw"), "foo | qux");
    assert_eq!(run("foo |  bar", "diw"), "foo|bar");
    assert_eq!(run("foo b|ar baz", "yiwP"), "foo ba|rbar baz");
}

#[test]
fn quote_objects() {
    assert_eq!(run("x = \"a|b c\" + y", "di\""), "x = \"|\" + y");
    assert_eq!(run("x = \"a|b c\" + y", "da\""), "x = |+ y");
    assert_eq!(run("x = |'ab' + y", "ci'z<Esc>"), "x = '|z' + y");
    // Before the first quote: the first string on the line.
    assert_eq!(run("|x = \"ab\"", "di\""), "x = \"|\"");
    assert_eq!(run("x = \"a\\\"|b\"", "di\""), "x = \"|\"");
    assert_eq!(run("x|y", "di\""), "x|y");
}

#[test]
fn bracket_objects() {
    assert_eq!(run("f(a, |b) x", "di("), "f(|) x");
    assert_eq!(run("f(a, |b) x", "dib"), "f(|) x");
    assert_eq!(run("f(a, |b) x", "da("), "f| x");
    assert_eq!(run("f(a, (|b)) x", "d2i("), "f(|) x");
    assert_eq!(run("f|(a, b) x", "di)"), "f(|) x");
    assert_eq!(run("f(a, b|) x", "ci(z<Esc>"), "f(|z) x");
    assert_eq!(run("x[1, |2]", "yi[P"), "x[1, |21, 2]");
    assert_eq!(run("<a|b>", "di<"), "<|>");
    // A block whose braces end and start lines: the lines between.
    assert_eq!(run("fn x() {\n    |a;\n    b;\n}", "di{"), "fn x() {\n|}");
    assert_eq!(
        run("fn x() {\n    a;\n    |b;\n}", "ci{z<Esc>"),
        "fn x() {\n    |z\n}"
    );
    assert_eq!(run("fn x() {\n    |a;\n}", "da{"), "fn x()| ");
    assert_eq!(run("no |brackets", "di("), "no |brackets");
}

#[test]
fn paragraph_objects() {
    let t = "a\nb|b\nc\n\n\nd";
    assert_eq!(run(t, "dip"), "|\n\nd");
    assert_eq!(run(t, "dap"), "|d");
    assert_eq!(run(t, "yipP"), "|a\nbb\nc\na\nbb\nc\n\n\nd");
    assert_eq!(run("a\n\n|d\ne", "dap"), "|a");
    assert_eq!(run("a\n\n|d\ne", "vipd"), "a\n|");
}

// MARK: Visual mode

#[test]
fn visual_selects_characters() {
    let mut ed = Ed::new("foo |bar baz");
    ed.keys("v");
    assert_eq!(ed.vim.status().mode, "--VISUAL--");
    assert_eq!(ed.selected(), "b");
    ed.keys("e");
    assert_eq!(ed.selected(), "bar");
    ed.keys("o");
    ed.keys("b");
    assert_eq!(ed.selected(), "foo bar");
    ed.keys("d");
    assert_eq!(ed.show(), "| baz");
    assert_eq!(ed.vim.mode(), Mode::Normal);
    assert_eq!(run("foo |bar baz", "vey$p"), "foo bar bazba|r");
    assert_eq!(run("foo |bar baz", "vecx<Esc>"), "foo |x baz");
    assert_eq!(run("foo |bar baz", "viwd"), "foo | baz");
    assert_eq!(run("foo |bar baz", "vawd"), "foo |baz");
    assert_eq!(run("foo |bar baz", "vjd"), "foo |ar baz");
}

#[test]
fn visual_selects_lines() {
    let mut ed = Ed::new("one\nt|wo\nthree\nfour");
    ed.keys("V");
    assert_eq!(ed.vim.status().mode, "--VISUAL LINE--");
    assert_eq!(ed.selected(), "two\n");
    ed.keys("j");
    assert_eq!(ed.selected(), "two\nthree\n");
    ed.keys("d");
    assert_eq!(ed.show(), "one\n|four");
    assert_eq!(run("one\nt|wo\nthree", "Vjy"), "one\nt|wo\nthree");
    assert_eq!(
        run("one\nt|wo\nthree", "VjyGp"),
        "one\ntwo\nthree\n|two\nthree"
    );
    assert_eq!(run("one\nt|wo\nthree", "Vcx<Esc>"), "one\n|x\nthree");
    // `v` and `V` switch kinds; the same key again leaves.
    let mut ed = Ed::new("a|b\ncd");
    ed.keys("vV");
    assert_eq!(ed.vim.mode(), Mode::VisualLine);
    ed.keys("v");
    assert_eq!(ed.vim.mode(), Mode::Visual);
    ed.keys("v");
    assert_eq!(ed.vim.mode(), Mode::Normal);
    // Escape keeps the cursor where the selection's head was.
    assert_eq!(run("|abc", "vll<Esc>"), "ab|c");
}

#[test]
fn visual_puts_replace_the_selection() {
    assert_eq!(run("|foo bar", "yiwwviwp"), "foo fo|o");
    // The replaced text is what `p` puts next.
    assert_eq!(run("|foo bar", "yiwwviwp0viwp"), "ba|r foo");
    assert_eq!(run("|one\ntwo", "yyjVp"), "one\n|one");
    assert_eq!(run("|one\nab", "yyjlvp"), "one\na\n|one\n");
}

#[test]
fn reselects() {
    let mut ed = Ed::new("|abc def");
    ed.keys("ve<Esc>w");
    ed.keys("gv");
    assert_eq!(ed.selected(), "abc");
}

#[test]
fn visual_insert_and_append() {
    assert_eq!(run("foo |bar baz", "veIx<Esc>"), "foo |xbar baz");
    assert_eq!(run("foo |bar baz", "veAx<Esc>"), "foo bar|x baz");
    assert_eq!(run("  foo\n  |bar", "VIx<Esc>"), "  foo\n  |xbar");
}

// MARK: Insert mode

#[test]
fn inserts() {
    assert_eq!(run("a|bc", "ix<Esc>"), "a|xbc");
    assert_eq!(run("a|bc", "ax<Esc>"), "ab|xc");
    assert_eq!(run("  a|bc", "Ix<Esc>"), "  |xabc");
    assert_eq!(run("a|bc", "Ax<Esc>"), "abc|x");
    assert_eq!(run("  a|bc\nd", "ox<Esc>"), "  abc\n  |x\nd");
    assert_eq!(run("  a|bc\nd", "Ox<Esc>"), "  |x\n  abc\nd");
    assert_eq!(run("a|bc", "3ix<Esc>"), "axx|xbc");
    assert_eq!(run("a|b", "2oxy<Esc>"), "ab\nxy\nx|y");
    // `a` on an empty line inserts there.
    assert_eq!(run("|", "ax<Esc>"), "|x");
    // Escape steps back onto the last character, not past a line's start.
    assert_eq!(run("|abc", "i<Esc>"), "|abc");
    assert_eq!(run("a|bc", "i<CR><Esc>"), "a\n|bc");
}

#[test]
fn insert_mode_keys_go_to_the_editor() {
    let mut vim = Vim::new();
    let mut none = || None;
    let mut cx = Context {
        text: "ab",
        cursor: 0,
        visible_lines: 0..0,
        indent: "    ",
        tab_size: 4,
        clipboard: &mut none,
    };
    assert!(vim.key(Key::Char('i'), &mut cx).handled);
    assert!(!vim.takes_keys());
    assert!(!vim.key(Key::Char('x'), &mut cx).handled);
    assert!(!vim.key(Key::Enter, &mut cx).handled);
    assert!(vim.key(Key::Escape, &mut cx).handled);
    assert!(vim.takes_keys());
}

// MARK: Repeat, undo, redo

#[test]
fn repeats_the_last_change() {
    assert_eq!(run("|a b c d", "dw."), "|c d");
    assert_eq!(run("|a b c d", "dw2."), "|d");
    assert_eq!(run("|abcdef", "x.."), "|def");
    assert_eq!(run("|abcdef", "2x."), "|ef");
    assert_eq!(run("|foo bar", "ciwx<Esc>w."), "x |x");
    assert_eq!(run("|a\nb", "A;<Esc>j."), "a;\nb|;");
    assert_eq!(run("a|b\ncd", ">>j."), "    ab\n    |cd");
    assert_eq!(run("|ab\ncd", "ix<Esc>j0."), "xab\n|xcd");
    assert_eq!(run("|ab\ncd", "3ix<Esc>j0."), "xxxab\nxx|xcd");
    assert_eq!(run("|ab\ncd", "oz<Esc>."), "ab\nz\n|z\ncd");
    assert_eq!(run("|a,b,c", "rxl.l."), "xx|x,c");
    // Yanks and moves aren't changes.
    assert_eq!(run("|a b c d", "dwyw."), "|c d");
    // Visual changes repeat on the same amount of text.
    assert_eq!(run("|abcdef", "vld."), "|ef");
    assert_eq!(run("|a\nb\nc\nd\ne", "Vjd."), "|e");
    // A change typed with Backspace in it.
    assert_eq!(run("|abc\nabc", "A<BS>x<Esc>j0."), "abx\nab|x");
}

#[test]
fn undoes_and_redoes() {
    let mut ed = Ed::new("|a b c");
    ed.keys("dwdw");
    assert_eq!(ed.show(), "|c");
    ed.keys("u");
    assert_eq!(ed.show(), "|b c");
    ed.keys("u");
    assert_eq!(ed.show(), "|a b c");
    ed.keys("<C-r>");
    assert_eq!(ed.show(), "|b c");
    ed.keys("u2<C-r>");
    assert_eq!(ed.show(), "|c");
    ed.keys("2u");
    assert_eq!(ed.show(), "|a b c");
}

// MARK: Registers

#[test]
fn registers() {
    // Named registers, and appending to them.
    assert_eq!(run("|one\ntwo", "\"ayyj\"ap"), "one\ntwo\n|one");
    assert_eq!(run("|one\ntwo", "\"ayyj\"Ayy\"ap"), "one\ntwo\n|one\ntwo");
    // The black hole keeps the unnamed register.
    assert_eq!(run("|one\ntwo", "yyj\"_ddp"), "one\n|one");
    // `0` keeps the last yank through deletes.
    assert_eq!(run("|ab cd", "ywwdw\"0P"), "abab|  ");
    // Deleted lines shift through `1`…`9`.
    assert_eq!(run("|a\nb\nc", "dddd\"2p"), "c\n|a");
    // Small deletes go to `-`.
    assert_eq!(run("|ab", "x\"-p"), "b|a");
}

#[test]
fn clipboard_registers() {
    let mut ed = Ed::new("|one\ntwo");
    ed.keys("\"+yy");
    assert_eq!(ed.actions, vec![Action::CopyToClipboard("one\n".into())]);
    ed.clipboard = Some("pasted".into());
    ed.keys("j\"+p");
    assert_eq!(ed.show(), "one\ntpaste|dwo");
    ed.clipboard = Some("line\n".into());
    ed.keys("\"*P");
    assert_eq!(ed.show(), "one\n|line\ntpastedwo");
    // Nothing on the clipboard: nothing to put.
    ed.clipboard = None;
    ed.keys("\"+p");
    assert_eq!(ed.show(), "one\n|line\ntpastedwo");
}

// MARK: Ex commands

#[test]
fn ex_file_commands() {
    let cases = [
        (":w<CR>", Action::Save),
        (":write<CR>", Action::Save),
        (":q<CR>", Action::Close(CloseSave::Never)),
        (":q!<CR>", Action::Close(CloseSave::Never)),
        (":wq<CR>", Action::Close(CloseSave::Always)),
        (":x<CR>", Action::Close(CloseSave::IfModified)),
        (":xit<CR>", Action::Close(CloseSave::IfModified)),
    ];
    for (keys, action) in cases {
        let mut ed = Ed::new("|a");
        ed.keys(keys);
        assert_eq!(ed.actions, vec![action], "{keys}");
    }
}

#[test]
fn ex_lines_and_ranges() {
    let t = "|one\n  two\nthree\nfour";
    assert_eq!(run(t, ":2<CR>"), "one\n  |two\nthree\nfour");
    assert_eq!(run(t, ":$<CR>"), "one\n  two\nthree\n|four");
    assert_eq!(run(t, ":99<CR>"), "one\n  two\nthree\n|four");
    assert_eq!(run(t, ":2,3d<CR>"), "one\n|four");
    assert_eq!(run(t, "j:.,+1d<CR>"), "one\n|four");
    assert_eq!(run(t, "2:d<CR>"), "|three\nfour");
    assert_eq!(run(t, ":2y<CR>P"), "  |two\none\n  two\nthree\nfour");
}

#[test]
fn ex_substitutes() {
    let t = "|a a\na b\nc";
    assert_eq!(run(t, ":s/a/x/<CR>"), "|x a\na b\nc");
    assert_eq!(run(t, ":s/a/x/g<CR>"), "|x x\na b\nc");
    assert_eq!(run(t, ":%s/a/x/g<CR>"), "x x\n|x b\nc");
    assert_eq!(run(t, ":%s/a/x<CR>"), "x a\n|x b\nc");
    assert_eq!(run(t, ":%s#a#/#g<CR>"), "/ /\n|/ b\nc");
    assert_eq!(run(t, ":s/\\(a\\) \\(a\\)/\\2-\\1/<CR>"), "|a a\na b\nc");
    assert_eq!(run(t, ":s/(a) (a)/\\2-\\1/<CR>"), "|a-a\na b\nc");
    assert_eq!(run(t, ":s/a/[&]/g<CR>"), "|[a] [a]\na b\nc");
    assert_eq!(run(t, ":s/ /\\r/<CR>"), "a\n|a\na b\nc");
    assert_eq!(run(t, ":s/A/x/i<CR>"), "|x a\na b\nc");
    assert_eq!(run(t, "jVj:s/b|c/z/<CR>"), "a a\na z\n|z");
    // An empty pattern is the last search.
    assert_eq!(run(t, "/b<CR>:%s//y/<CR>"), "a a\n|a y\nc");
    let mut ed = Ed::new(t);
    ed.keys(":s/q/x/<CR>");
    assert_eq!(
        ed.vim.status().message.as_deref(),
        Some("Pattern not found: q")
    );
    ed.keys(":frobnicate<CR>");
    assert_eq!(
        ed.vim.status().message.as_deref(),
        Some("Not an editor command: frobnicate")
    );
}

#[test]
fn command_line_editing() {
    let mut ed = Ed::new("|a");
    ed.keys(":wx");
    assert_eq!(ed.vim.status().command_line.as_deref(), Some(":wx"));
    ed.keys("<BS>");
    assert_eq!(ed.vim.status().command_line.as_deref(), Some(":w"));
    ed.keys("<Esc>");
    assert_eq!(ed.vim.status().command_line, None);
    assert!(ed.actions.is_empty());
    ed.keys(":<BS>");
    assert_eq!(ed.vim.status().command_line, None);
    ed.keys("vl:");
    assert_eq!(ed.vim.status().command_line.as_deref(), Some(":'<,'>"));
}

// MARK: Status and the editor's own changes

#[test]
fn status_shows_mode_and_pending_keys() {
    let mut ed = Ed::new("|a b");
    assert_eq!(ed.vim.status().mode, "--NORMAL--");
    ed.keys("2d");
    assert_eq!(ed.vim.status().keys, "2d");
    ed.keys("<Esc>");
    assert_eq!(ed.vim.status().keys, "");
    ed.keys("\"a");
    assert_eq!(ed.vim.status().keys, "\"a");
    ed.keys("i");
    assert_eq!(ed.vim.status().mode, "--INSERT--");
    assert_eq!(ed.vim.status().keys, "");
}

#[test]
fn follows_the_editors_selection() {
    let mut ed = Ed::new("|abc\ndef");
    // A click past a line's end lands on its last character.
    ed.cursor = 3;
    ed.selection = 3..3;
    ed.sync();
    assert_eq!(ed.show(), "ab|c\ndef");
    // A drag (or Select All) starts Visual mode.
    ed.cursor = 6;
    ed.selection = 1..6;
    ed.sync();
    assert_eq!(ed.vim.mode(), Mode::Visual);
    ed.keys("d");
    assert_eq!(ed.show(), "a|f");
    // A click ends Visual mode.
    ed.keys("v");
    ed.cursor = 0;
    ed.selection = 0..0;
    ed.sync();
    assert_eq!(ed.vim.mode(), Mode::Normal);
}

#[test]
fn block_cursor_covers_the_character() {
    let mut vim = Vim::new();
    assert_eq!(vim.block_cursor("ab\n\nc", 1), Some(1..2));
    assert_eq!(vim.block_cursor("ab\n\nc", 3), None);
    assert_eq!(vim.block_cursor("é", 0), Some(0..2));
    let mut none = || None;
    let mut cx = Context {
        text: "ab",
        cursor: 0,
        visible_lines: 0..0,
        indent: "    ",
        tab_size: 4,
        clipboard: &mut none,
    };
    vim.key(Key::Char('i'), &mut cx);
    assert_eq!(vim.block_cursor("ab", 0), None);
}

#[test]
fn keeps_to_character_boundaries() {
    assert_eq!(run("|éàü", "l"), "é|àü");
    assert_eq!(run("é|àü", "x"), "é|ü");
    assert_eq!(run("|é é", "dw"), "|é");
    assert_eq!(run("|é", "rü"), "|ü");
    assert_eq!(run("|ab", "~"), "A|b");
}

#[test]
fn notation() {
    assert_eq!(
        Key::parse_all("d<C-r><Esc><CR>x<lt>"),
        vec![
            Key::Char('d'),
            Key::Ctrl('r'),
            Key::Escape,
            Key::Enter,
            Key::Char('x'),
            Key::Char('<'),
        ]
    );
}

#[test]
fn marks() {
    let t = "one\n  t|wo\nthree\nfour";
    // `'a` goes to the line's first non-blank, `` `a `` to the place.
    assert_eq!(run(t, "majjj'a"), "one\n  |two\nthree\nfour");
    assert_eq!(run(t, "majjj`a"), "one\n  t|wo\nthree\nfour");
    // As targets: `'` takes whole lines, `` ` `` up to the place.
    assert_eq!(run(t, "majjd'a"), "|one");
    assert_eq!(run("ab|cdef", "mal`a"), "ab|cdef");
    assert_eq!(run("ab|cdef", "ma$d`a"), "ab|f");
    // `''` returns to where the last jump started.
    assert_eq!(run(t, "G''"), "one\n  |two\nthree\nfour");
    // An unset mark goes nowhere.
    assert_eq!(run(t, "'z"), t);
}

#[test]
fn macros() {
    // Recorded keys, Insert mode's typing included, played with `@`.
    let t = "|a\nb\nc\nd";
    assert_eq!(run(t, "qaA!<Esc>jq@a"), "a!\nb!\n|c\nd");
    assert_eq!(run(t, "qaA!<Esc>jq2@a"), "a!\nb!\nc!\n|d");
    assert_eq!(run(t, "qaA!<Esc>jq@a@@"), "a!\nb!\nc!\n|d");
    // A macro with an operator and a count.
    assert_eq!(run("|one two three", "qqdwq@q"), "|three");
    // An unknown macro does nothing.
    assert_eq!(run(t, "@z"), t);
    let mut ed = Ed::new(t);
    ed.keys("qa");
    assert_eq!(ed.vim.recording(), Some('a'));
    ed.keys("q");
    assert_eq!(ed.vim.recording(), None);
}

#[test]
fn incremental_search() {
    let t = "|foo bar\nbaz bar\nbar";
    let mut ed = Ed::new(t);
    // Each key shows the first match from where the search started.
    ed.keys("/ba");
    assert_eq!(ed.show(), "foo |bar\nbaz bar\nbar");
    assert_eq!(ed.vim.highlighted_pattern(), Some("ba"));
    ed.keys("z");
    assert_eq!(ed.show(), "foo bar\n|baz bar\nbar");
    ed.keys("<BS>");
    assert_eq!(ed.show(), "foo |bar\nbaz bar\nbar");
    // Enter lands on the same match; Escape goes back.
    ed.keys("z<CR>");
    assert_eq!(ed.show(), "foo bar\n|baz bar\nbar");
    let mut ed = Ed::new(t);
    ed.keys("/baz<Esc>");
    assert_eq!(ed.show(), t);
    assert_eq!(ed.vim.highlighted_pattern(), None);
    // Backwards from the end.
    assert_eq!(
        run("foo bar\nbaz bar\n|bar", "?baz<CR>"),
        "foo bar\n|baz bar\nbar"
    );
}
