import AppKit

// MARK: - Icon Color Field

/// Which theme color an icon uses, mirroring the Rust `ColorField` enum.
enum IconColorField {
    case orange, blue, yellow, green, cyan, magenta, red, comment, fg

    func resolve(_ theme: Theme) -> String {
        switch self {
        case .orange:  return theme.orangeHex
        case .blue:    return theme.blueHex
        case .yellow:  return theme.yellowHex
        case .green:   return theme.greenHex
        case .cyan:    return theme.cyanHex
        case .magenta: return theme.magentaHex
        case .red:     return theme.redHex
        case .comment: return theme.commentHex
        case .fg:      return theme.fgHex
        }
    }
}

// MARK: - Icon Definition

private struct IconDef {
    let name: String
    let color: IconColorField
}

/// All icon definitions, matching `file_icons.rs` exactly.
private let allIcons: [IconDef] = [
    // Languages
    IconDef(name: "rust", color: .orange),
    IconDef(name: "python", color: .blue),
    IconDef(name: "javascript", color: .yellow),
    IconDef(name: "typescript", color: .blue),
    IconDef(name: "go", color: .cyan),
    IconDef(name: "c", color: .blue),
    IconDef(name: "cpp", color: .blue),
    IconDef(name: "java", color: .orange),
    IconDef(name: "kotlin", color: .magenta),
    IconDef(name: "swift", color: .orange),
    IconDef(name: "ruby", color: .red),
    IconDef(name: "php", color: .magenta),
    IconDef(name: "csharp", color: .green),
    IconDef(name: "zig", color: .orange),
    IconDef(name: "haskell", color: .magenta),
    IconDef(name: "lua", color: .blue),
    IconDef(name: "dart", color: .cyan),
    IconDef(name: "elixir", color: .magenta),
    IconDef(name: "scala", color: .magenta),
    IconDef(name: "clojure", color: .green),
    IconDef(name: "erlang", color: .red),
    IconDef(name: "nim", color: .yellow),
    IconDef(name: "julia", color: .magenta),
    IconDef(name: "r", color: .blue),
    IconDef(name: "tex", color: .orange),
    // Web
    IconDef(name: "html", color: .orange),
    IconDef(name: "css", color: .blue),
    IconDef(name: "sass", color: .magenta),
    IconDef(name: "vue", color: .green),
    IconDef(name: "svelte", color: .orange),
    IconDef(name: "react", color: .cyan),
    // Data / Config
    IconDef(name: "json", color: .yellow),
    IconDef(name: "yaml", color: .orange),
    IconDef(name: "toml", color: .orange),
    IconDef(name: "xml", color: .comment),
    IconDef(name: "markdown", color: .blue),
    IconDef(name: "settings", color: .comment),
    // Tooling
    IconDef(name: "console", color: .green),
    IconDef(name: "docker", color: .cyan),
    IconDef(name: "git", color: .orange),
    IconDef(name: "lock", color: .comment),
    IconDef(name: "database", color: .cyan),
    // Media
    IconDef(name: "image", color: .green),
    IconDef(name: "audio", color: .magenta),
    IconDef(name: "video", color: .magenta),
    IconDef(name: "pdf", color: .red),
    // General
    IconDef(name: "document", color: .fg),
    IconDef(name: "archive", color: .comment),
    IconDef(name: "binary", color: .comment),
    // Folders
    IconDef(name: "folder", color: .cyan),
    IconDef(name: "folder-open", color: .cyan),
    // Toolbar
    IconDef(name: "toolbar-sidebar", color: .fg),
    IconDef(name: "toolbar-plus", color: .fg),
    IconDef(name: "toolbar-eye-open", color: .fg),
    IconDef(name: "toolbar-eye-closed", color: .fg),
    IconDef(name: "toolbar-collapse", color: .fg),
    IconDef(name: "toolbar-refresh", color: .fg),
]

// MARK: - Icon Name Lookup

/// Maps a filename to its icon name, exactly mirroring the Rust `lookup_icon_name`.
func lookupIconName(filename: String, isDirectory: Bool, expanded: Bool) -> String {
    if isDirectory {
        return expanded ? "folder-open" : "folder"
    }

    let ext = filename.split(separator: ".").last.map(String.init)?.lowercased() ?? ""
    switch ext {
    // Languages
    case "rs": return "rust"
    case "py", "pyi", "pyw": return "python"
    case "js", "mjs", "cjs": return "javascript"
    case "ts", "mts", "cts": return "typescript"
    case "go": return "go"
    case "c", "h": return "c"
    case "cpp", "cc", "cxx", "hpp", "hxx", "hh": return "cpp"
    case "java": return "java"
    case "kt", "kts": return "kotlin"
    case "swift": return "swift"
    case "rb", "erb": return "ruby"
    case "php": return "php"
    case "cs": return "csharp"
    case "zig": return "zig"
    case "hs", "lhs": return "haskell"
    case "lua": return "lua"
    case "dart": return "dart"
    case "ex", "exs", "heex": return "elixir"
    case "scala", "sc": return "scala"
    case "clj", "cljs", "cljc", "edn": return "clojure"
    case "erl", "hrl": return "erlang"
    case "nim", "nims": return "nim"
    case "jl": return "julia"
    case "r", "rmd": return "r"
    case "tex", "sty", "cls", "bib": return "tex"
    // Web
    case "html", "htm": return "html"
    case "css": return "css"
    case "scss", "sass", "less": return "sass"
    case "vue": return "vue"
    case "svelte": return "svelte"
    case "jsx", "tsx": return "react"
    // Data / Config
    case "json", "jsonc", "json5": return "json"
    case "yaml", "yml": return "yaml"
    case "toml": return "toml"
    case "xml", "xsl", "xslt", "xsd", "wsdl": return "xml"
    case "md", "mdx", "markdown": return "markdown"
    case "ini", "cfg", "conf", "ron", "properties": return "settings"
    // Shell / Tooling
    case "sh", "bash", "zsh", "fish", "ps1", "bat", "cmd": return "console"
    case "lock": return "lock"
    case "sql", "sqlite", "db": return "database"
    // Media
    case "png", "jpg", "jpeg", "gif", "svg", "ico", "webp", "bmp", "tiff", "avif": return "image"
    case "mp3", "wav", "flac", "ogg", "aac", "wma", "m4a": return "audio"
    case "mp4", "mkv", "avi", "webm", "mov", "wmv", "flv": return "video"
    case "pdf": return "pdf"
    // Archives
    case "zip", "tar", "gz", "bz2", "xz", "7z", "rar", "zst", "tgz": return "archive"
    // Binary / Executables
    case "exe", "dll", "so", "dylib", "a", "o", "wasm": return "binary"
    default: return lookupByFilename(filename)
    }
}

