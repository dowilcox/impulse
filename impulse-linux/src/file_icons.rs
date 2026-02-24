use std::collections::HashMap;

use gtk4::gdk;
use gtk4::glib;

use crate::theme::ThemeColors;

// ---------------------------------------------------------------------------
// Embedded SVGs (Material Icon Theme, MIT license)
// ---------------------------------------------------------------------------

// Languages
const RUST_SVG: &str = include_str!("../../assets/icons/rust.svg");
const PYTHON_SVG: &str = include_str!("../../assets/icons/python.svg");
const JAVASCRIPT_SVG: &str = include_str!("../../assets/icons/javascript.svg");
const TYPESCRIPT_SVG: &str = include_str!("../../assets/icons/typescript.svg");
const GO_SVG: &str = include_str!("../../assets/icons/go.svg");
const C_SVG: &str = include_str!("../../assets/icons/c.svg");
const CPP_SVG: &str = include_str!("../../assets/icons/cpp.svg");
const JAVA_SVG: &str = include_str!("../../assets/icons/java.svg");
const KOTLIN_SVG: &str = include_str!("../../assets/icons/kotlin.svg");
const SWIFT_SVG: &str = include_str!("../../assets/icons/swift.svg");
const RUBY_SVG: &str = include_str!("../../assets/icons/ruby.svg");
const PHP_SVG: &str = include_str!("../../assets/icons/php.svg");
const CSHARP_SVG: &str = include_str!("../../assets/icons/csharp.svg");
const ZIG_SVG: &str = include_str!("../../assets/icons/zig.svg");
const HASKELL_SVG: &str = include_str!("../../assets/icons/haskell.svg");
const LUA_SVG: &str = include_str!("../../assets/icons/lua.svg");
const DART_SVG: &str = include_str!("../../assets/icons/dart.svg");
const ELIXIR_SVG: &str = include_str!("../../assets/icons/elixir.svg");
const SCALA_SVG: &str = include_str!("../../assets/icons/scala.svg");
const CLOJURE_SVG: &str = include_str!("../../assets/icons/clojure.svg");
const ERLANG_SVG: &str = include_str!("../../assets/icons/erlang.svg");
const NIM_SVG: &str = include_str!("../../assets/icons/nim.svg");
const JULIA_SVG: &str = include_str!("../../assets/icons/julia.svg");
const R_SVG: &str = include_str!("../../assets/icons/r.svg");
const TEX_SVG: &str = include_str!("../../assets/icons/tex.svg");

// Web
const HTML_SVG: &str = include_str!("../../assets/icons/html.svg");
const CSS_SVG: &str = include_str!("../../assets/icons/css.svg");
const SASS_SVG: &str = include_str!("../../assets/icons/sass.svg");
const VUE_SVG: &str = include_str!("../../assets/icons/vue.svg");
const SVELTE_SVG: &str = include_str!("../../assets/icons/svelte.svg");
const REACT_SVG: &str = include_str!("../../assets/icons/react.svg");

// Data / Config
const JSON_SVG: &str = include_str!("../../assets/icons/json.svg");
const YAML_SVG: &str = include_str!("../../assets/icons/yaml.svg");
const TOML_SVG: &str = include_str!("../../assets/icons/toml.svg");
const XML_SVG: &str = include_str!("../../assets/icons/xml.svg");
const MARKDOWN_SVG: &str = include_str!("../../assets/icons/markdown.svg");
const SETTINGS_SVG: &str = include_str!("../../assets/icons/settings.svg");

// Tooling
const CONSOLE_SVG: &str = include_str!("../../assets/icons/console.svg");
const DOCKER_SVG: &str = include_str!("../../assets/icons/docker.svg");
const GIT_SVG: &str = include_str!("../../assets/icons/git.svg");
const LOCK_SVG: &str = include_str!("../../assets/icons/lock.svg");
const DATABASE_SVG: &str = include_str!("../../assets/icons/database.svg");

// Media
const IMAGE_SVG: &str = include_str!("../../assets/icons/image.svg");
const AUDIO_SVG: &str = include_str!("../../assets/icons/audio.svg");
const VIDEO_SVG: &str = include_str!("../../assets/icons/video.svg");
const PDF_SVG: &str = include_str!("../../assets/icons/pdf.svg");

