//! What the project setup screen proposes for a repository, worked out from
//! what's in it: the ignored files and folders a fresh checkout lacks, the
//! ports and per-task values in its Compose and env files, scripts from its
//! lock files and package scripts, and a database whose data folder can be
//! cloned. Everything here is a suggestion for the user to accept or edit.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use super::compose_file::{self, ComposeFile, Port, Service};
use super::config::Action;
use super::task_environment;
use crate::git::task_registry::join;
use crate::git::{last_path_component, trim_blanks};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    /// Ticked when the screen opens.
    pub suggested: bool,
    /// Why it isn't ticked, when it isn't.
    pub note: Option<String>,
}

impl Entry {
    pub fn new(path: impl Into<String>, suggested: bool) -> Self {
        Self {
            path: path.into(),
            suggested,
            note: None,
        }
    }

    fn noted(path: &str, note: &str) -> Self {
        Self {
            path: path.into(),
            suggested: false,
            note: Some(note.into()),
        }
    }
}

/// A host port the Compose file writes as a plain number (`"8080:80"`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixedPort {
    pub service: String,
    pub port: Port,
}

/// A service's `container_name`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerName {
    pub service: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectSuggestions {
    /// Ignored files to copy into tasks.
    pub copies: Vec<Entry>,
    /// Ignored folders to clone into tasks.
    pub clones: Vec<Entry>,
    /// Ports by name with the main checkout's value.
    pub ports: BTreeMap<String, i64>,
    /// Values that have to differ per task.
    pub values: BTreeMap<String, String>,
    /// What keeps two checkouts' stacks from running at once, which a
    /// task's Compose override moves and renames: fixed host ports and
    /// `container_name`.
    pub fixed_ports: Vec<FixedPort>,
    pub container_names: Vec<ContainerName>,
    pub setup: Option<String>,
    pub archive: Option<String>,
    pub check: Option<String>,
    pub actions: Vec<Action>,
    /// What to run when files change in a pull, merge or checkout.
    pub on_change: BTreeMap<String, String>,
    /// A database whose data is a folder of the project.
    pub database_folder: Option<String>,
    pub database_service: Option<String>,
    /// The Compose file, when there is one.
    pub compose_file_name: Option<String>,
}

/// Folders worth cloning when they're ignored.
const CLONE_CANDIDATES: [&str; 10] = [
    "vendor",
    "node_modules",
    "public/build",
    "dist",
    "build",
    ".venv",
    "venv",
    "target",
    "Pods",
    "storage/app",
];

/// Lock files and the install command each implies.
pub const INSTALL_COMMANDS: [(&str, &str); 9] = [
    ("package-lock.json", "npm ci"),
    ("pnpm-lock.yaml", "pnpm install --frozen-lockfile"),
    ("yarn.lock", "yarn install --immutable"),
    ("bun.lock", "bun install --frozen-lockfile"),
    ("bun.lockb", "bun install --frozen-lockfile"),
    ("composer.lock", "composer install"),
    ("Gemfile.lock", "bundle install"),
    ("uv.lock", "uv sync"),
    ("poetry.lock", "poetry install"),
];

/// Database images and where they keep their data.
const DATABASE_DATA_PATHS: [(&str, &str); 5] = [
    ("mysql", "/var/lib/mysql"),
    ("mariadb", "/var/lib/mysql"),
    ("postgres", "/var/lib/postgresql/data"),
    ("postgis", "/var/lib/postgresql/data"),
    ("mongo", "/data/db"),
];

/// Ports of servers a project's checkouts usually share rather than run one
/// each of: databases, caches, queues, search and mail catchers.
const SHARED_SERVER_PORTS: [i64; 10] =
    [1025, 2525, 3306, 5432, 5672, 6379, 7700, 9200, 11211, 27017];

/// Names that usually hold a database name in an env file.
const DATABASE_NAME_KEYS: [&str; 5] = [
    "DB_DATABASE",
    "DB_NAME",
    "DATABASE_NAME",
    "POSTGRES_DB",
    "MYSQL_DATABASE",
];