/// Fallback lookup by full filename for special files.
private func lookupByFilename(_ filename: String) -> String {
    switch filename.lowercased() {
    case "dockerfile", "containerfile": return "docker"
    case "makefile", "rakefile", "justfile", "taskfile": return "console"
    case ".gitignore", ".gitmodules", ".gitattributes": return "git"
    case "license", "licence", "license.md", "licence.md", "license.txt", "licence.txt": return "document"
    case "readme", "readme.md", "readme.txt": return "document"
    case "changelog", "changelog.md", "authors", "contributing", "contributing.md": return "document"
    case "cargo.toml", "cargo.lock": return "rust"
    case "package.json", "package-lock.json": return "javascript"
    case "tsconfig.json": return "typescript"
    case "go.mod", "go.sum": return "go"
    case "gemfile", "gemfile.lock": return "ruby"
    case "composer.json", "composer.lock": return "php"
    case ".eslintrc", ".prettierrc", ".editorconfig": return "settings"
    default: return "document"
    }
}

// MARK: - SVG Recoloring

/// Replaces hex fill/stroke attribute values in an SVG string with a theme color.
private func recolorSVG(_ svg: String, color: String) -> String {
    let colorValue = color.hasPrefix("#") ? color : "#\(color)"
    var result = svg

    // Replace fill="#XXXXXX" and stroke="#XXXXXX" with the theme color
    if let regex = try? NSRegularExpression(pattern: ##"(fill|stroke)="#[0-9A-Fa-f]{3,8}""##) {
        result = regex.stringByReplacingMatches(
            in: result,
            range: NSRange(result.startIndex..., in: result),
            withTemplate: "$1=\"\(colorValue)\""
        )
    }
    return result
}

// MARK: - Icon Cache

/// Pre-renders all SVG icons for the current theme and caches the resulting NSImages.
final class IconCache {
    private var images: [String: NSImage] = [:]
    private var svgStrings: [String: String] = [:]
    private var iconDefs: [String: IconDef] = [:]

    init(theme: Theme) {
        // Index icon defs by name
        for def in allIcons {
            iconDefs[def.name] = def
        }
        loadSVGs()
        build(theme: theme)
    }

    /// Loads raw SVG strings from the bundle resource directory.
    private func loadSVGs() {
        guard let iconsURL = Bundle.module.url(forResource: "icons", withExtension: nil) else {
            NSLog("IconCache: icons resource directory not found in bundle")
            return
        }

        for def in allIcons {
            let svgURL = iconsURL.appendingPathComponent("\(def.name).svg")
            if let svgString = try? String(contentsOf: svgURL, encoding: .utf8) {
                svgStrings[def.name] = svgString
            }
        }
    }

    /// Rebuilds the cache for a new theme.
    func rebuild(theme: Theme) {
        images.removeAll()
        build(theme: theme)
    }

    private func build(theme: Theme) {
        for def in allIcons {
            guard let svg = svgStrings[def.name] else { continue }
            let color = def.color.resolve(theme)
            let recolored = recolorSVG(svg, color: color)
            if let image = renderSVG(recolored, size: 16) {
                images[def.name] = image
            }
        }
    }

    /// Renders an SVG string to an NSImage at the given point size.
    private func renderSVG(_ svg: String, size: CGFloat) -> NSImage? {
        guard let data = svg.data(using: .utf8) else { return nil }
        guard let image = NSImage(data: data) else { return nil }
        image.size = NSSize(width: size, height: size)
        return image
    }

    /// Returns the themed icon for a file or directory.
    func icon(filename: String, isDirectory: Bool, expanded: Bool) -> NSImage? {
        let name = lookupIconName(filename: filename, isDirectory: isDirectory, expanded: expanded)
        return images[name]
    }

    /// Returns a toolbar icon by name (e.g. "toolbar-sidebar", "console", "settings").
    func toolbarIcon(name: String) -> NSImage? {
        return images[name]
    }
}
