//! Which servers run for which language, and where (the Swift `LSPConfig`
//! in `LanguageTable.swift`, itself a port of the Rust-era `LspConfig`).
//!
//! The defaults below can be changed by the global `lsp.json`
//! (`$XDG_CONFIG_HOME/impulse/lsp.json`, else `~/.config/impulse/lsp.json`),
//! which may define any server, and by a project's `.impulse/lsp.json` (or
//! `.impulse-lsp.json`), which may only choose among servers that already
//! exist: a repository can't make Impulse run a command of its choosing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

#[derive(Clone, Debug, PartialEq)]
pub struct ServerConfig {
    pub command: String,
    pub args: Vec<String>,
    pub initialization_options: Option<Value>,
}

impl ServerConfig {
    fn new(command: &str, args: &[&str]) -> Self {
        Self {
            command: command.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
            initialization_options: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LspConfig {
    pub servers: BTreeMap<String, ServerConfig>,
    pub language_servers: BTreeMap<String, Vec<String>>,
    pub root_markers: Vec<String>,
}

impl Default for LspConfig {
    fn default() -> Self {
        let servers = [
            ("rust-analyzer", ServerConfig::new("rust-analyzer", &[])),
            (
                "pyright",
                ServerConfig::new("pyright-langserver", &["--stdio"]),
            ),
            ("clangd", ServerConfig::new("clangd", &[])),
            (
                "typescript-language-server",
                ServerConfig::new("typescript-language-server", &["--stdio"]),
            ),
            (
                "intelephense",
                ServerConfig::new("intelephense", &["--stdio"]),
            ),
            (
                "vscode-html-language-server",
                ServerConfig::new("vscode-html-language-server", &["--stdio"]),
            ),
            (
                "vscode-css-language-server",
                ServerConfig::new("vscode-css-language-server", &["--stdio"]),
            ),
            (
                "vscode-json-language-server",
                ServerConfig::new("vscode-json-language-server", &["--stdio"]),
            ),
            (
                "vscode-eslint-language-server",
                ServerConfig::new("vscode-eslint-language-server", &["--stdio"]),
            ),
            (
                "tailwindcss-language-server",
                ServerConfig::new("tailwindcss-language-server", &["--stdio"]),
            ),
            (
                "vue-language-server",
                ServerConfig::new("vue-language-server", &["--stdio"]),
            ),
            (
                "svelteserver",
                ServerConfig::new("svelteserver", &["--stdio"]),
            ),
            (
                "graphql-lsp",
                ServerConfig::new("graphql-lsp", &["server", "-m", "stream"]),
            ),
            ("emmet-ls", ServerConfig::new("emmet-ls", &["--stdio"])),
            (
                "yaml-language-server",
                ServerConfig::new("yaml-language-server", &["--stdio"]),
            ),
            (
                "docker-langserver",
                ServerConfig::new("docker-langserver", &["--stdio"]),
            ),
            (
                "bash-language-server",
                ServerConfig::new("bash-language-server", &["start"]),
            ),
            // Ships with Xcode and the Command Line Tools.
            ("sourcekit-lsp", ServerConfig::new("sourcekit-lsp", &[])),
        ]
        .into_iter()
        .map(|(id, config)| (id.to_string(), config))
        .collect();

        let web_stack = [
            "typescript-language-server",
            "vscode-eslint-language-server",
            "tailwindcss-language-server",
            "emmet-ls",
        ];
        let css = [
            "vscode-css-language-server",
            "tailwindcss-language-server",
            "emmet-ls",
        ];
        let component = |first: &'static str| {
            vec![
                first,
                "vscode-eslint-language-server",
                "tailwindcss-language-server",
                "emmet-ls",
            ]
        };
        let language_servers: Vec<(&str, Vec<&str>)> = vec![
            ("rust", vec!["rust-analyzer"]),
            ("python", vec!["pyright"]),
            ("c", vec!["clangd"]),
            ("cpp", vec!["clangd"]),
            ("javascript", web_stack.to_vec()),
            ("javascriptreact", web_stack.to_vec()),
            ("typescript", web_stack.to_vec()),
            ("typescriptreact", web_stack.to_vec()),
            ("php", vec!["intelephense"]),
            (
                "html",
                vec![
                    "vscode-html-language-server",
                    "tailwindcss-language-server",
                    "emmet-ls",
                ],
            ),
            ("css", css.to_vec()),
            ("scss", css.to_vec()),
            ("less", css.to_vec()),
            ("json", vec!["vscode-json-language-server"]),
            ("jsonc", vec!["vscode-json-language-server"]),
            ("yaml", vec!["yaml-language-server"]),
            ("vue", component("vue-language-server")),
            ("svelte", component("svelteserver")),
            ("graphql", vec!["graphql-lsp"]),
            ("dockerfile", vec!["docker-langserver"]),
            ("shellscript", vec!["bash-language-server"]),
            ("swift", vec!["sourcekit-lsp"]),
        ];

        let root_markers = [
            "Cargo.toml",
            "package.json",
            "tsconfig.json",
            "jsconfig.json",
            "pnpm-workspace.yaml",
            "yarn.lock",
            "package-lock.json",
            "bun.lockb",
            "turbo.json",
            "nx.json",
            "go.mod",
            "pyproject.toml",
            "setup.py",
            "composer.json",
            "Gemfile",
            "deno.json",
            "deno.jsonc",
            "Package.swift",
        ];

        Self {
            servers,
            language_servers: language_servers
                .into_iter()
                .map(|(lang, ids)| {
                    (
                        lang.to_string(),
                        ids.iter().map(|s| s.to_string()).collect(),
                    )
                })
                .collect(),
            root_markers: root_markers.iter().map(|s| s.to_string()).collect(),
        }
    }
}

/// A parsed `lsp.json`. Any structural error rejects the whole file.
#[derive(Default)]
struct Overrides {
    servers: Option<BTreeMap<String, ServerConfig>>,
    language_servers: Option<BTreeMap<String, Vec<String>>>,
    root_markers: Option<Vec<String>>,
}

fn string_list(value: &Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|v| v.as_str().map(str::to_string))
        .collect()
}

fn parse_overrides(data: &[u8]) -> Option<Overrides> {
    let root: Map<String, Value> = serde_json::from_slice(data).ok()?;
    let present = |key: &str| root.get(key).filter(|v| !v.is_null());
    let mut overrides = Overrides::default();
    if let Some(raw) = present("servers") {
        let mut servers = BTreeMap::new();
        for (id, value) in raw.as_object()? {
            let object = value.as_object()?;
            let command = object.get("command")?.as_str()?.to_string();
            let args = match object.get("args") {
                Some(args) => string_list(args)?,
                None => Vec::new(),
            };
            let initialization_options = object
                .get("initialization_options")
                .filter(|v| !v.is_null())
                .cloned();
            servers.insert(
                id.clone(),
                ServerConfig {
                    command,
                    args,
                    initialization_options,
                },
            );
        }
        overrides.servers = Some(servers);
    }
    if let Some(raw) = present("language_servers") {
        let mut map = BTreeMap::new();
        for (lang, ids) in raw.as_object()? {
            map.insert(lang.clone(), string_list(ids)?);
        }
        overrides.language_servers = Some(map);
    }
    if let Some(raw) = present("root_markers") {
        overrides.root_markers = Some(string_list(raw)?);
    }
    Some(overrides)
}

impl LspConfig {
    /// The defaults with the global file, then the project's, applied.
    pub fn load(global: Option<&Path>, project_folder: Option<&Path>) -> Self {
        let mut config = Self::default();
        if let Some(global) = global {
            config.apply_file(global, true);
        }
        if let Some(folder) = project_folder {
            for path in project_config_paths(folder) {
                config.apply_file(&path, false);
            }
        }
        config
    }