/// Suggestions for the repository at `root`; `ignored` is what git ignores
/// there (folders end in "/").
pub fn suggest(root: &str, ignored: &[String]) -> ProjectSuggestions {
    let mut suggestions = ProjectSuggestions::default();
    let exists = |relative: &str| Path::new(&join(root, relative)).exists();
    let read = |relative: &str| fs::read_to_string(join(root, relative)).ok();

    let compose = compose_file::find(root).map(|(name, text)| (name, ComposeFile::parse(&text)));
    suggestions.compose_file_name = compose.as_ref().map(|(name, _)| name.clone());
    let services: &[Service] = compose.as_ref().map_or(&[], |(_, file)| &file.services);

    // Folders the containers can't see from the host: a bind mount of the
    // project plus an anonymous volume under it (`.:/var/www` and
    // `/var/www/vendor`).
    let mut hidden_from_containers: BTreeSet<String> = BTreeSet::new();
    for service in services {
        for bind in &service.volumes {
            if !matches!(bind.host_path.as_deref(), Some(".") | Some("./")) {
                continue;
            }
            let prefix = format!("{}/", bind.target);
            for anonymous in &service.volumes {
                if let Some(inside) = anonymous.target.strip_prefix(&prefix) {
                    if anonymous.is_anonymous {
                        hidden_from_containers.insert(inside.to_string());
                    }
                }
            }
        }
    }

    // Files and folders a fresh checkout lacks.
    for entry in ignored.iter().take(500) {
        if let Some(folder) = entry.strip_suffix('/') {
            if !CLONE_CANDIDATES.contains(&folder) {
                continue;
            }
            suggestions
                .clones
                .push(if hidden_from_containers.contains(folder) {
                    Entry::noted(
                        folder,
                        "Compose hides it from the containers (an anonymous volume)",
                    )
                } else if folder == "node_modules" && exists("package-lock.json") {
                    Entry::noted(
                        folder,
                        "npm ci deletes it first; use npm install to keep a clone",
                    )
                } else {
                    Entry::new(folder, true)
                });
        } else if entry.split('/').filter(|p| !p.is_empty()).count() <= 2 {
            let name = last_path_component(entry);
            let is_env = name == ".env" || name.starts_with(".env.");
            let suggested = is_env && !name.ends_with(".example") && name != ".env.testing";
            suggestions
                .copies
                .push(Entry::new(entry.clone(), suggested));
        }
    }

    // Ports: variables in the Compose file, then `_PORT` values in .env
    // that are ports on this machine. Beside `DB_HOST=mysql`, `DB_PORT` is
    // the port inside the Compose network (or on another machine), which a
    // task's stack keeps. A well-known server's port (`DB_PORT=3306` beside
    // `DB_HOST=127.0.0.1`) is a server on this machine that every checkout
    // shares, unless the Compose file publishes it: moved, a task would
    // connect to nothing.
    for service in services {
        for port in &service.ports {
            if let (Some(variable), Some(value)) = (&port.variable, port.variable_default) {
                suggestions.ports.insert(variable.clone(), value);
            } else if port.host.is_some() {
                suggestions.fixed_ports.push(FixedPort {
                    service: service.name.clone(),
                    port: port.clone(),
                });
            }
        }
        if let Some(name) = &service.container_name {
            suggestions.container_names.push(ContainerName {
                service: service.name.clone(),
                name: name.clone(),
            });
        }
    }
    let published: BTreeSet<i64> = services
        .iter()
        .flat_map(|s| &s.ports)
        .filter_map(|p| p.host.or(p.variable_default))
        .collect();
    let env = read(".env")
        .map(|text| parse_env(&text))
        .unwrap_or_default();
    for (key, value) in &env {
        let Some(stem) = key.strip_suffix("_PORT") else {
            continue;
        };
        if suggestions.ports.contains_key(key) {
            continue;
        }
        let host = env.get(&format!("{stem}_HOST"));
        if !host.is_none_or(|h| is_this_machine(h)) {
            continue;
        }
        let Some(port) = value
            .parse::<i64>()
            .ok()
            .filter(|p| (1..=65535).contains(p))
        else {
            continue;
        };
        if SHARED_SERVER_PORTS.contains(&port) && !published.contains(&port) {
            continue;
        }
        suggestions.ports.insert(key.clone(), port);
    }

    // A database whose data is a folder of the project.
    for service in services {
        let Some(image) = service.image.as_ref().map(|i| i.to_lowercase()) else {
            continue;
        };
        let Some((_, target)) = DATABASE_DATA_PATHS
            .iter()
            .find(|(known, _)| image.starts_with(known) || image.contains(&format!("/{known}")))
        else {
            continue;
        };
        let host = service
            .volumes
            .iter()
            .find(|v| v.target == *target && v.host_path.is_some())
            .and_then(|v| v.host_path.as_deref());
        let Some(folder) = host
            .and_then(|h| h.strip_prefix("./"))
            .filter(|f| !f.is_empty())
        else {
            continue;
        };
        suggestions.database_folder = Some(folder.to_string());
        suggestions.database_service = Some(service.name.clone());
        break;
    }

    // Per-task values: a database name, unless each task runs its own
    // database (Compose has one), and an app URL on a task port.
    if suggestions.database_service.is_none() && !services.iter().any(is_database) {
        for key in DATABASE_NAME_KEYS {
            if let Some(value) = env.get(key).filter(|v| !v.is_empty()) {
                suggestions
                    .values
                    .insert(key.to_string(), format!("{value}_{{task_}}"));
            }
        }
    }
    let url_key = ["APP_URL", "BASE_URL", "SITE_URL"]
        .into_iter()
        .find(|k| env.contains_key(*k));
    let port_key = ["APP_PORT", "PORT", "WEB_PORT", "HTTP_PORT"]
        .into_iter()
        .find(|k| suggestions.ports.contains_key(*k));
    if let (Some(url_key), Some(port_key)) = (url_key, port_key) {
        suggestions.values.insert(
            url_key.to_string(),
            format!("http://localhost:{{{port_key}}}"),
        );
    }

    // Scripts.
    let mut setup: Vec<&str> = if compose.is_some() {
        vec!["docker compose up -d"]
    } else {
        Vec::new()
    };
    for (lock_file, command) in INSTALL_COMMANDS {
        if exists(lock_file) && !setup.contains(&command) {
            setup.push(command);
        }
    }
    suggestions.setup = (!setup.is_empty()).then(|| setup.join(" && "));
    suggestions.archive = compose
        .as_ref()
        .map(|_| "docker compose down -v".to_string());
    for (lock_file, command) in INSTALL_COMMANDS {
        if exists(lock_file) {
            suggestions
                .on_change
                .insert(lock_file.into(), command.into());
        }
    }
    if compose.is_some() && exists("Dockerfile") {
        // Anonymous volumes survive a rebuild: renew them, or the old
        // dependencies stay.
        suggestions.on_change.insert(
            "Dockerfile".into(),
            "docker compose build && docker compose up -d --renew-anon-volumes".into(),
        );
    }

    let package_json = read("package.json");
    let npm_scripts = package_json
        .as_deref()
        .and_then(scripts)
        .unwrap_or_default();
    let runner = script_runner(package_json.as_deref(), exists);
    let composer_scripts = read("composer.json")
        .as_deref()
        .and_then(scripts)
        .unwrap_or_default();
    let mut check: Vec<String> = Vec::new();
    for name in ["typecheck", "type-check", "check-types", "lint"] {
        if npm_scripts.iter().any(|s| s == name) {
            check.push(format!("{} {name}", runner.run));
        }
    }
    if npm_scripts.iter().any(|s| s == "test") {
        check.push(runner.test.to_string());
    }
    if composer_scripts.iter().any(|s| s == "test") {
        check.push("composer test".into());
    }
    suggestions.check = (!check.is_empty()).then(|| check.join(" && "));

    suggestions.actions = npm_scripts
        .iter()
        .take(12)
        .map(|s| Action::new(s.clone(), format!("{} {s}", runner.run)))
        .collect();
    suggestions.actions.extend(
        composer_scripts
            .iter()
            .take(6)
            .filter(|s| !s.starts_with("pre-") && !s.starts_with("post-"))
            .map(|s| Action::new(format!("composer {s}"), format!("composer {s}"))),
    );
    let targets = read("Makefile")
        .map(|text| make_targets(&text))
        .unwrap_or_default();
    suggestions.actions.extend(
        targets
            .iter()
            .take(8)
            .map(|t| Action::new(format!("make {t}"), format!("make {t}"))),
    );
    suggestions
}

