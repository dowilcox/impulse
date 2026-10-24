//! Where the app's files are: the icons (lucide and material) and the
//! bundled fonts. An installed app has them in one folder beside its
//! executable; a build run from the repository reads them from the tree.
//!
//! Looked for in order: `$IMPULSE_RESOURCES`, `<exe>/../share/impulse`
//! (Linux packages: `/usr/bin` and `/usr/share/impulse`),
//! `<exe>/../Resources` (a macOS app bundle), then the source tree.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

pub(crate) struct Resources {
    /// Lucide's SVGs (`<name>.svg`).
    pub lucide: PathBuf,
    /// The material file icons and `icon-mapping.json`.
    pub material: PathBuf,
    /// `inter/` and `jetbrains-mono/`.
    pub fonts: PathBuf,
}

impl Resources {
    /// An installed layout under `root`.
    fn installed(root: &Path) -> Self {
        Resources {
            lucide: root.join("lucide"),
            material: root.join("material"),
            fonts: root.join("fonts"),
        }
    }

    fn is_complete(&self) -> bool {
        self.lucide.is_dir() && self.material.is_dir() && self.fonts.is_dir()
    }
}

static RESOURCES: LazyLock<Resources> = LazyLock::new(|| {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os("IMPULSE_RESOURCES") {
        roots.push(dir.into());
    }
    if let Some(bin) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.canonicalize().ok())
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        roots.push(bin.join("../share/impulse"));
        roots.push(bin.join("../Resources"));
    }
    roots
        .iter()
        .map(|root| Resources::installed(root))
        .find(Resources::is_complete)
        .unwrap_or_else(|| {
            let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
            Resources {
                lucide: repo.join("vendor/lucide/icons"),
                material: repo.join("assets/icons/material"),
                fonts: repo.join("vendor/fonts"),
            }
        })
});

pub(crate) fn get() -> &'static Resources {
    &RESOURCES
}
