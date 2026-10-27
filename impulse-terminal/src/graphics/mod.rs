//! Images programs show, by the kitty graphics protocol
//! (<https://sw.kovidgoyal.net/kitty/graphics-protocol/>): APC sequences
//! the scanner picks out, kept per terminal, placed in the grid directly
//! or shown by Unicode placeholder cells.

mod command;
mod decode;
mod placeholder;
mod store;

pub(crate) use command::Command;
pub(crate) use placeholder::{color_id, is_diacritic, PlaceholderRow};
pub use placeholder::{strip_placeholders, PLACEHOLDER};
pub(crate) use store::{reply, Cursor, ImageStore};
pub use store::{ImageLayer, ImagePlacement, TerminalImage};
