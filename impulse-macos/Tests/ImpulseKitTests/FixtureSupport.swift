import Foundation

/// Loads golden parity fixtures generated from the Rust implementation by
/// `cargo run -p impulse-editor --example dump_fixtures`.
enum Fixtures {
    static var root: URL {
        guard let url = Bundle.module.url(forResource: "Fixtures", withExtension: nil) else {
            fatalError("Fixtures directory missing from test bundle")
        }
        return url
    }

    static func data(_ relativePath: String) throws -> Data {
        try Data(contentsOf: root.appendingPathComponent(relativePath))
    }

    static func json(_ relativePath: String) throws -> Any {
        try JSONSerialization.jsonObject(with: data(relativePath), options: [.fragmentsAllowed])
    }

    static func decode<T: Decodable>(_ type: T.Type, from relativePath: String) throws -> T {
        try JSONDecoder().decode(type, from: data(relativePath))
    }

    /// All fixture JSON files, as paths relative to the Fixtures root.
    static func allJSONPaths() throws -> [String] {
        let fm = FileManager.default
        guard let enumerator = fm.enumerator(at: root, includingPropertiesForKeys: nil) else {
            return []
        }
        var paths: [String] = []
        for case let url as URL in enumerator where url.pathExtension == "json" {
            let rel = url.path.replacingOccurrences(of: root.path + "/", with: "")
            paths.append(rel)
        }
        return paths.sorted()
    }
}
