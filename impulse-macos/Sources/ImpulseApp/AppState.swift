/// Global application state flags set once at startup.
enum AppState {
    /// Whether the app was launched with `--dev` for side-by-side development.
    static var isDev: Bool = false

    /// False while running a headless debug snapshot: settings and session
    /// state must not be written (see `DebugSnapshot`).
    static var persistenceEnabled: Bool = true
}
