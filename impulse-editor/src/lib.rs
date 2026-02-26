pub mod assets;
pub mod markdown;
pub mod protocol;
pub mod svg;

/// Check whether a file path is a previewable type (markdown or SVG).
pub fn is_previewable_file(path: &str) -> bool {
    markdown::is_markdown_file(path) || svg::is_svg_file(path)
}