/// For a new task's setup script: start its own database (Compose
/// `service`), give it two minutes to accept connections, then load a dump
/// of the main checkout's into it. The setup script is typed into the
/// user's shell, so it's one `sh -c '...'`: the same from bash, zsh or fish.
/// The task's terminal has `IMPULSE_REPO_ROOT`; credentials come from the
/// image's own variables, inside the containers (`\$`, left for the
/// container's shell). `None` for other databases, and for a service name
/// Compose wouldn't take, which could end the quotes.
pub fn dump_and_load(service: &str, image: &str) -> Option<String> {
    if !is_service_name(service) {
        return None;
    }
    let commands = database_commands(image)?;
    let exec = format!("docker compose exec -T {service} sh -c");
    Some(format!(
        "sh -c 'docker compose up -d {service} && n=0 && until {exec} \"{ready}\"; do n=$((n+1)); \
         if [ $n -ge 120 ]; then echo \"{service} did not start\" >&2; exit 1; fi; sleep 1; done \
         && (cd \"$IMPULSE_REPO_ROOT\" && {exec} \"{dump}\") | {exec} \"{load}\"'",
        ready = commands.ready,
        dump = commands.dump,
        load = commands.load,
    ))
}

/// `dump_and_load` as Impulse 0.33 wrote it (bash and zsh only, waiting
/// forever for a database that never starts), so Project Setup can replace
/// it in a saved setup script.
pub fn earlier_dump_and_load(service: &str, image: &str) -> Option<String> {
    if !is_service_name(service) {
        return None;
    }
    database_commands(image)?;
    let exec = format!("docker compose exec -T {service}");
    if image.to_lowercase().contains("postg") {
        return Some(format!(
            "until {exec} sh -c 'pg_isready -U \"$POSTGRES_USER\"'; do sleep 1; done \
             && (cd \"$IMPULSE_REPO_ROOT\" && {exec} sh -c 'pg_dumpall -U \"$POSTGRES_USER\"') \
             | {exec} sh -c 'psql -U \"$POSTGRES_USER\"'"
        ));
    }
    let auth = r#"-uroot -p"$MYSQL_ROOT_PASSWORD""#;
    Some(format!(
        "until {exec} sh -c 'mysqladmin ping {auth} --silent'; do sleep 1; done \
         && (cd \"$IMPULSE_REPO_ROOT\" && {exec} sh -c 'mysqldump {auth} --all-databases --single-transaction') \
         | {exec} sh -c 'mysql {auth}'"
    ))
}