    /// Apply one `lsp.json`. `trusted` (the global file) may define servers;
    /// a project's file may only map languages to servers that exist.
    pub fn apply_file(&mut self, path: &Path, trusted: bool) {
        let Ok(data) = std::fs::read(path) else {
            return;
        };
        let Some(overrides) = parse_overrides(&data) else {
            log::warn!("Invalid LSP config at {}", path.display());
            return;
        };
        if let Some(servers) = overrides.servers {
            if trusted {
                self.servers.extend(servers);
            } else {
                for id in servers.keys() {
                    log::warn!(
                        "Project LSP config tried to configure server '{id}': ignoring \
                         (only the global config can define servers)"
                    );
                }
            }
        }
        if let Some(language_servers) = overrides.language_servers {
            for (lang, ids) in language_servers {
                let ids: Vec<String> = if trusted {
                    ids
                } else {
                    ids.into_iter()
                        .filter(|id| self.servers.contains_key(id))
                        .collect()
                };
                if trusted || !ids.is_empty() {
                    self.language_servers.insert(lang, ids);
                }
            }
        }
        if let Some(markers) = overrides.root_markers {
            let markers: Vec<String> = markers
                .into_iter()
                .filter(|m| {
                    !m.is_empty() && !m.contains('/') && !m.contains('\\') && !m.contains("..")
                })
                .collect();
            if !markers.is_empty() {
                self.root_markers = markers;
            }
        }
    }

