import Foundation

/// Loads the golden parity fixtures generated from the Rust implementation
/// (Tests/ImpulseGitTests/Fixtures).
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
}
