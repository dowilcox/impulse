//! What Impulse needs from a Docker Compose file, read line by line
//! (Compose files keep these parts simple; there's no YAML library here):
//! each service's `container_name`, `image`, published `ports` and
//! `volumes`. From those, the project setup screen suggests ports and a
//! database to clone, warns about what keeps two checkouts' stacks from
//! running at once, and each new task gets an override file that renames
//! the containers and moves the fixed ports by its slot, so the project's
//! own file never has to change.

use std::fs;

use super::task_environment::{self, PORT_RANGE};
use crate::git::task_registry::join;
use crate::git::trim_blanks;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Port {
    /// As written: `"8000:8000"`, `"${APP_PORT:-8000}:8000"`.
    pub raw: String,
    /// A fixed host port (`8000` in `8000:8000` or `127.0.0.1:8000:8000`).
    pub host: Option<i64>,
    /// The variable a host port is read from, with its default (`APP_PORT`,
    /// 8000 in `${APP_PORT:-8000}:8000`).
    pub variable: Option<String>,
    pub variable_default: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Volume {
    /// As written: `./docker/data/mysql:/var/lib/mysql`, `/app/vendor`.
    pub raw: String,
    /// The host side of a bind mount (`./docker/data/mysql`), `None`
    /// otherwise.
    pub host_path: Option<String>,
    /// The path in the container.
    pub target: String,
    /// A container path with no source: an anonymous volume, which hides
    /// the host's folder from the container.
    pub is_anonymous: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Service {
    pub name: String,
    pub container_name: Option<String>,
    pub image: Option<String>,
    pub ports: Vec<Port>,
    pub volumes: Vec<Volume>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComposeFile {
    pub services: Vec<Service>,
}

/// The names Compose looks for, in its order.
pub const FILE_NAMES: [&str; 4] = [
    "compose.yaml",
    "compose.yml",
    "docker-compose.yaml",
    "docker-compose.yml",
];

/// The Compose file in `folder`, if any: its name and contents.
pub fn find(folder: &str) -> Option<(String, String)> {
    FILE_NAMES.iter().find_map(|name| {
        let text = fs::read_to_string(join(folder, name)).ok()?;
        Some((name.to_string(), text))
    })
}

impl ComposeFile {
    pub fn parse(text: &str) -> ComposeFile {
        let mut file = ComposeFile::default();
        let mut in_services = false;
        let mut service_indent: Option<usize> = None;
        let mut key_indent: Option<usize> = None;
        let mut list_key: Option<String> = None;
        let mut current: Option<Service> = None;

        fn finish(
            file: &mut ComposeFile,
            current: &mut Option<Service>,
            key_indent: &mut Option<usize>,
            list_key: &mut Option<String>,
        ) {
            if let Some(service) = current.take() {
                file.services.push(service);
            }
            *key_indent = None;
            *list_key = None;
        }

        for raw_line in text.split('\n') {
            let line = strip_comment(raw_line);
            if trim_blanks(line).is_empty() {
                continue;
            }
            let indent = line.chars().take_while(|&c| c == ' ').count();
            let content = trim_blanks(line);
            if indent == 0 {
                finish(&mut file, &mut current, &mut key_indent, &mut list_key);
                in_services = content == "services:";
                service_indent = None;
                continue;
            }
            if !in_services {
                continue;
            }
            let service_indent = *service_indent.get_or_insert(indent);
            if indent == service_indent {
                if let Some(name) = content.strip_suffix(':') {
                    finish(&mut file, &mut current, &mut key_indent, &mut list_key);
                    current = Some(Service {
                        name: unquote(name),
                        ..Service::default()
                    });
                    continue;
                }
            }
            let Some(service) = current.as_mut() else {
                continue;
            };
            if indent <= service_indent {
                continue;
            }
            let key_indent = *key_indent.get_or_insert(indent);
            if indent == key_indent {
                list_key = None;
                let Some(colon) = content.find(':') else {
                    continue;
                };
                let key = &content[..colon];
                let value = trim_blanks(&content[colon + 1..]);
                match key {
                    "container_name" => service.container_name = Some(unquote(value)),
                    "image" => service.image = Some(unquote(value)),
                    "ports" | "volumes" => {
                        if value.starts_with('[') {
                            for item in flow_items(value) {
                                add(item, key, service);
                            }
                        } else if value.is_empty() {
                            list_key = Some(key.to_string());
                        }
                    }
                    _ => {}
                }
            } else if let (Some(key), Some(item)) = (&list_key, content.strip_prefix("- ")) {
                add(item, key, service);
            }
        }
        finish(&mut file, &mut current, &mut key_indent, &mut list_key);
        file
    }

    /// The override file that lets a task's stack run beside the main
    /// checkout's: containers renamed (`app` to `app-fix-elevation`) and
    /// fixed host ports moved by `slot` times `offset`. Ports read from
    /// variables are kept: the task's env file moves them. A fixed port
    /// that would move past 65535 isn't published in the task. `None` when
    /// nothing needs overriding. Needs Compose 2.24 or later (`!override`).
    pub fn task_override(&self, task: &str, slot: i64, offset: i64) -> Option<String> {
        let mut lines: Vec<String> = Vec::new();
        for service in &self.services {
            let mut body: Vec<String> = Vec::new();
            if let Some(name) = &service.container_name {
                body.push(format!("    container_name: {name}-{task}"));
            }
            if service.ports.iter().any(|p| p.host.is_some()) {
                let ports: Vec<String> = service
                    .ports
                    .iter()
                    .filter_map(|p| moved(p, slot, offset))
                    .collect();
                body.push(if ports.is_empty() {
                    "    ports: !override []".into()
                } else {
                    "    ports: !override".into()
                });
                body.extend(ports.iter().map(|p| format!("      - \"{p}\"")));
            }
            if body.is_empty() {
                continue;
            }
            lines.push(format!("  {}:", service.name));
            lines.extend(body);
        }
        if lines.is_empty() {
            return None;
        }
        Some(format!(
            "# Written by Impulse for the task {task} (slot {slot}), so its stack runs\n\
             # beside the main checkout's. Deleted when the task is archived.\n\
             services:\n{}\n",
            lines.join("\n")
        ))
    }
}

fn add(item: &str, key: &str, service: &mut Service) {
    let value = unquote(trim_blanks(item));
    // The long syntax (`- target: 80`) isn't read; it's left alone.
    if value.is_empty() || value.contains(": ") {
        return;
    }
    if key == "ports" {
        service.ports.push(port(&value));
    } else {
        service.volumes.push(volume(&value));
    }
}

/// A number that is a port; anything else is none.
fn port_number(text: &str) -> Option<i64> {
    text.parse::<i64>().ok().filter(|p| PORT_RANGE.contains(p))
}

/// `[ip:]host:container[/protocol]`; the host part may be a variable
/// (`${NAME}` or `${NAME:-default}`).
pub fn port(raw: &str) -> Port {
    if let Some((name, default)) = variable_head(raw) {
        return Port {
            raw: raw.to_string(),
            host: None,
            variable: Some(name.to_string()),
            variable_default: default.and_then(port_number),
        };
    }
    let parts: Vec<&str> = raw
        .split('/')
        .find(|p| !p.is_empty())
        .map(|first| first.split(':').filter(|p| !p.is_empty()).collect())
        .unwrap_or_default();
    let host = if parts.len() >= 2 {
        port_number(parts[parts.len() - 2])
    } else {
        None
    };
    Port {
        raw: raw.to_string(),
        host,
        variable: None,
        variable_default: None,
    }
}

/// The variable and default digits of a host port written
/// `${NAME}:`, `${NAME-8000}:` or `${NAME:-8000}:`.
fn variable_head(raw: &str) -> Option<(&str, Option<&str>)> {
    let rest = raw.strip_prefix("${")?;
    let name_length = rest
        .char_indices()
        .find(|&(i, c)| !(c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit())))
        .map_or(rest.len(), |(i, _)| i);
    if name_length == 0 {
        return None;
    }
    let (name, mut rest) = rest.split_at(name_length);
    let mut default = None;
    if let Some(after) = rest.strip_prefix(":-").or_else(|| rest.strip_prefix('-')) {
        let digits = after.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 {
            default = Some(&after[..digits]);
            rest = &after[digits..];
        }
    }
    rest.starts_with("}:").then_some((name, default))
}

pub fn volume(raw: &str) -> Volume {
    let parts: Vec<&str> = raw.split(':').collect();
    if parts.len() == 1 {
        return Volume {
            raw: raw.to_string(),
            host_path: None,
            target: raw.to_string(),
            is_anonymous: true,
        };
    }
    let source = parts[0];
    let is_bind = source.starts_with(['.', '/', '~', '$']);
    Volume {
        raw: raw.to_string(),
        host_path: is_bind.then(|| source.to_string()),
        target: parts[1].to_string(),
        is_anonymous: false,
    }
}

/// `port` as a task's override writes it: a fixed host port moved by
/// `slot` times `offset` (`8000:8000` to `8100:8000` in slot 1), anything
/// else as written. `None` when the moved port would be past 65535.
pub fn moved(port: &Port, slot: i64, offset: i64) -> Option<String> {
    let Some(host) = port.host else {
        return Some(port.raw.clone());
    };
    task_environment::port(host, slot, offset).map(|moved| moving(&port.raw, moved))
}

/// `8000:8000` to `8100:8000`, `127.0.0.1:8000:8000` to
/// `127.0.0.1:8100:8000`.
fn moving(raw: &str, host: i64) -> String {
    let mut parts: Vec<String> = raw.split(':').map(String::from).collect();
    if parts.len() < 2 {
        return raw.to_string();
    }
    let index = parts.len() - 2;
    parts[index] = host.to_string();
    parts.join(":")
}

/// A line without its `# comment` (a `#` inside quotes stays).
fn strip_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut previous = ' ';
    for (offset, c) in line.char_indices() {
        if let Some(open) = quote {
            if c == open {
                quote = None;
            }
        } else if c == '"' || c == '\'' {
            quote = Some(c);
        } else if c == '#' && (previous == ' ' || offset == 0) {
            return &line[..offset];
        }
        previous = c;
    }
    line
}

fn unquote(text: &str) -> String {
    let trimmed = trim_blanks(text);
    let mut chars = trimmed.chars();
    match (chars.next(), chars.next_back()) {
        (Some(first), Some(last)) if first == last && (first == '"' || first == '\'') => {
            trimmed[1..trimmed.len() - 1].to_string()
        }
        _ => trimmed.to_string(),
    }
}

fn flow_items(value: &str) -> impl Iterator<Item = &str> {
    value
        .trim_matches(['[', ']', ' '])
        .split(',')
        .filter(|item| !item.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaped like a Laravel project's dev stack.
    const LARAVEL: &str = r#"# Local development
services:
  app:
    build: .
    container_name: pulseboard-app
    ports:
      - "8000:8000"
      - "${VITE_PORT:-5173}:5173" # Vite
    volumes:
      - .:/var/www
      - /var/www/vendor
      - /var/www/node_modules
  mysql:
    image: mysql:8.4
    container_name: pulseboard-mysql
    ports: ["127.0.0.1:3306:3306"]
    volumes:
      - ./docker/data/mysql:/var/lib/mysql
  redis:
    image: "redis:8"
    ports:
      - target: 6379
        published: 6379

volumes:
  cache: {}"#;

    #[test]
    fn services_ports_and_volumes_are_read() {
        let file = ComposeFile::parse(LARAVEL);
        let names: Vec<&str> = file.services.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["app", "mysql", "redis"]);
        let app = &file.services[0];
        assert_eq!(app.container_name.as_deref(), Some("pulseboard-app"));
        assert_eq!(
            app.ports.iter().map(|p| p.host).collect::<Vec<_>>(),
            [Some(8000), None]
        );
        assert_eq!(app.ports[1].variable.as_deref(), Some("VITE_PORT"));
        assert_eq!(app.ports[1].variable_default, Some(5173));
        assert_eq!(
            app.volumes
                .iter()
                .map(|v| v.is_anonymous)
                .collect::<Vec<_>>(),
            [false, true, true]
        );
        assert_eq!(app.volumes[2].target, "/var/www/node_modules");
        let mysql = &file.services[1];
        assert_eq!(mysql.image.as_deref(), Some("mysql:8.4"));
        assert_eq!(
            mysql.ports.iter().map(|p| p.host).collect::<Vec<_>>(),
            [Some(3306)]
        );
        assert_eq!(
            mysql.volumes[0].host_path.as_deref(),
            Some("./docker/data/mysql")
        );
        assert_eq!(mysql.volumes[0].target, "/var/lib/mysql");
        assert_eq!(file.services[2].image.as_deref(), Some("redis:8"));
        assert!(
            file.services[2].ports.is_empty(),
            "the long syntax is left alone"
        );
    }

    #[test]
    fn port_variables_without_a_default() {
        let variable = port("${PORT2}:80");
        assert_eq!(variable.variable.as_deref(), Some("PORT2"));
        assert_eq!(variable.variable_default, None);
        assert_eq!(
            port("8080").host,
            None,
            "a container port alone isn't published to a fixed port"
        );
        assert_eq!(port("8000:8000/tcp").host, Some(8000));
    }

    #[test]
    fn a_tasks_override_renames_and_moves_fixed_ports() {
        let text = ComposeFile::parse(LARAVEL)
            .task_override("fix-elevation", 1, 100)
            .unwrap();
        assert!(text.ends_with(
            r#"services:
  app:
    container_name: pulseboard-app-fix-elevation
    ports: !override
      - "8100:8000"
      - "${VITE_PORT:-5173}:5173"
  mysql:
    container_name: pulseboard-mysql-fix-elevation
    ports: !override
      - "127.0.0.1:3406:3306"
"#
        ));
        assert!(text.starts_with("# Written by Impulse for the task fix-elevation (slot 1)"));
    }

    #[test]
    fn a_port_as_a_task_gets_it() {
        assert_eq!(moved(&port("8080:80"), 1, 100).as_deref(), Some("8180:80"));
        assert_eq!(
            moved(&port("127.0.0.1:3306:3306"), 2, 100).as_deref(),
            Some("127.0.0.1:3506:3306")
        );
        assert_eq!(
            moved(&port("${APP_PORT:-8000}:8000"), 1, 100).as_deref(),
            Some("${APP_PORT:-8000}:8000")
        );
        assert_eq!(moved(&port("65500:80"), 1, 100), None, "past 65535");
        assert_eq!(
            moved(&port("8080:80"), i64::MAX, i64::MAX),
            None,
            "no overflow"
        );
    }

    #[test]
    fn numbers_that_arent_ports_are_read_as_none() {
        assert_eq!(port("9223372036854775807:80").host, None);
        assert_eq!(port("99999999999999999999:80").host, None);
        assert_eq!(port("0:80").host, None);
        assert_eq!(port("70000:80").host, None);
        assert_eq!(port("${P:-9223372036854775807}:80").variable_default, None);
        assert_eq!(port("${P:-70000}:80").variable_default, None);
        assert_eq!(port("${P:-8000}:80").variable_default, Some(8000));

        // A fixed port with no room left isn't published in the task.
        let file = r#"services:
  web:
    ports:
      - "65500:80"
      - "8080:81"
  admin:
    ports:
      - "65535:80""#;
        let text = ComposeFile::parse(file).task_override("x", 1, 100).unwrap();
        assert!(text.contains(
            "  web:\n    ports: !override\n      - \"8180:81\"\n  admin:\n    ports: !override []\n"
        ));
        assert!(
            ComposeFile::parse(file)
                .task_override("x", i64::MAX, i64::MAX)
                .is_some(),
            "no overflow"
        );
    }

    #[test]
    fn nothing_to_override_when_the_file_is_ready_already() {
        let ready = "services:\n  web:\n    ports:\n      - \"${WEB_PORT:-3000}:3000\"";
        assert_eq!(ComposeFile::parse(ready).task_override("x", 1, 100), None);
    }
}