// General
const DOCUMENT_SVG: &str = include_str!("../../assets/icons/document.svg");
const ARCHIVE_SVG: &str = include_str!("../../assets/icons/archive.svg");
const BINARY_SVG: &str = include_str!("../../assets/icons/binary.svg");

// Folders
const FOLDER_SVG: &str = include_str!("../../assets/icons/folder.svg");
const FOLDER_OPEN_SVG: &str = include_str!("../../assets/icons/folder-open.svg");

// Toolbar
const TOOLBAR_SIDEBAR_SVG: &str = include_str!("../../assets/icons/toolbar-sidebar.svg");
const TOOLBAR_PLUS_SVG: &str = include_str!("../../assets/icons/toolbar-plus.svg");
const TOOLBAR_EYE_OPEN_SVG: &str = include_str!("../../assets/icons/toolbar-eye-open.svg");
const TOOLBAR_EYE_CLOSED_SVG: &str = include_str!("../../assets/icons/toolbar-eye-closed.svg");
const TOOLBAR_COLLAPSE_SVG: &str = include_str!("../../assets/icons/toolbar-collapse.svg");
const TOOLBAR_REFRESH_SVG: &str = include_str!("../../assets/icons/toolbar-refresh.svg");
const TOOLBAR_NEW_FILE_SVG: &str = include_str!("../../assets/icons/toolbar-new-file.svg");
const TOOLBAR_NEW_FOLDER_SVG: &str = include_str!("../../assets/icons/toolbar-new-folder.svg");
const PIN_SVG: &str = include_str!("../../assets/icons/pin.svg");

// ---------------------------------------------------------------------------
// Color field — which theme color an icon uses
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum ColorField {
    Orange,
    Blue,
    Yellow,
    Green,
    Cyan,
    Magenta,
    Red,
    Comment,
    Fg,
}

impl ColorField {
    fn resolve(self, theme: &ThemeColors) -> &'static str {
        match self {
            ColorField::Orange => theme.orange,
            ColorField::Blue => theme.blue,
            ColorField::Yellow => theme.yellow,
            ColorField::Green => theme.green,
            ColorField::Cyan => theme.cyan,
            ColorField::Magenta => theme.magenta,
            ColorField::Red => theme.red,
            ColorField::Comment => theme.comment,
            ColorField::Fg => theme.fg,
        }
    }
}

// ---------------------------------------------------------------------------
// Icon registry — each icon has a name, SVG template, and color field
// ---------------------------------------------------------------------------

struct IconDef {
    name: &'static str,
    svg: &'static str,
    color: ColorField,
}

