import Foundation

// MARK: - Review Commands (Swift -> review.js)

/// Commands sent from Swift to the Review Changes WebView via
/// `window.__applyReviewCommand(<json>)`.
///
/// JSON encoding uses a tagged union format with a "type" field and snake_case
/// keys, matching the Rust `ReviewCommand` serde output from impulse-editor.
enum ReviewCommand: Encodable {
    case render(files: [ReviewFileEntry])
    case setHunks(path: String, hunks: ImpulseCore.FileHunks)
    case setTheme(theme: MonacoThemeDefinition)

    // MARK: Tagged Enum Encoding

    private enum TypeTag: String, Encodable {
        case render = "Render"
        case setHunks = "SetHunks"
        case setTheme = "SetTheme"
    }

    private enum CodingKeys: String, CodingKey {
        case type
        case files
        case path
        case hunks
        case theme
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)

        switch self {
        case let .render(files):
            try container.encode(TypeTag.render, forKey: .type)
            try container.encode(files, forKey: .files)

        case let .setHunks(path, hunks):
            try container.encode(TypeTag.setHunks, forKey: .type)
            try container.encode(path, forKey: .path)
            try container.encode(hunks, forKey: .hunks)

        case let .setTheme(theme):
            try container.encode(TypeTag.setTheme, forKey: .type)
            try container.encode(theme, forKey: .theme)
        }
    }
}

/// A single changed file in the Render command, mirroring the Rust
/// `ReviewFileEntry` struct (snake_case keys).
struct ReviewFileEntry: Encodable {
    let path: String
    /// Status letter: "A" | "M" | "D" | "R" | "?".
    let status: String
    let oldPath: String?
    let added: UInt32
    let removed: UInt32
    let isBinary: Bool

    enum CodingKeys: String, CodingKey {
        case path, status, added, removed
        case oldPath = "old_path"
        case isBinary = "is_binary"
    }
}

// MARK: - Review Events (review.js -> Swift)

/// Events received from the Review Changes WebView via WKScriptMessageHandler
/// (handler name "impulseReview").
///
/// JSON decoding expects the same tagged union format with a "type" field
/// produced by the review.js layer.
enum ReviewEvent: Decodable {
    case ready
    case requestDiff(path: String)
    case discard(path: String)
    case toggleFile(path: String, expanded: Bool)
    case refresh

    private enum TypeTag: String, Decodable {
        case ready = "Ready"
        case requestDiff = "RequestDiff"
        case discard = "Discard"
        case toggleFile = "ToggleFile"
        case refresh = "Refresh"
    }

    private enum CodingKeys: String, CodingKey {
        case type
        case path
        case expanded
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let tag = try container.decode(TypeTag.self, forKey: .type)

        switch tag {
        case .ready:
            self = .ready

        case .requestDiff:
            let path = try container.decode(String.self, forKey: .path)
            self = .requestDiff(path: path)

        case .discard:
            let path = try container.decode(String.self, forKey: .path)
            self = .discard(path: path)

        case .toggleFile:
            let path = try container.decode(String.self, forKey: .path)
            let expanded = try container.decode(Bool.self, forKey: .expanded)
            self = .toggleFile(path: path, expanded: expanded)

        case .refresh:
            self = .refresh
        }
    }
}
