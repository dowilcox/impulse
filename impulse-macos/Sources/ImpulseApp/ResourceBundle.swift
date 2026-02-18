import Foundation

extension Bundle {
    /// Locates the SwiftPM resource bundle in both .app and development contexts.
    ///
    /// The auto-generated `Bundle.module` uses `Bundle.main.bundleURL` which
    /// points to the .app root for packaged apps — but codesign rejects bundles
    /// placed there ("unsealed contents"). This accessor checks the standard
    /// macOS locations instead.
    static let appResources: Bundle = {
        let bundleName = "ImpulseApp_ImpulseApp"

        // .app bundle: Contents/Resources/
        if let url = Bundle.main.resourceURL?
            .appendingPathComponent("\(bundleName).bundle"),
           let bundle = Bundle(url: url) {
            return bundle
        }

        // Development: SwiftPM places the bundle next to the binary
        let devURL = Bundle.main.bundleURL
            .appendingPathComponent("\(bundleName).bundle")
        if let bundle = Bundle(url: devURL) {
            return bundle
        }

        fatalError("Could not find resource bundle \(bundleName)")
    }()
}