const ALL_ICONS: &[IconDef] = &[
    // Languages
    IconDef {
        name: "rust",
        svg: RUST_SVG,
        color: ColorField::Orange,
    },
    IconDef {
        name: "python",
        svg: PYTHON_SVG,
        color: ColorField::Blue,
    },
    IconDef {
        name: "javascript",
        svg: JAVASCRIPT_SVG,
        color: ColorField::Yellow,
    },
    IconDef {
        name: "typescript",
        svg: TYPESCRIPT_SVG,
        color: ColorField::Blue,
    },
    IconDef {
        name: "go",
        svg: GO_SVG,
        color: ColorField::Cyan,
    },
    IconDef {
        name: "c",
        svg: C_SVG,
        color: ColorField::Blue,
    },
    IconDef {
        name: "cpp",
        svg: CPP_SVG,
        color: ColorField::Blue,
    },
    IconDef {
        name: "java",
        svg: JAVA_SVG,
        color: ColorField::Orange,
    },
    IconDef {
        name: "kotlin",
        svg: KOTLIN_SVG,
        color: ColorField::Magenta,
    },
    IconDef {
        name: "swift",
        svg: SWIFT_SVG,
        color: ColorField::Orange,
    },
    IconDef {
        name: "ruby",
        svg: RUBY_SVG,
        color: ColorField::Red,
    },
    IconDef {
        name: "php",
        svg: PHP_SVG,
        color: ColorField::Magenta,
    },
    IconDef {
        name: "csharp",
        svg: CSHARP_SVG,
        color: ColorField::Green,
    },
    IconDef {
        name: "zig",
        svg: ZIG_SVG,
        color: ColorField::Orange,
    },
    IconDef {
        name: "haskell",
        svg: HASKELL_SVG,
        color: ColorField::Magenta,
    },
    IconDef {
        name: "lua",
        svg: LUA_SVG,
        color: ColorField::Blue,
    },
    IconDef {
        name: "dart",
        svg: DART_SVG,
        color: ColorField::Cyan,
    },
    IconDef {
        name: "elixir",
        svg: ELIXIR_SVG,
        color: ColorField::Magenta,
    },
    IconDef {
        name: "scala",
        svg: SCALA_SVG,
        color: ColorField::Magenta,
    },
    IconDef {
        name: "clojure",
        svg: CLOJURE_SVG,
        color: ColorField::Green,
    },
    IconDef {
        name: "erlang",
        svg: ERLANG_SVG,
        color: ColorField::Red,
    },
    IconDef {
        name: "nim",
        svg: NIM_SVG,
        color: ColorField::Yellow,
    },
    IconDef {
        name: "julia",
        svg: JULIA_SVG,
        color: ColorField::Magenta,
    },
    IconDef {
        name: "r",
        svg: R_SVG,
        color: ColorField::Blue,
    },
    IconDef {
        name: "tex",
        svg: TEX_SVG,
        color: ColorField::Orange,
    },
    // Web
    IconDef {
        name: "html",
        svg: HTML_SVG,
        color: ColorField::Orange,
    },
    IconDef {
        name: "css",
        svg: CSS_SVG,
        color: ColorField::Blue,
    },
    IconDef {
        name: "sass",
        svg: SASS_SVG,
        color: ColorField::Magenta,
    },
    IconDef {
        name: "vue",
        svg: VUE_SVG,
        color: ColorField::Green,
    },
    IconDef {
        name: "svelte",
        svg: SVELTE_SVG,
        color: ColorField::Orange,
    },
    IconDef {
        name: "react",
        svg: REACT_SVG,
        color: ColorField::Cyan,
    },
    // Data / Config
    IconDef {
        name: "json",
        svg: JSON_SVG,
        color: ColorField::Yellow,
    },
    IconDef {
        name: "yaml",
        svg: YAML_SVG,
        color: ColorField::Orange,
    },
    IconDef {
        name: "toml",
        svg: TOML_SVG,
        color: ColorField::Orange,
    },
    IconDef {
        name: "xml",
        svg: XML_SVG,
        color: ColorField::Comment,
    },
    IconDef {
        name: "markdown",
        svg: MARKDOWN_SVG,
        color: ColorField::Blue,
    },
    IconDef {
        name: "settings",
        svg: SETTINGS_SVG,
        color: ColorField::Comment,
    },
    // Tooling
    IconDef {
        name: "console",
        svg: CONSOLE_SVG,
        color: ColorField::Green,
    },
    IconDef {
        name: "docker",
        svg: DOCKER_SVG,
        color: ColorField::Cyan,
    },
    IconDef {
        name: "git",
        svg: GIT_SVG,
        color: ColorField::Orange,
    },
    IconDef {
        name: "lock",
        svg: LOCK_SVG,
        color: ColorField::Comment,
    },
    IconDef {
        name: "database",
        svg: DATABASE_SVG,
        color: ColorField::Cyan,
    },
    // Media
    IconDef {
        name: "image",
        svg: IMAGE_SVG,
        color: ColorField::Green,
    },
    IconDef {
        name: "audio",
        svg: AUDIO_SVG,
        color: ColorField::Magenta,
    },
    IconDef {
        name: "video",
        svg: VIDEO_SVG,
        color: ColorField::Magenta,
    },
    IconDef {
        name: "pdf",
        svg: PDF_SVG,
        color: ColorField::Red,
    },
    // General
    IconDef {
        name: "document",
        svg: DOCUMENT_SVG,
        color: ColorField::Fg,
    },
    IconDef {
        name: "archive",
        svg: ARCHIVE_SVG,
        color: ColorField::Comment,
    },
    IconDef {
        name: "binary",
        svg: BINARY_SVG,
        color: ColorField::Comment,
    },
    // Folders
    IconDef {
        name: "folder",
        svg: FOLDER_SVG,
        color: ColorField::Cyan,
    },
    IconDef {
        name: "folder-open",
        svg: FOLDER_OPEN_SVG,
        color: ColorField::Cyan,
    },
    // Toolbar
    IconDef {
        name: "toolbar-sidebar",
        svg: TOOLBAR_SIDEBAR_SVG,
        color: ColorField::Fg,
    },
    IconDef {
        name: "toolbar-plus",
        svg: TOOLBAR_PLUS_SVG,
        color: ColorField::Fg,
    },
    IconDef {
        name: "toolbar-eye-open",
        svg: TOOLBAR_EYE_OPEN_SVG,
        color: ColorField::Fg,
    },
    IconDef {
        name: "toolbar-eye-closed",
        svg: TOOLBAR_EYE_CLOSED_SVG,
        color: ColorField::Fg,
    },
    IconDef {
        name: "toolbar-collapse",
        svg: TOOLBAR_COLLAPSE_SVG,
        color: ColorField::Fg,
    },
    IconDef {
        name: "toolbar-refresh",
        svg: TOOLBAR_REFRESH_SVG,
        color: ColorField::Fg,
    },
    IconDef {
        name: "toolbar-new-file",
        svg: TOOLBAR_NEW_FILE_SVG,
        color: ColorField::Fg,
    },
    IconDef {
        name: "toolbar-new-folder",
        svg: TOOLBAR_NEW_FOLDER_SVG,
        color: ColorField::Fg,
    },
    IconDef {
        name: "pin",
        svg: PIN_SVG,
        color: ColorField::Comment,
    },
];