/// A Compose service name as Compose allows them (`[a-zA-Z0-9._-]+`),
/// starting with a letter or digit so `docker compose` doesn't take it for
/// an option: nothing a shell reads specially, in or out of quotes, in any
/// shell.
pub fn is_service_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

struct DatabaseCommands {
    ready: String,
    dump: String,
    load: String,
}

/// How to tell a database image is ready, dump it and load a dump, run by
/// `sh -c "..."` in its container.
fn database_commands(image: &str) -> Option<DatabaseCommands> {
    let image = image.to_lowercase();
    if image.starts_with("mysql")
        || image.starts_with("mariadb")
        || image.contains("/mysql")
        || image.contains("/mariadb")
    {
        let auth = r#"-uroot -p\"\$MYSQL_ROOT_PASSWORD\""#;
        return Some(DatabaseCommands {
            ready: format!("mysqladmin ping {auth} --silent"),
            dump: format!("mysqldump {auth} --all-databases --single-transaction"),
            load: format!("mysql {auth}"),
        });
    }
    if image.starts_with("postgres") || image.starts_with("postgis") || image.contains("/postgres")
    {
        let user = r#"-U \"\$POSTGRES_USER\""#;
        return Some(DatabaseCommands {
            ready: format!("pg_isready {user}"),
            dump: format!("pg_dumpall {user}"),
            load: format!("psql {user}"),
        });
    }
    None
}