    /// The servers for a language, in order (the first one answers
    /// completion, hover and definition).
    pub fn servers_for(&self, language: &str) -> Vec<(String, ServerConfig)> {
        self.language_servers
            .get(language)
            .into_iter()
            .flatten()
            .filter_map(|id| self.servers.get(id).map(|c| (id.clone(), c.clone())))
            .collect()
    }
}

/// `$XDG_CONFIG_HOME/impulse/lsp.json`, else `~/.config/impulse/lsp.json`
/// (on every platform, as the Swift app has it).
pub fn global_config_path() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg).join("impulse/lsp.json"));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/impulse/lsp.json"))
}

pub fn project_config_paths(folder: &Path) -> [PathBuf; 2] {
    [
        folder.join(".impulse/lsp.json"),
        folder.join(".impulse-lsp.json"),
    ]
}

/// The nearest folder at or above `directory` with a project `lsp.json`,
/// not above the repository it's in.
pub fn project_config_folder(directory: &Path) -> Option<PathBuf> {
    let mut dir = Some(directory);
    while let Some(current) = dir {
        if project_config_paths(current).iter().any(|p| p.exists()) {
            return Some(current.to_path_buf());
        }
        if current.join(".git").exists() {
            return None;
        }
        dir = current.parent();
    }
    None
}

/// The folder a server for `file` works in: the innermost folder with a root
/// marker, else the innermost repository. With a `ceiling` (the trusted
/// folder the file is in, as `trust::normalize` spells it) the walk stops at
/// the last folder inside it, which is the root when nothing below it is: a
/// server never works above the folder that was trusted.
pub fn detect_project_root(
    file: &Path,
    markers: &[String],
    ceiling: Option<&str>,
) -> Option<PathBuf> {
    let mut dir = file.parent()?;
    let mut git_root = None;
    let mut last_inside = None;
    loop {
        if let Some(ceiling) = ceiling {
            let resolved = impulse_kit::trust::normalize(&dir.to_string_lossy());
            let inside = ceiling == "/"
                || resolved == ceiling
                || resolved.starts_with(&format!("{ceiling}/"));
            if !inside {
                break;
            }
            last_inside = Some(dir.to_path_buf());
        }
        if markers.iter().any(|m| dir.join(m).exists()) {
            // The innermost marker: nothing further up matters.
            return Some(dir.to_path_buf());
        }
        if git_root.is_none() && dir.join(".git").exists() {
            git_root = Some(dir.to_path_buf());
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent,
            _ => break,
        }
    }
    git_root.or(last_inside)
}

/// Initialization options some servers need (`defaultInitOptions`).
pub fn default_init_options(server_id: &str, managed_root: Option<&Path>) -> Option<Value> {
    match server_id {
        "intelephense" => {
            let storage = managed_root
                .map(Path::to_path_buf)
                .unwrap_or_else(|| impulse_kit::paths::data_dir(false).join("lsp"))
                .join("intelephense");
            let storage = storage.to_string_lossy();
            Some(json!({ "storagePath": storage, "globalStoragePath": storage }))
        }
        "typescript-language-server" => Some(json!({
            "hostInfo": "Impulse",
            "preferences": {
                "quotePreference": "auto",
                "importModuleSpecifierPreference": "shortest",
                "jsxAttributeCompletionStyle": "auto",
            },
        })),
        "vscode-eslint-language-server" => Some(json!({ "validate": "probe" })),
        _ => None,
    }
}

fn ts_js_settings() -> Value {
    json!({
        "implicitProjectConfiguration": {
            "checkJs": false,
            "experimentalDecorators": false,
            "module": "ESNext",
            "strictNullChecks": true,
            "strictFunctionTypes": true,
            "target": "ES2020",
        },
        "suggest": { "autoImports": true, "completeFunctionCalls": false },
        "preferences": {
            "quotePreference": "auto",
            "importModuleSpecifierPreference": "shortest",
            "jsxAttributeCompletionStyle": "auto",
        },
        "format": { "insertSpaceAfterOpeningAndBeforeClosingNonemptyBraces": true },
    })
}

