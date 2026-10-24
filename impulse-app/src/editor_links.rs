//! Monaco's detected links: the `http`, `https` and `file` URLs in the
//! text are underlined (`.detected-link`), and Cmd+click (Ctrl+click on
//! Linux) opens them, through GPUI Kit's go-to-definition, which opens a
//! web target with the system. Other positions go to the language server.

use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::input::{DefinitionProvider, RopeExt as _, TextDecoration};
use gpui_kit::{px, App, HighlightStyle, Task, UnderlineStyle, Window};
use lsp_types::LocationLink;

use crate::lsp_hub::DocumentLink;

/// The links in `text`, by line (`impulse_kit::terminal_hints::urls`).
pub(crate) fn links(text: &str) -> Vec<Range<usize>> {
    let mut start = 0;
    let mut out = Vec::new();
    for line in text.split('\n') {
        out.extend(
            impulse_kit::terminal_hints::urls(line)
                .into_iter()
                .map(|r| start + r.start..start + r.end),
        );
        start += line.len() + 1;
    }
    out
}

/// The underline under each link.
pub(crate) fn underlines(text: &str) -> Vec<TextDecoration> {
    links(text)
        .into_iter()
        .map(|range| {
            TextDecoration::new(
                range,
                HighlightStyle {
                    underline: Some(UnderlineStyle {
                        thickness: px(1.),
                        color: None,
                        wavy: false,
                    }),
                    ..Default::default()
                },
            )
        })
        .collect()
}

/// Go to definition with links first: a URL under the pointer is its own
/// target; anywhere else asks the language server, when there is one.
pub(crate) struct LinkDefinitions {
    pub server: Option<Rc<DocumentLink>>,
}

impl DefinitionProvider for LinkDefinitions {
    fn definitions(
        &self,
        text: &gpui_kit::component::input::Rope,
        offset: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<gpui_kit::Result<Vec<LocationLink>>> {
        let row = text.offset_to_point(offset).row;
        let start = text.line_start_offset(row);
        let line = text.slice_line(row).to_string();
        let at = offset - start;
        let found = impulse_kit::terminal_hints::urls(&line)
            .into_iter()
            .find(|r| r.start <= at && at < r.end);
        if let Some(range) = found {
            let link = line[range.clone()]
                .parse::<lsp_types::Uri>()
                .ok()
                .map(|uri| {
                    let origin = lsp_types::Range::new(
                        text.offset_to_position(start + range.start),
                        text.offset_to_position(start + range.end),
                    );
                    LocationLink {
                        origin_selection_range: Some(origin),
                        target_uri: uri,
                        target_range: Default::default(),
                        target_selection_range: Default::default(),
                    }
                });
            return Task::ready(Ok(link.into_iter().collect()));
        }
        match &self.server {
            Some(server) => server.definitions(text, offset, window, cx),
            None => Task::ready(Ok(Vec::new())),
        }
    }
}
