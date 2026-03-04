/// Global application state flags set once at startup.
enum AppState {
    /// Whether the app was launched with `--dev` for side-by-side development.
    static var isDev: Bool = false
}
