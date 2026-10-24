//! File and folder icons from the material icon set
//! (`assets/icons/material`, mapped by its `icon-mapping.json`), picked the
//! way the Swift `FileIcons.swift` picks them: a file by its whole name, then
//! by its last extension, else the plain file icon; a folder by its name
//! (lowercased, leading dots and underscores and trailing underscores
//! dropped), else the plain folder icon. The SVGs keep their own colors; the
//! asset loader serves them as `material/<file>.svg`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::LazyLock;

use serde::Deserialize;

#[derive(Deserialize, Default)]
struct Mapping {
    /// Icon name to SVG file name.
    file_icons: HashMap<String, String>,
    /// Extension to icon name.
    file_suffixes: HashMap<String, String>,
    /// Lowercase file name to icon name.
    file_stems: HashMap<String, String>,
    directory_icons: DirectoryIcons,
    /// Folder name to [collapsed, expanded] SVG file names.
    named_directories: HashMap<String, Vec<String>>,
}

#[derive(Deserialize)]
struct DirectoryIcons {
    collapsed: String,
    expanded: String,
}

impl Default for DirectoryIcons {
    fn default() -> Self {
        DirectoryIcons {
            collapsed: "folder.svg".into(),
            expanded: "folder-open.svg".into(),
        }
    }
}

/// Where the material icons are.
pub fn material_dir() -> PathBuf {
    crate::resources::get().material.clone()
}

static MAPPING: LazyLock<Mapping> = LazyLock::new(|| {
    std::fs::read(material_dir().join("icon-mapping.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_else(|| {
            log::warn!("icon-mapping.json didn't load; files get the plain icon");
            Mapping::default()
        })
});

/// The asset path (`material/<file>.svg`) of the icon for `name`.
pub fn icon_path(name: &str, is_dir: bool, expanded: bool) -> String {
    format!("material/{}", icon_file(&MAPPING, name, is_dir, expanded))
}

fn icon_file(mapping: &Mapping, name: &str, is_dir: bool, expanded: bool) -> String {
    if is_dir {
        let normalized = name
            .to_lowercase()
            .trim_start_matches(['.', '_'])
            .trim_end_matches('_')
            .to_string();
        if let Some(pair) = mapping.named_directories.get(&normalized) {
            if let Some(file) = pair.get(usize::from(expanded)) {
                return file.clone();
            }
        }
        let icons = &mapping.directory_icons;
        return if expanded {
            icons.expanded.clone()
        } else {
            icons.collapsed.clone()
        };
    }
    let lower = name.to_lowercase();
    let by_stem = mapping.file_stems.get(&lower);
    let by_suffix = lower
        .rfind('.')
        .and_then(|dot| mapping.file_suffixes.get(&lower[dot + 1..]));
    by_stem
        .into_iter()
        .chain(by_suffix)
        .find_map(|icon| mapping.file_icons.get(icon))
        .cloned()
        .unwrap_or_else(|| "file.svg".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_by_name_then_extension_then_default() {
        let mapping = &*MAPPING;
        assert_eq!(icon_file(mapping, "main.rs", false, false), "rust.svg");
        assert_eq!(icon_file(mapping, "Cargo.TOML", false, false), "toml.svg");
        assert_eq!(
            icon_file(mapping, "no-such.extension", false, false),
            "file.svg"
        );
        assert_eq!(icon_file(mapping, "src", true, false), "folder-src.svg");
        assert_eq!(
            icon_file(mapping, "_src_", true, true),
            "folder-src-open.svg"
        );
        assert_eq!(icon_file(mapping, "plain", true, false), "folder.svg");
    }
}
