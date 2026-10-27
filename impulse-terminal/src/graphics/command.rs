//! A graphics command: the `key=value` pairs before the `;`, and the base64
//! payload after it.

/// One kitty graphics command, with the protocol's defaults for the keys
/// it left out.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Command {
    /// `a`: t (transmit, the default), T (transmit and place), q (query),
    /// p (place), d (delete), f, a and c (animation).
    pub action: u8,
    /// `q`: 1 keeps OK answers back, 2 every answer.
    pub quiet: u32,
    /// `i`: the image's id, chosen by the program.
    pub id: u32,
    /// `I`: the image's number; the terminal picks the id.
    pub number: u32,
    /// `p`: the placement's id.
    pub placement: u32,
    /// `f`: 24 (RGB), 32 (RGBA, the default) or 100 (PNG).
    pub format: u32,
    /// `t`: d (in the sequence, the default), f (a file), t (a temporary
    /// file) or s (shared memory).
    pub medium: u8,
    /// `o`: z for zlib, 0 for none.
    pub compression: u8,
    /// `s` and `v`: a raw image's width and height in pixels.
    pub width: u32,
    pub height: u32,
    /// `S` and `O`: how much of a file to read, and from where.
    pub size: u32,
    pub offset: u32,
    /// `m=1`: more chunks follow.
    pub more: bool,
    /// `x`, `y`, `w`, `h`: the part of the image to show (a delete's `x`
    /// and `y` are a cell or a range).
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// `X` and `Y`: where in its first cell the image starts, in pixels.
    pub cell_x: u32,
    pub cell_y: u32,
    /// `c` and `r`: the cells the image is scaled to.
    pub cols: u32,
    pub rows: u32,
    /// `C=1`: the cursor stays where it is.
    pub keep_cursor: bool,
    /// `U=1`: a virtual placement, shown by Unicode placeholder cells.
    pub virtual_placement: bool,
    /// `z`: under the text when negative.
    pub z: i32,
    /// `d`: what a delete removes.
    pub delete: u8,
    /// Only `m` and `q` were given: the next chunk of a transmission.
    pub continuation: bool,
}

impl Default for Command {
    fn default() -> Self {
        Command {
            action: b't',
            quiet: 0,
            id: 0,
            number: 0,
            placement: 0,
            format: 32,
            medium: b'd',
            compression: 0,
            width: 0,
            height: 0,
            size: 0,
            offset: 0,
            more: false,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            cell_x: 0,
            cell_y: 0,
            cols: 0,
            rows: 0,
            keep_cursor: false,
            virtual_placement: false,
            z: 0,
            delete: 0,
            continuation: true,
        }
    }
}

impl Command {
    /// Parse an APC's content from its `G`: the command and its payload
    /// (still base64). `None` when it isn't a graphics command or a key is
    /// malformed.
    pub fn parse(apc: &[u8]) -> Option<(Command, &[u8])> {
        let body = apc.strip_prefix(b"G")?;
        let (control, payload) = match body.iter().position(|&b| b == b';') {
            Some(i) => (&body[..i], &body[i + 1..]),
            None => (body, &b""[..]),
        };
        let mut cmd = Command::default();
        for pair in control.split(|&b| b == b',').filter(|p| !p.is_empty()) {
            let (&key, value) = pair.split_first()?;
            let value = value.strip_prefix(b"=")?;
            if !matches!(key, b'm' | b'q') {
                cmd.continuation = false;
            }
            let number = || -> Option<u32> { std::str::from_utf8(value).ok()?.parse().ok() };
            let letter = || -> Option<u8> {
                match value {
                    [b] => Some(*b),
                    _ => None,
                }
            };
            match key {
                b'a' => cmd.action = letter()?,
                b'q' => cmd.quiet = number()?,
                b'i' => cmd.id = number()?,
                b'I' => cmd.number = number()?,
                b'p' => cmd.placement = number()?,
                b'f' => cmd.format = number()?,
                b't' => cmd.medium = letter()?,
                b'o' => cmd.compression = letter()?,
                b's' => cmd.width = number()?,
                b'v' => cmd.height = number()?,
                b'S' => cmd.size = number()?,
                b'O' => cmd.offset = number()?,
                b'm' => cmd.more = number()? == 1,
                b'x' => cmd.x = number()?,
                b'y' => cmd.y = number()?,
                b'w' => cmd.w = number()?,
                b'h' => cmd.h = number()?,
                b'X' => cmd.cell_x = number()?,
                b'Y' => cmd.cell_y = number()?,
                b'c' => cmd.cols = number()?,
                b'r' => cmd.rows = number()?,
                b'C' => cmd.keep_cursor = number()? == 1,
                b'U' => cmd.virtual_placement = number()? == 1,
                b'z' => cmd.z = std::str::from_utf8(value).ok()?.parse().ok()?,
                b'd' => cmd.delete = letter()?,
                // Keys Impulse doesn't act on (relative placements, frame
                // numbers) are taken and ignored.
                _ => {}
            }
        }
        Some((cmd, payload))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_keys_and_payload() {
        let (cmd, payload) = Command::parse(b"Ga=T,f=100,i=31,q=2,z=-5,C=1;iVBORw0K").unwrap();
        assert_eq!(cmd.action, b'T');
        assert_eq!(cmd.format, 100);
        assert_eq!(cmd.id, 31);
        assert_eq!(cmd.quiet, 2);
        assert_eq!(cmd.z, -5);
        assert!(cmd.keep_cursor);
        assert!(!cmd.continuation);
        assert_eq!(payload, b"iVBORw0K");
    }

    #[test]
    fn defaults_and_continuations() {
        let (cmd, payload) = Command::parse(b"G").unwrap();
        assert_eq!((cmd.action, cmd.format, cmd.medium), (b't', 32, b'd'));
        assert!(cmd.continuation);
        assert!(payload.is_empty());
        let (cmd, _) = Command::parse(b"Gm=1,q=1;AAAA").unwrap();
        assert!(cmd.continuation && cmd.more);
        let (cmd, _) = Command::parse(b"Gm=0;AAAA").unwrap();
        assert!(cmd.continuation && !cmd.more);
    }

    #[test]
    fn rejects_malformed_keys() {
        assert!(Command::parse(b"Xa=T").is_none());
        assert!(Command::parse(b"Ga").is_none());
        assert!(Command::parse(b"Gi=x").is_none());
        assert!(Command::parse(b"Ga=TT").is_none());
        assert!(Command::parse(b"Gi=99999999999").is_none());
    }
}
