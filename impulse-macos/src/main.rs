mod app;
mod editor;
mod window;

fn main() {
    env_logger::init();
    log::info!("Impulse macOS starting...");

    // On macOS, this will:
    // 1. Create an NSApplication
    // 2. Set up the app delegate
    // 3. Create the main window with sidebar, editor, and terminal areas
    // 4. Run the event loop
    //
    // For now, this is a skeleton that prints a message.
    // Build on macOS with: cargo run -p impulse-macos

    println!("Impulse macOS frontend");
    println!("This is a skeleton — build on macOS to use native AppKit integration.");
    println!();
    println!("Architecture:");
    println!("  impulse-core    — Shared backend (LSP, PTY, search, filesystem)");
    println!("  impulse-editor  — Shared Monaco editor (protocol + web assets)");
    println!("  impulse-macos   — Native macOS frontend (this crate)");
    println!();
    println!("The editor uses Monaco embedded in a WKWebView,");
    println!("sharing the same HTML/JS as the Linux (WebKitGTK) frontend.");
}