/// The answer to `workspace/configuration` for one section.
pub fn workspace_configuration_for_section(section: &str) -> Value {
    match section {
        "typescript" | "javascript" => ts_js_settings(),
        "typescript.format" | "javascript.format" => {
            json!({ "insertSpaceAfterOpeningAndBeforeClosingNonemptyBraces": true })
        }
        _ => json!({}),
    }
}

/// Pushed with `workspace/didChangeConfiguration` after `initialized`.
pub fn default_workspace_settings() -> Value {
    json!({ "typescript": ts_js_settings(), "javascript": ts_js_settings() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn default_tables_match_the_swift_app() {
        let cfg = LspConfig::default();
        assert_eq!(cfg.servers.len(), 18);
        assert_eq!(cfg.servers["rust-analyzer"].command, "rust-analyzer");
        assert!(cfg.servers["rust-analyzer"].args.is_empty());
        assert_eq!(cfg.servers["pyright"].command, "pyright-langserver");
        assert_eq!(cfg.servers["pyright"].args, ["--stdio"]);
        assert_eq!(cfg.servers["bash-language-server"].args, ["start"]);
        assert_eq!(cfg.servers["graphql-lsp"].args, ["server", "-m", "stream"]);
        assert_eq!(
            cfg.language_servers["typescript"],
            [
                "typescript-language-server",
                "vscode-eslint-language-server",
                "tailwindcss-language-server",
                "emmet-ls"
            ]
        );
        assert_eq!(cfg.language_servers["rust"], ["rust-analyzer"]);
        assert_eq!(
            cfg.language_servers["shellscript"],
            ["bash-language-server"]
        );
        assert_eq!(cfg.servers["sourcekit-lsp"].command, "sourcekit-lsp");
        assert_eq!(cfg.language_servers["swift"], ["sourcekit-lsp"]);
        assert_eq!(cfg.root_markers.len(), 18);
        assert_eq!(cfg.root_markers[0], "Cargo.toml");
        assert!(cfg.root_markers.contains(&"deno.jsonc".to_string()));
    }

    #[test]
    fn untrusted_project_config_cannot_define_servers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lsp.json");
        write(
            &path,
            r#"{
              "servers": {"evil": {"command": "/bin/evil"}},
              "language_servers": {"rust": ["typescript-language-server"], "foo": ["evil"]},
              "root_markers": ["ok.json", "../bad", "a/b", ""]
            }"#,
        );
        let mut cfg = LspConfig::default();
        cfg.apply_file(&path, false);
        assert!(!cfg.servers.contains_key("evil"));
        // Remapping to an existing server is allowed.
        assert_eq!(cfg.language_servers["rust"], ["typescript-language-server"]);
        // Mapping to an unknown server is filtered to nothing.
        assert!(!cfg.language_servers.contains_key("foo"));
        // Markers with path separators, ".." or nothing are filtered out.
        assert_eq!(cfg.root_markers, ["ok.json"]);
    }

    #[test]
    fn trusted_global_config_can_define_servers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lsp.json");
        write(
            &path,
            r#"{
              "servers": {"mylang-ls": {"command": "/usr/local/bin/mylang-ls", "args": ["--stdio"]}},
              "language_servers": {"mylang": ["mylang-ls"]}
            }"#,
        );
        let mut cfg = LspConfig::default();
        cfg.apply_file(&path, true);
        assert_eq!(cfg.servers["mylang-ls"].command, "/usr/local/bin/mylang-ls");
        assert_eq!(cfg.servers["mylang-ls"].args, ["--stdio"]);
        assert_eq!(cfg.language_servers["mylang"], ["mylang-ls"]);
        assert!(
            cfg.servers.contains_key("rust-analyzer"),
            "untouched defaults survive"
        );
        assert_eq!(cfg.servers_for("mylang")[0].0, "mylang-ls");
    }

    #[test]
    fn invalid_config_file_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lsp.json");
        write(&path, "not json");
        let mut cfg = LspConfig::default();
        cfg.apply_file(&path, true);
        assert_eq!(cfg.servers.len(), 18);
        // Structurally invalid overrides are rejected wholesale.
        write(&path, r#"{"servers": {"x": {"args": []}}}"#);
        cfg.apply_file(&path, true);
        assert!(!cfg.servers.contains_key("x"));
        assert_eq!(cfg.servers.len(), 18);
    }

    #[test]
    fn project_config_folder_is_the_nearest_one_in_the_repository() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // A config above the repository doesn't apply to it.
        write(&root.join(".impulse/lsp.json"), "{}");
        std::fs::create_dir_all(root.join("repo/.git")).unwrap();
        std::fs::create_dir_all(root.join("repo/src/deep")).unwrap();
        assert_eq!(project_config_folder(&root.join("repo/src/deep")), None);

        write(&root.join("repo/.impulse-lsp.json"), "{}");
        assert_eq!(
            project_config_folder(&root.join("repo/src/deep")),
            Some(root.join("repo"))
        );

        write(&root.join("repo/src/.impulse/lsp.json"), "{}");
        assert_eq!(
            project_config_folder(&root.join("repo/src/deep")),
            Some(root.join("repo/src"))
        );
        // Outside any repository, the walk goes up until it finds one.
        std::fs::create_dir_all(root.join("loose/a")).unwrap();
        assert_eq!(
            project_config_folder(&root.join("loose/a")),
            Some(root.to_path_buf())
        );
    }

    #[test]
    fn project_config_applies_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("one/.git")).unwrap();
        write(
            &root.join("one/.impulse/lsp.json"),
            r#"{"language_servers": {"rust": ["clangd"]}}"#,
        );
        let global = root.join("global/lsp.json");
        write(
            &global,
            r#"{"servers": {"gopls": {"command": "gopls"}}, "language_servers": {"go": ["gopls"]}}"#,
        );
        let cfg = LspConfig::load(Some(&global), Some(&root.join("one")));
        assert_eq!(cfg.language_servers["rust"], ["clangd"]);
        assert_eq!(cfg.servers["gopls"].command, "gopls");
        assert_eq!(cfg.servers_for("go").len(), 1);
    }

    fn markers(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn marker_root_beats_outer_git_root() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let package = repo.join("packages/app");
        write(&package.join("package.json"), "{}");
        let file = package.join("src/main.ts");
        write(&file, "export {};\n");
        assert_eq!(
            detect_project_root(&file, &markers(&["package.json"]), None),
            Some(package)
        );
    }

    #[test]
    fn git_root_used_when_no_marker_exists() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let file = repo.join("src/main.rs");
        write(&file, "fn main() {}\n");
        assert_eq!(
            detect_project_root(&file, &markers(&["go.mod"]), None),
            Some(repo.to_path_buf())
        );
    }

    #[test]
    fn innermost_marker_wins() {
        let dir = tempfile::tempdir().unwrap();
        let outer = dir.path();
        write(&outer.join("Cargo.toml"), "");
        let inner = outer.join("crates/sub");
        write(&inner.join("Cargo.toml"), "");
        let file = inner.join("lib.rs");
        write(&file, "");
        assert_eq!(
            detect_project_root(&file, &markers(&["Cargo.toml"]), None),
            Some(inner)
        );
    }

    #[test]
    fn a_trusted_folder_is_as_far_up_as_the_root_goes() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        write(&repo.join("package.json"), "{}");
        let app = repo.join("packages/app");
        std::fs::create_dir_all(app.join("src")).unwrap();
        let file = app.join("src/main.ts");
        let ceiling = impulse_kit::trust::normalize(&app.to_string_lossy());
        let package_json = markers(&["package.json"]);

        assert_eq!(
            detect_project_root(&file, &package_json, None),
            Some(repo.to_path_buf())
        );
        assert_eq!(
            detect_project_root(&file, &package_json, Some(&ceiling)),
            Some(app.clone()),
            "the trusted folder, in the file's own spelling"
        );
        // A marker inside the trusted folder still counts.
        write(&app.join("src/package.json"), "{}");
        assert_eq!(
            detect_project_root(&file, &package_json, Some(&ceiling)),
            Some(app.join("src"))
        );
    }

    #[test]
    fn workspace_configuration_sections() {
        assert_eq!(
            workspace_configuration_for_section("typescript")["suggest"]["autoImports"],
            true
        );
        assert_eq!(workspace_configuration_for_section("other"), json!({}));
        assert_eq!(
            default_init_options("typescript-language-server", None).unwrap()["hostInfo"],
            "Impulse"
        );
        assert!(default_init_options("rust-analyzer", None).is_none());
    }
}
