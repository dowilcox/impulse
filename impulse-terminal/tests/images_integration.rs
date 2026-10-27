//! Images through a real PTY: a script prints kitty graphics commands and
//! Unicode placeholder cells, and the backend's image layer, decoded
//! pixels and grid snapshot are checked.

use std::io::Write;
use std::time::{Duration, Instant};

use impulse_terminal::{
    TerminalBackend, TerminalConfig, CELL_STRIDE, EXTRA_IMAGE_CELL, FIXED_HEADER_SIZE,
    RANGE_ENTRY_SIZE,
};

fn write_script(body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::temp_dir().join(format!("impulse_images_test_{}.sh", std::process::id()));
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(body.as_bytes()).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for {what}");
}

/// The image cells in a grid snapshot: cell index, image id, placement
/// id, row and column in the image.
fn image_cells(buf: &[u8]) -> Vec<(usize, u32, u32, u16, u16)> {
    let u16_at = |at: usize| u16::from_le_bytes([buf[at], buf[at + 1]]) as usize;
    let u32_at = |at: usize| u32::from_le_bytes(buf[at..at + 4].try_into().unwrap());
    let (cols, rows) = (u16_at(0), u16_at(2));
    let ranges = u16_at(12) + u16_at(14);
    let mut at = FIXED_HEADER_SIZE + ranges * RANGE_ENTRY_SIZE + cols * rows * CELL_STRIDE;
    let count = u32_at(at);
    at += 4;
    let mut cells = Vec::new();
    for _ in 0..count {
        let (index, kind, len) = (u32_at(at) as usize, buf[at + 4], buf[at + 5] as usize);
        let payload = &buf[at + 6..at + 6 + len];
        if kind == EXTRA_IMAGE_CELL {
            cells.push((
                index,
                u32::from_le_bytes(payload[0..4].try_into().unwrap()),
                u32::from_le_bytes(payload[4..8].try_into().unwrap()),
                u16::from_le_bytes([payload[8], payload[9]]),
                u16::from_le_bytes([payload[10], payload[11]]),
            ));
        }
        at += 6 + len;
    }
    cells
}

#[test]
fn a_program_shows_images_both_ways() {
    // A 2×2 RGB image placed after "ab", and image 9 shown by two
    // placeholder cells (foreground color 9, row 0 and columns 0 and 1 by
    // their marks) after a virtual placement two cells wide.
    let red = "/wAA/wAA/wAA/wAA"; // four red pixels
    let placeholders = "\u{10EEEE}\u{0305}\u{0305}\u{10EEEE}\u{0305}\u{030D}";
    let body = format!(
        "#!/bin/bash\n\
         printf 'ab\\033_Ga=T,f=24,s=2,v=2,i=7;{red}\\033\\\\'\n\
         printf '\\033_Ga=t,f=24,s=2,v=2,i=9,q=2;{red}\\033\\\\'\n\
         printf '\\033_Ga=p,U=1,i=9,c=2,r=1,q=2\\033\\\\'\n\
         printf '\\r\\n\\033[38;5;9m%s\\033[0m\\r\\n' '{placeholders}'\n\
         sleep 4\n"
    );
    let script = write_script(&body);
    let config = TerminalConfig {
        shell_path: script.to_string_lossy().to_string(),
        ..TerminalConfig::default()
    };
    let backend = TerminalBackend::new(config, 80, 24, 8, 16).expect("spawn backend");

    wait_for("the placement", || {
        !backend.image_layer().placements.is_empty()
    });
    wait_for("the pixels", || {
        backend.image(7).is_some() && backend.image(9).is_some()
    });

    let layer = backend.image_layer();
    let placement = &layer.placements[0];
    assert_eq!(
        (placement.image_id, placement.row, placement.col),
        (7, 0, 2)
    );
    // 2×2 pixels in an 8×16 cell.
    assert_eq!((placement.width, placement.height), (0.25, 0.125));
    assert_eq!(layer.virtuals.get(&(9, 0)), Some(&(2, 1)));
    let image = backend.image(7).unwrap();
    assert_eq!(&image.rgba[..4], &[255, 0, 0, 255]);

    let mut buf = vec![0; backend.grid_buffer_size()];
    let written = backend.write_grid_to_buffer(&mut buf);
    assert_eq!(
        image_cells(&buf[..written]),
        [(80, 9, 0, 0, 0), (81, 9, 0, 0, 1)],
        "two placeholder cells on the second row"
    );
    // The transcript (and so session restore) has no placeholders in it.
    assert!(!backend.transcript(100, false).contains('\u{10EEEE}'));

    backend.shutdown();
    let _ = std::fs::remove_file(&script);
}