/// For a new task's setup script: an empty database with the project's
/// migrations and seed data, when the framework is one Impulse knows.
pub fn fresh_database(root: &str) -> Option<&'static str> {
    let exists = |relative: &str| Path::new(&join(root, relative)).exists();
    if exists("artisan") {
        Some("php artisan migrate --seed")
    } else if exists("bin/rails") {
        Some("bin/rails db:setup")
    } else if exists("manage.py") {
        Some("python manage.py migrate")
    } else {
        None
    }
}

/// The Compose service running a database, with its image.
pub fn database_service(services: &[Service]) -> Option<&Service> {
    services.iter().find(|s| is_database(s))
}

fn is_database(service: &Service) -> bool {
    let Some(image) = service.image.as_ref().map(|i| i.to_lowercase()) else {
        return false;
    };
    DATABASE_DATA_PATHS
        .iter()
        .any(|(known, _)| image.starts_with(known) || image.contains(&format!("/{known}")))
}

/// Whether a host name in an env file is this machine.
fn is_this_machine(host: &str) -> bool {
    ["localhost", "127.0.0.1", "0.0.0.0", "::1", "[::1]"].contains(&host.to_lowercase().as_str())
}

/// `KEY=value` pairs of a dotenv file (quotes removed).
pub fn parse_env(text: &str) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for line in text.split('\n') {
        let (Some(key), Some(equals)) = (task_environment::key_of(line), line.find('=')) else {
            continue;
        };
        let mut value = trim_blanks(&line[equals + 1..]);
        let mut chars = value.chars();
        if let (Some(first), Some(last)) = (chars.next(), chars.next_back()) {
            if first == last && (first == '"' || first == '\'') {
                value = &value[1..value.len() - 1];
            }
        }
        values.insert(key, value.to_string());
    }
    values
}

/// How a project runs its package.json scripts.
pub struct ScriptRunner {
    /// Runs a script by name (`npm run`).
    pub run: &'static str,
    /// Runs the `test` script.
    pub test: &'static str,
}

/// The package manager the package.json's `packageManager` field names,
/// else the one its lock file belongs to, else npm. Bun's own `bun test` is
/// its test runner, not the script.
pub fn script_runner(package_json: Option<&str>, exists: impl Fn(&str) -> bool) -> ScriptRunner {
    let declared = package_json
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .and_then(|object| {
            let field = object.get("packageManager")?.as_str()?;
            Some(field.split('@').find(|p| !p.is_empty())?.to_string())
        });
    let locks = [
        ("package-lock.json", "npm"),
        ("pnpm-lock.yaml", "pnpm"),
        ("yarn.lock", "yarn"),
        ("bun.lock", "bun"),
        ("bun.lockb", "bun"),
    ];
    let manager = declared.or_else(|| {
        locks
            .iter()
            .find(|(file, _)| exists(file))
            .map(|(_, m)| m.to_string())
    });
    match manager.as_deref() {
        Some("pnpm") => ScriptRunner {
            run: "pnpm run",
            test: "pnpm test",
        },
        Some("yarn") => ScriptRunner {
            run: "yarn run",
            test: "yarn test",
        },
        Some("bun") => ScriptRunner {
            run: "bun run",
            test: "bun run test",
        },
        _ => ScriptRunner {
            run: "npm run",
            test: "npm test",
        },
    }
}

/// The names under `"scripts"` in a package.json or composer.json, sorted.
pub fn scripts(json: &str) -> Option<Vec<String>> {
    let object: serde_json::Value = serde_json::from_str(json).ok()?;
    let scripts = object.get("scripts")?.as_object()?;
    let mut names: Vec<String> = scripts.keys().cloned().collect();
    names.sort();
    Some(names)
}