// ---------------------------------------------------------------------------
// Extension / filename → icon name lookup
// ---------------------------------------------------------------------------

fn lookup_icon_name(filename: &str, is_dir: bool, expanded: bool) -> &'static str {
    if is_dir {
        return if expanded { "folder-open" } else { "folder" };
    }

    let ext = filename.rsplit('.').next().unwrap_or("");
    match ext.to_lowercase().as_str() {
        // Languages
        "rs" => "rust",
        "py" | "pyi" | "pyw" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "go" => "go",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "hh" => "cpp",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "rb" | "erb" => "ruby",
        "php" => "php",
        "cs" => "csharp",
        "zig" => "zig",
        "hs" | "lhs" => "haskell",
        "lua" => "lua",
        "dart" => "dart",
        "ex" | "exs" | "heex" => "elixir",
        "scala" | "sc" => "scala",
        "clj" | "cljs" | "cljc" | "edn" => "clojure",
        "erl" | "hrl" => "erlang",
        "nim" | "nims" => "nim",
        "jl" => "julia",
        "r" | "rmd" => "r",
        "tex" | "sty" | "cls" | "bib" => "tex",

        // Web
        "html" | "htm" => "html",
        "css" => "css",
        "scss" | "sass" | "less" => "sass",
        "vue" => "vue",
        "svelte" => "svelte",
        "jsx" | "tsx" => "react",

        // Data / Config
        "json" | "jsonc" | "json5" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "xml" | "xsl" | "xslt" | "xsd" | "wsdl" => "xml",
        "md" | "mdx" | "markdown" => "markdown",
        "ini" | "cfg" | "conf" | "ron" | "properties" => "settings",

        // Shell / Tooling
        "sh" | "bash" | "zsh" | "fish" | "ps1" | "bat" | "cmd" => "console",
        "lock" => "lock",
        "sql" | "sqlite" | "db" => "database",

        // Media
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "ico" | "webp" | "bmp" | "tiff" | "avif" => {
            "image"
        }
        "mp3" | "wav" | "flac" | "ogg" | "aac" | "wma" | "m4a" => "audio",
        "mp4" | "mkv" | "avi" | "webm" | "mov" | "wmv" | "flv" => "video",
        "pdf" => "pdf",

        // Archives
        "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "zst" | "tgz" => "archive",

        // Binary / Executables
        "exe" | "dll" | "so" | "dylib" | "a" | "o" | "wasm" => "binary",

        // Default: check special filenames
        _ => lookup_by_filename(filename),
    }
}

fn lookup_by_filename(filename: &str) -> &'static str {
    match filename.to_lowercase().as_str() {
        "dockerfile" | "containerfile" => "docker",
        "makefile" | "rakefile" | "justfile" | "taskfile" => "console",
        ".gitignore" | ".gitmodules" | ".gitattributes" => "git",
        "license" | "licence" | "license.md" | "licence.md" | "license.txt" | "licence.txt" => {
            "document"
        }
        "readme" | "readme.md" | "readme.txt" => "document",
        "changelog" | "changelog.md" | "authors" | "contributing" | "contributing.md" => "document",
        "cargo.toml" | "cargo.lock" => "rust",
        "package.json" | "package-lock.json" => "javascript",
        "tsconfig.json" => "typescript",
        "go.mod" | "go.sum" => "go",
        "gemfile" | "gemfile.lock" => "ruby",
        "composer.json" | "composer.lock" => "php",
        ".eslintrc" | ".prettierrc" | ".editorconfig" => "settings",
        _ => "document",
    }
}

// ---------------------------------------------------------------------------
// SVG recoloring — replaces hex fill/stroke values with theme color
// ---------------------------------------------------------------------------

fn replace_attr_colors(svg: &str, attr: &str, color: &str) -> String {
    let pattern = format!("{attr}=\"#");
    let prefix = format!("{attr}=\"");
    let mut result = String::with_capacity(svg.len());
    let mut remaining = svg;

    while let Some(pos) = remaining.find(&pattern) {
        result.push_str(&remaining[..pos]);
        result.push_str(&prefix);
        result.push_str(color);
        result.push('"');
        let value_start = pos + prefix.len();
        if let Some(end_quote) = remaining[value_start..].find('"') {
            remaining = &remaining[value_start + end_quote + 1..];
        } else {
            remaining = "";
            break;
        }
    }
    result.push_str(remaining);
    result
}

fn recolor_svg(svg: &str, color: &str) -> String {
    let result = replace_attr_colors(svg, "fill", color);
    replace_attr_colors(&result, "stroke", color)
}

// ---------------------------------------------------------------------------
// SVG → gdk::Texture rendering via resvg
// ---------------------------------------------------------------------------

const ICON_RENDER_SIZE: u32 = 32;

fn render_svg_to_texture(svg: &str, color: &str) -> Option<gdk::Texture> {
    let recolored = recolor_svg(svg, color);
    let opts = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(recolored.as_bytes(), &opts).ok()?;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(ICON_RENDER_SIZE, ICON_RENDER_SIZE)?;

    // Scale SVG to fit within the target size, centered
    let svg_size = tree.size();
    let sx = ICON_RENDER_SIZE as f32 / svg_size.width();
    let sy = ICON_RENDER_SIZE as f32 / svg_size.height();
    let scale = sx.min(sy);
    let tx = (ICON_RENDER_SIZE as f32 - svg_size.width() * scale) / 2.0;
    let ty = (ICON_RENDER_SIZE as f32 - svg_size.height() * scale) / 2.0;

    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(tx, ty);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let png_data = pixmap.encode_png().ok()?;
    let bytes = glib::Bytes::from(&png_data);
    gdk::Texture::from_bytes(&bytes).ok()
}

// ---------------------------------------------------------------------------
// Icon cache — pre-renders all icons for the current theme
// ---------------------------------------------------------------------------

pub struct IconCache {
    textures: HashMap<&'static str, gdk::Texture>,
}

impl IconCache {
    pub fn new(theme: &ThemeColors) -> Self {
        let mut cache = Self {
            textures: HashMap::with_capacity(ALL_ICONS.len()),
        };
        cache.build(theme);
        cache
    }

    pub fn rebuild(&mut self, theme: &ThemeColors) {
        self.textures.clear();
        self.build(theme);
    }

    fn build(&mut self, theme: &ThemeColors) {
        for icon in ALL_ICONS {
            let color = icon.color.resolve(theme);
            if let Some(texture) = render_svg_to_texture(icon.svg, color) {
                self.textures.insert(icon.name, texture);
            }
        }
    }

    pub fn get(&self, filename: &str, is_dir: bool, expanded: bool) -> Option<&gdk::Texture> {
        let name = lookup_icon_name(filename, is_dir, expanded);
        self.textures.get(name)
    }

    /// Returns a toolbar icon texture by name (e.g. "toolbar-sidebar", "console", "settings").
    pub fn get_toolbar_icon(&self, name: &str) -> Option<&gdk::Texture> {
        self.textures.get(name)
    }
}