/// A Makefile's plain targets (`build:`), not variables, patterns or
/// special targets.
pub fn make_targets(text: &str) -> Vec<String> {
    text.split('\n')
        .filter_map(|line| {
            let first = line.chars().next()?;
            if !first.is_alphanumeric() {
                return None;
            }
            let colon = line.find(':')?;
            let name = &line[..colon];
            if name.contains([' ', '%', '=', '$', '.']) || line[colon..].starts_with(":=") {
                return None;
            }
            Some(name.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    fn write(root: &Path, name: &str, text: &str) {
        fs::write(root.join(name), text).unwrap();
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn map<V: Clone>(items: &[(&str, V)]) -> BTreeMap<String, V> {
        items
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    /// A Laravel project with a Docker dev stack, like PulseBoard.
    fn laravel_project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "docker-compose.yml",
            r#"services:
  app:
    container_name: pulseboard-app
    ports:
      - "8000:8000"
      - "${VITE_PORT:-5173}:5173"
    volumes:
      - .:/var/www
      - /var/www/vendor
      - /var/www/node_modules
  mysql:
    image: mysql:8.4
    ports:
      - "${FORWARD_DB_PORT:-3306}:3306"
    volumes:
      - ./docker/data/mysql:/var/lib/mysql"#,
        );
        write(
            root,
            ".env",
            "APP_URL=http://localhost:8000
APP_PORT=8000
DB_HOST=mysql
DB_PORT=3306
DB_DATABASE=pulseboard
REDIS_HOST=127.0.0.1
REDIS_PORT=nope
MAIL_HOST=smtp.example.com
MAIL_PORT=2525
REVERB_HOST=localhost
REVERB_PORT=8080
",
        );
        write(root, "composer.lock", "{}");
        write(root, "package-lock.json", "{}");
        write(
            root,
            "package.json",
            r#"{"scripts": {"dev": "vite", "build": "vite build", "test": "vitest", "typecheck": "tsc --noEmit"}}"#,
        );
        write(
            root,
            "composer.json",
            r#"{"scripts": {"test": "phpunit", "post-install-cmd": "x"}}"#,
        );
        write(
            root,
            "Makefile",
            ".PHONY: up\nup:\n\tdocker compose up -d\nVAR := 1\n%.o: %.c\nfresh: up\n",
        );
        dir
    }

    #[test]
    fn a_docker_laravel_project() {
        let dir = laravel_project();
        let found = suggest(
            &dir.path().to_string_lossy(),
            &strings(&[
                ".env",
                ".env.testing",
                "vendor/",
                "node_modules/",
                "public/build/",
                "storage/logs/laravel.log",
            ]),
        );

        assert_eq!(
            found.copies,
            [Entry::new(".env", true), Entry::new(".env.testing", false)]
        );
        let clones: Vec<&str> = found.clones.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(clones, ["vendor", "node_modules", "public/build"]);
        let ticked: Vec<bool> = found.clones.iter().map(|c| c.suggested).collect();
        assert_eq!(
            ticked,
            [false, false, true],
            "the containers can't see vendor or node_modules"
        );
        assert!(found.clones[0]
            .note
            .as_ref()
            .unwrap()
            .contains("anonymous volume"));
        assert_eq!(
            found.ports,
            map(&[
                ("VITE_PORT", 5173),
                ("FORWARD_DB_PORT", 3306),
                ("APP_PORT", 8000),
                ("REVERB_PORT", 8080)
            ]),
            "DB_PORT is inside the Compose network and MAIL_PORT on another machine: neither moves"
        );
        assert_eq!(
            found.values,
            map(&[("APP_URL", "http://localhost:{APP_PORT}".to_string())]),
            "each task runs its own database"
        );
        assert_eq!(
            found.fixed_ports,
            [FixedPort {
                service: "app".into(),
                port: compose_file::port("8000:8000")
            }]
        );
        assert_eq!(
            found.container_names,
            [ContainerName {
                service: "app".into(),
                name: "pulseboard-app".into()
            }]
        );
        assert_eq!(
            found.setup.as_deref(),
            Some("docker compose up -d && npm ci && composer install")
        );
        assert_eq!(found.archive.as_deref(), Some("docker compose down -v"));
        assert_eq!(
            found.on_change,
            map(&[
                ("package-lock.json", "npm ci".to_string()),
                ("composer.lock", "composer install".to_string())
            ])
        );
        assert_eq!(
            found.check.as_deref(),
            Some("npm run typecheck && npm test && composer test")
        );
        let actions: Vec<&str> = found.actions.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            actions,
            [
                "build",
                "dev",
                "test",
                "typecheck",
                "composer test",
                "make up",
                "make fresh"
            ]
        );
        assert_eq!(found.database_folder.as_deref(), Some("docker/data/mysql"));
        assert_eq!(found.database_service.as_deref(), Some("mysql"));
        assert_eq!(
            found.compose_file_name.as_deref(),
            Some("docker-compose.yml")
        );
    }

    #[test]
    fn a_shared_database_server_gets_a_database_per_task() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".env",
            "DB_DATABASE=trailhead\nPORT=3000\nBASE_URL=http://localhost:3000\n",
        );
        let found = suggest(&dir.path().to_string_lossy(), &[]);
        assert_eq!(
            found.values,
            map(&[("DB_DATABASE", "trailhead_{task_}".to_string())]),
            "PORT isn't read: only _PORT names"
        );
        assert_eq!(found.setup, None);
        assert_eq!(found.archive, None);
        assert_eq!(found.database_folder, None);
    }

    #[test]
    fn servers_every_checkout_shares_keep_their_ports() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        // Laravel's own: MySQL, Redis and a mail catcher on this machine.
        write(
            dir.path(),
            ".env",
            "APP_URL=http://localhost
DB_HOST=127.0.0.1
DB_PORT=3306
DB_DATABASE=laravel
REDIS_HOST=127.0.0.1
REDIS_PORT=6379
MAIL_HOST=127.0.0.1
MAIL_PORT=2525
REVERB_HOST=localhost
REVERB_PORT=8080",
        );
        let found = suggest(&root, &[]);
        assert_eq!(
            found.ports,
            map(&[("REVERB_PORT", 8080)]),
            "the servers are shared; Reverb is the app's own"
        );
        assert_eq!(
            found.values,
            map(&[("DB_DATABASE", "laravel_{task_}".to_string())])
        );

        // Published by the Compose file, the database is the project's own.
        write(
            dir.path(),
            "compose.yaml",
            "services:\n  db:\n    image: mysql:8.4\n    ports:\n      - \"3306:3306\"\n",
        );
        let found = suggest(&root, &[]);
        assert_eq!(
            found.ports,
            map(&[("DB_PORT", 3306), ("REVERB_PORT", 8080)])
        );
    }

    #[test]
    fn scripts_run_with_the_projects_package_manager() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        write(
            dir.path(),
            "package.json",
            r#"{"scripts": {"dev": "vite", "lint": "eslint .", "test": "vitest"}}"#,
        );
        write(dir.path(), "pnpm-lock.yaml", "");
        let found = suggest(&root, &[]);
        assert_eq!(found.check.as_deref(), Some("pnpm run lint && pnpm test"));
        assert_eq!(
            found.actions.first(),
            Some(&Action::new("dev", "pnpm run dev"))
        );

        fs::remove_file(dir.path().join("pnpm-lock.yaml")).unwrap();
        write(dir.path(), "bun.lock", "");
        let found = suggest(&root, &[]);
        assert_eq!(
            found.check.as_deref(),
            Some("bun run lint && bun run test"),
            "bun test is Bun's own runner"
        );

        write(
            dir.path(),
            "package.json",
            r#"{"packageManager": "yarn@4.5.0", "scripts": {"test": "vitest"}}"#,
        );
        let found = suggest(&root, &[]);
        assert_eq!(
            found.check.as_deref(),
            Some("yarn test"),
            "packageManager wins over the lock file"
        );
    }

    #[test]
    fn database_commands_for_setup() {
        assert_eq!(
            dump_and_load("mysql", "mysql:8.4").as_deref(),
            Some(
                r#"sh -c 'docker compose up -d mysql && n=0 && until docker compose exec -T mysql sh -c "mysqladmin ping -uroot -p\"\$MYSQL_ROOT_PASSWORD\" --silent"; do n=$((n+1)); if [ $n -ge 120 ]; then echo "mysql did not start" >&2; exit 1; fi; sleep 1; done && (cd "$IMPULSE_REPO_ROOT" && docker compose exec -T mysql sh -c "mysqldump -uroot -p\"\$MYSQL_ROOT_PASSWORD\" --all-databases --single-transaction") | docker compose exec -T mysql sh -c "mysql -uroot -p\"\$MYSQL_ROOT_PASSWORD\""'"#
            )
        );
        assert!(dump_and_load("db", "postgres:17")
            .unwrap()
            .contains(r#""pg_dumpall -U \"\$POSTGRES_USER\"""#));
        assert_eq!(dump_and_load("cache", "redis:8"), None);
        assert!(earlier_dump_and_load("mysql", "mysql:8.4")
            .unwrap()
            .starts_with("until docker compose exec -T mysql sh -c 'mysqladmin ping"));
        assert!(earlier_dump_and_load("db", "postgres:17")
            .unwrap()
            .contains("pg_dumpall"));
    }

    /// The service name goes into the script's single quotes, which the
    /// user's shell (bash, zsh or fish) reads: only names Compose allows.
    #[test]
    fn only_plain_service_names_get_dump_and_load() {
        for name in [
            "db'; touch PWNED; '",
            "db$(id)",
            "my db",
            "db\\",
            "-p",
            "db\"x",
            "db\nx",
            "",
        ] {
            assert_eq!(dump_and_load(name, "mysql:8.4"), None, "{name}");
            assert_eq!(earlier_dump_and_load(name, "mysql:8.4"), None, "{name}");
        }
        assert!(dump_and_load("mysql_1.db-x", "mysql:8.4")
            .unwrap()
            .contains("exec -T mysql_1.db-x sh"));
    }

    fn run_with_fake_docker(
        command: &str,
        docker: &str,
        folder: &Path,
    ) -> std::process::ExitStatus {
        let docker_path = folder.join("docker");
        fs::write(&docker_path, docker).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&docker_path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Command::new("/bin/sh")
            .args(["-c", &command.replace("sleep 1", "true")])
            .env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin", folder.display()))
            .env("IMPULSE_REPO_ROOT", folder)
            .stderr(Stdio::null())
            .status()
            .unwrap()
    }

    /// The command as the user's shell hands it to sh: the database's own
    /// variables are left for the container, and the loop gives up.
    #[test]
    fn dump_and_load_runs_under_sh() {
        let command = dump_and_load("mysql", "mysql:8.4").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().display().to_string();
        // A docker that records its arguments: up succeeds, ping fails twice.
        let docker = format!(
            r#"#!/bin/sh
echo "$@" >> "{folder}/calls"
case "$*" in
  *"up -d"*) exit 0 ;;
  *ping*) [ "$(grep -c ping "{folder}/calls")" -ge 3 ] && exit 0 || exit 1 ;;
  *) exit 0 ;;
esac
"#
        );
        assert!(run_with_fake_docker(&command, &docker, dir.path()).success());
        let calls = fs::read_to_string(dir.path().join("calls")).unwrap();
        assert!(calls.starts_with("compose up -d mysql\n"));
        assert!(calls.contains(r#"compose exec -T mysql sh -c mysqladmin ping -uroot -p"$MYSQL_ROOT_PASSWORD" --silent"#));
        assert!(
            calls.contains(r#"sh -c mysqldump -uroot -p"$MYSQL_ROOT_PASSWORD" --all-databases"#)
        );
    }

    #[test]
    fn dump_and_load_gives_up_on_a_database_that_never_starts() {
        let command = dump_and_load("mysql", "mysql:8.4").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let status = run_with_fake_docker(
            &command,
            "#!/bin/sh\ncase \"$*\" in *ping*) exit 1 ;; esac\n",
            dir.path(),
        );
        assert_eq!(status.code(), Some(1));
    }

    #[test]
    fn make_targets_and_env_values() {
        assert_eq!(
            make_targets("all: build\nbuild:\n\tcc\n.PHONY: all\nX = 1\nY := 2\n"),
            ["all", "build"]
        );
        assert_eq!(
            parse_env("A=1\nexport B=\"two words\"\n# C=3\n"),
            map(&[("A", "1".to_string()), ("B", "two words".to_string())])
        );
    }

    #[test]
    fn fresh_databases_and_the_database_service() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(fresh_database(&dir.path().to_string_lossy()), None);
        write(dir.path(), "artisan", "");
        assert_eq!(
            fresh_database(&dir.path().to_string_lossy()),
            Some("php artisan migrate --seed")
        );
        let file = ComposeFile::parse(
            "services:\n  web:\n    image: nginx\n  db:\n    image: bitnami/postgresql:17\n",
        );
        assert_eq!(
            database_service(&file.services).map(|s| s.name.as_str()),
            Some("db")
        );
    }
}
