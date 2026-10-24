//! The completion menu for the terminal input (the Swift
//! `CompletionEngine`): what can come next at the cursor. The words before
//! it are walked through the command's spec (`super::specs`), which says
//! whether a subcommand, an option or an argument value comes next, and
//! where the values come from (git branches, package scripts, make targets,
//! ssh hosts, ...). Commands without a spec complete paths.
//!
//! Paths keep their `"path"` kind; everything else is `"command"`,
//! `"subcommand"`, `"option"` or the generator's kind (`"branch"`,
//! `"script"`, ...), with a description in `detail`.

use std::path::{Path, PathBuf};

use super::input::{self, command_basename, path_executables, COMMON_COMMANDS};
use super::shell_completions;
use super::specs::{self, CompletionGenerator, CompletionOption, CompletionSpec};
use super::{CompletionCandidate, CompletionResult, TextSpan};
use crate::shell_parser::{parse_shell_input, ShellCompletionKind, ShellToken, ShellTokenRole};

/// How many candidates the menu shows by default.
pub const DEFAULT_LIMIT: usize = 50;

type Lister<'a> = Box<dyn Fn() -> Vec<String> + 'a>;
type ShellLister<'a> = Box<dyn Fn(&str) -> Vec<CompletionCandidate> + 'a>;

/// What the engine can't work out from the input alone.
pub struct CompletionContext<'a> {
    pub cwd: Option<String>,
    pub home: String,
    pub git_branches: Lister<'a>,
    pub git_remotes: Lister<'a>,
    pub git_tags: Lister<'a>,
    /// The shell's own completions for the line (opt-in), merged after the
    /// built-in ones.
    pub shell_completions: Option<ShellLister<'a>>,
}

impl<'a> CompletionContext<'a> {
    /// A context for `cwd` with the user's home folder and no git refs.
    pub fn new(cwd: Option<&str>) -> Self {
        CompletionContext {
            cwd: cwd.map(String::from),
            home: dirs::home_dir()
                .map(|h| h.to_string_lossy().into_owned())
                .unwrap_or_default(),
            git_branches: Box::new(Vec::new),
            git_remotes: Box::new(Vec::new),
            git_tags: Box::new(Vec::new),
            shell_completions: None,
        }
    }

    pub fn home(mut self, home: &str) -> Self {
        self.home = home.to_string();
        self
    }

    pub fn git_branches(mut self, list: impl Fn() -> Vec<String> + 'a) -> Self {
        self.git_branches = Box::new(list);
        self
    }

    pub fn git_remotes(mut self, list: impl Fn() -> Vec<String> + 'a) -> Self {
        self.git_remotes = Box::new(list);
        self
    }

    pub fn git_tags(mut self, list: impl Fn() -> Vec<String> + 'a) -> Self {
        self.git_tags = Box::new(list);
        self
    }

    pub fn shell_completions(
        mut self,
        list: impl Fn(&str) -> Vec<CompletionCandidate> + 'a,
    ) -> Self {
        self.shell_completions = Some(Box::new(list));
        self
    }
}

/// Candidates for the token at the end of `input`, at most `limit`, with
/// the shell's own completions after the built-in ones when the context has
/// them.
pub fn candidates(input: &str, context: &CompletionContext, limit: usize) -> CompletionResult {
    let built_in = built_in_candidates(input, context, limit);
    let Some(shell) = &context.shell_completions else {
        return built_in;
    };
    if parse_shell_input(input, input.len()).incomplete {
        return built_in;
    }
    CompletionResult {
        span: built_in.span,
        candidates: shell_completions::merge(built_in.candidates, shell(input), limit),
    }
}

fn built_in_candidates(input: &str, context: &CompletionContext, limit: usize) -> CompletionResult {
    let parsed = parse_shell_input(input, input.len());
    let completion = &parsed.completion;
    let span = completion.span;
    let result = |candidates: Vec<CompletionCandidate>| CompletionResult {
        span,
        candidates: candidates.into_iter().take(limit).collect(),
    };
    if parsed.incomplete {
        return result(Vec::new());
    }
    let prefix = completion.prefix.as_str();

    match completion.kind {
        ShellCompletionKind::Command => {
            if prefix.is_empty() {
                return result(Vec::new());
            }
            return result(commands(prefix));
        }
        ShellCompletionKind::RedirectTarget => return paths(input, context, limit, false),
        ShellCompletionKind::EnvAssignment => return result(Vec::new()),
        ShellCompletionKind::Argument => {}
    }

    let Some(spec) = completion
        .command
        .as_deref()
        .and_then(|c| specs::spec(command_basename(c)))
    else {
        return paths(input, context, limit, false);
    };

    // Walk the words before the cursor through the spec.
    let mut current: &CompletionSpec = spec;
    let mut positional = 0;
    let mut pending_option: Option<&CompletionOption> = None;
    for word in arguments_before(span, &parsed.tokens) {
        if pending_option.take().is_some() {
            continue;
        }
        if word.starts_with('-') {
            if let Some(option) = current.option(word) {
                if option.argument.is_some() && !word.contains('=') {
                    pending_option = Some(option);
                }
            }
            continue;
        }
        if positional == 0 {
            if let Some(sub) = current.subcommands.iter().find(|s| s.name == word) {
                current = sub;
                continue;
            }
        }
        positional += 1;
    }

    if let Some(generator) = pending_option.and_then(|o| o.argument) {
        return match generator {
            CompletionGenerator::Paths => paths(input, context, limit, false),
            CompletionGenerator::Directories => paths(input, context, limit, true),
            _ => result(values(generator, prefix, context, limit)),
        };
    }
    let mut out = Vec::new();
    if prefix.starts_with('-') {
        for option in &current.options {
            for name in option.names {
                if name.starts_with(prefix) && *name != prefix {
                    out.push(
                        CompletionCandidate::new(*name, *name, "option", false)
                            .with_detail(Some(option.description)),
                    );
                }
            }
        }
        return result(out);
    }
    if positional == 0 {
        for sub in &current.subcommands {
            if sub.name.starts_with(prefix) && !sub.name.starts_with('-') {
                out.push(
                    CompletionCandidate::new(sub.name, sub.name, "subcommand", false)
                        .with_detail(Some(sub.description).filter(|d| !d.is_empty())),
                );
            }
        }
    }
    match current.generator(positional) {
        Some(CompletionGenerator::Paths) => {
            out.extend(paths(input, context, limit, false).candidates);
            CompletionResult {
                span,
                candidates: out,
            }
        }
        Some(CompletionGenerator::Directories) => {
            out.extend(paths(input, context, limit, true).candidates);
            CompletionResult {
                span,
                candidates: out,
            }
        }
        Some(generator) => {
            out.extend(values(generator, prefix, context, limit));
            result(out)
        }
        // No subcommand matched and nothing is known about the arguments:
        // paths are the safest guess.
        None if out.is_empty() && current.subcommands.is_empty() => {
            paths(input, context, limit, false)
        }
        None => result(out),
    }
}

/// Words of the current command (after any pipe or `&&`) before the token
/// being completed, without the command itself.
fn arguments_before(span: TextSpan, tokens: &[ShellToken]) -> Vec<&str> {
    let mut words = Vec::new();
    for token in tokens.iter().filter(|t| t.span.start < span.start) {
        match token.role {
            ShellTokenRole::PipelineSeparator
            | ShellTokenRole::ControlOperator
            | ShellTokenRole::Command => words.clear(),
            ShellTokenRole::Argument => words.push(token.text.as_str()),
            _ => {}
        }
    }
    words
}

/// Command names starting with `prefix`: spec'd and common commands first
/// (in that order), then the PATH's executables, shortest first.
fn commands(prefix: &str) -> Vec<CompletionCandidate> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut add = |name: &str, detail: Option<&str>| {
        if name.starts_with(prefix) && seen.insert(name.to_string()) {
            out.push(CompletionCandidate::new(name, name, "command", false).with_detail(detail));
        }
    };
    for spec in specs::all() {
        add(spec.name, Some(spec.description));
    }
    for name in COMMON_COMMANDS {
        add(name, None);
    }
    for name in path_executables() {
        add(name, None);
    }
    let known =
        |c: &CompletionCandidate| c.detail.is_some() || COMMON_COMMANDS.contains(&c.value.as_str());
    // Stable: known commands keep their order.
    out.sort_by(|a, b| match (known(a), known(b)) {
        (true, true) => std::cmp::Ordering::Equal,
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (false, false) => {
            (a.value.chars().count(), &a.value).cmp(&(b.value.chars().count(), &b.value))
        }
    });
    out
}

fn paths(
    input: &str,
    context: &CompletionContext,
    limit: usize,
    directories_only: bool,
) -> CompletionResult {
    let mut result = input::complete_candidates(input, context.cwd.as_deref(), limit);
    if directories_only {
        result.candidates.retain(|c| c.is_dir);
    }
    result
}

fn values(
    generator: CompletionGenerator,
    prefix: &str,
    context: &CompletionContext,
    limit: usize,
) -> Vec<CompletionCandidate> {
    let make = |items: Vec<(String, Option<String>)>, kind: &str| -> Vec<CompletionCandidate> {
        items
            .into_iter()
            .filter(|(value, _)| value.starts_with(prefix) && value != prefix)
            .take(limit)
            .map(|(value, detail)| {
                CompletionCandidate::new(value.clone(), value, kind, false).with_detail(detail)
            })
            .collect()
    };
    let plain = |items: Vec<String>| items.into_iter().map(|v| (v, None)).collect();
    let cwd = context.cwd.as_deref();
    match generator {
        CompletionGenerator::Paths | CompletionGenerator::Directories => Vec::new(),
        CompletionGenerator::GitBranches => make(plain((context.git_branches)()), "branch"),
        CompletionGenerator::GitRemotes => make(plain((context.git_remotes)()), "remote"),
        CompletionGenerator::GitTags => make(plain((context.git_tags)()), "tag"),
        CompletionGenerator::GitRefs => {
            let branches = (context.git_branches)()
                .into_iter()
                .map(|b| (b, Some("branch".to_string())));
            let tags = (context.git_tags)()
                .into_iter()
                .map(|t| (t, Some("tag".to_string())));
            make(branches.chain(tags).collect(), "branch")
        }
        CompletionGenerator::NpmScripts => make(sources::package_scripts(cwd), "script"),
        CompletionGenerator::MakeTargets => make(plain(sources::make_targets(cwd)), "target"),
        CompletionGenerator::JustRecipes => make(sources::just_recipes(cwd), "recipe"),
        CompletionGenerator::SshHosts => make(plain(sources::ssh_hosts(&context.home)), "host"),
        CompletionGenerator::Values(list) => {
            make(plain(list.iter().map(|v| v.to_string()).collect()), "value")
        }
    }
}

/// Values read from files in the project or home folder.
pub mod sources {
    use super::*;

    /// package.json scripts (name, command), from the nearest package.json
    /// up from `cwd`, sorted by name.
    pub fn package_scripts(cwd: Option<&str>) -> Vec<(String, Option<String>)> {
        let Some(path) = nearest("package.json", cwd) else {
            return Vec::new();
        };
        let Some(object) = std::fs::read(&path)
            .ok()
            .and_then(|data| serde_json::from_slice::<serde_json::Value>(&data).ok())
        else {
            return Vec::new();
        };
        let Some(scripts) = object.get("scripts").and_then(|s| s.as_object()) else {
            return Vec::new();
        };
        let mut names: Vec<&String> = scripts.keys().collect();
        names.sort();
        names
            .into_iter()
            .map(|name| (name.clone(), scripts[name].as_str().map(String::from)))
            .collect()
    }

    /// Makefile targets (not pattern rules or special targets), in file
    /// order.
    pub fn make_targets(cwd: Option<&str>) -> Vec<String> {
        let Some(cwd) = cwd else {
            return Vec::new();
        };
        for name in ["GNUmakefile", "makefile", "Makefile"] {
            let Ok(text) = std::fs::read_to_string(Path::new(cwd).join(name)) else {
                continue;
            };
            let mut targets: Vec<String> = Vec::new();
            for line in lines(&text) {
                let Some(first) = line.chars().next() else {
                    continue;
                };
                if first.is_whitespace() || first == '#' || first == '.' {
                    continue;
                }
                let Some(colon) = line.find(':') else {
                    continue;
                };
                // Not an assignment (`x := y`, `x ::= y`).
                let after = &line[colon + 1..];
                if after.starts_with('=') || after.starts_with(":=") {
                    continue;
                }
                for target in line[..colon].split(' ').filter(|t| !t.is_empty()) {
                    if target.contains(['%', '$', '=']) || targets.iter().any(|t| t == target) {
                        continue;
                    }
                    targets.push(target.to_string());
                }
            }
            return targets;
        }
        Vec::new()
    }

    /// justfile recipes, with the comment above each as its description.
    pub fn just_recipes(cwd: Option<&str>) -> Vec<(String, Option<String>)> {
        let Some(path) = ["justfile", "Justfile", ".justfile"]
            .into_iter()
            .find_map(|name| nearest(name, cwd))
        else {
            return Vec::new();
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return Vec::new();
        };
        let mut recipes = Vec::new();
        let mut comment: Option<String> = None;
        for line in lines(&text) {
            if line.starts_with('#') {
                comment = Some(line.trim_start_matches('#').trim().to_string());
                continue;
            }
            // Whatever this line is, the comment was only for it.
            let above = comment.take();
            let Some(first) = line.chars().next() else {
                continue;
            };
            if !(first.is_alphabetic() || first == '@' || first == '_') {
                continue;
            }
            let Some(colon) = line.find(':') else {
                continue;
            };
            if line[colon + 1..].starts_with('=') {
                continue;
            }
            let head = line[..colon].trim();
            if head.starts_with("set ")
                || head.starts_with("alias ")
                || head.starts_with("export ")
                || head.contains(":=")
            {
                continue;
            }
            let name = head
                .split(' ')
                .find(|w| !w.is_empty())
                .map(|w| w.trim_matches('@'))
                .unwrap_or_default();
            if !name.is_empty() && !name.starts_with('_') {
                recipes.push((name.to_string(), above));
            }
        }
        recipes
    }

    /// `Host` names from ~/.ssh/config (wildcards and negations left out).
    pub fn ssh_hosts(home: &str) -> Vec<String> {
        let Ok(text) = std::fs::read_to_string(Path::new(home).join(".ssh/config")) else {
            return Vec::new();
        };
        let mut hosts: Vec<String> = Vec::new();
        for line in lines(&text) {
            let mut words = line.split_whitespace();
            if !words.next().is_some_and(|w| w.eq_ignore_ascii_case("host")) {
                continue;
            }
            for word in words {
                if word.contains(['*', '?']) || word.starts_with('!') {
                    continue;
                }
                if !hosts.iter().any(|h| h == word) {
                    hosts.push(word.to_string());
                }
            }
        }
        hosts
    }

    /// `name` in `cwd` or the nearest folder above it.
    pub(crate) fn nearest(name: &str, cwd: Option<&str>) -> Option<PathBuf> {
        let cwd = cwd.filter(|c| !c.is_empty())?;
        let mut dir = Some(Path::new(cwd));
        while let Some(current) = dir {
            if current.as_os_str().is_empty() {
                return None;
            }
            let path = current.join(name);
            if path.exists() {
                return Some(path);
            }
            dir = current.parent();
        }
        None
    }

    /// Lines split at any line break (`\r\n` gives an empty line between,
    /// which none of the readers minds).
    fn lines(text: &str) -> impl Iterator<Item = &str> {
        text.split([
            '\n', '\r', '\u{0B}', '\u{0C}', '\u{85}', '\u{2028}', '\u{2029}',
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn context(cwd: Option<&str>, home: &str) -> CompletionContext<'static> {
        CompletionContext::new(cwd)
            .home(home)
            .git_branches(|| vec!["main".into(), "feature/search".into(), "fix-login".into()])
            .git_remotes(|| vec!["origin".into(), "upstream".into()])
            .git_tags(|| vec!["v1.0".into(), "v1.1".into()])
    }

    fn plain() -> CompletionContext<'static> {
        context(None, "/nonexistent")
    }

    fn values(input: &str, context: &CompletionContext) -> Vec<String> {
        candidates(input, context, DEFAULT_LIMIT)
            .candidates
            .into_iter()
            .map(|c| c.value)
            .collect()
    }

    #[test]
    fn subcommands_with_descriptions() {
        let result = candidates("git ch", &plain(), DEFAULT_LIMIT);
        let names: Vec<&str> = result.candidates.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(names, ["checkout", "cherry-pick"]);
        assert_eq!(result.candidates[0].kind, "subcommand");
        assert_eq!(
            result.candidates[0].detail.as_deref(),
            Some("Switch branches or restore files")
        );
        assert_eq!(result.span, TextSpan { start: 4, end: 6 });
    }

    #[test]
    fn arguments_come_from_generators() {
        let ctx = plain();
        assert_eq!(
            values("git checkout f", &ctx),
            ["feature/search", "fix-login"]
        );
        assert_eq!(values("git push ", &ctx), ["origin", "upstream"]);
        assert_eq!(
            values("git push origin m", &ctx),
            ["main"],
            "second argument is a branch"
        );
        assert_eq!(
            values("git branch -D fe", &ctx),
            ["feature/search"],
            "option arguments"
        );
        assert_eq!(values("git stash p", &ctx), ["push", "pop"]);
        assert!(
            values("git commit -m ", &ctx).is_empty(),
            "free-form option values"
        );
        assert!(
            values("git switch -c new-thing ", &ctx).contains(&"main".to_string()),
            "then a start point"
        );
    }

    #[test]
    fn options_match_the_subcommand() {
        let result = candidates("git commit --a", &plain(), DEFAULT_LIMIT);
        let names: Vec<&str> = result.candidates.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(names, ["--all", "--amend", "--allow-empty"]);
        assert!(result.candidates.iter().all(|c| c.kind == "option"));
    }

    #[test]
    fn pipelines_start_a_fresh_command() {
        assert_eq!(values("echo hi | git sta", &plain()), ["status", "stash"]);
        assert_eq!(values("make && git rem", &plain()), ["remote"]);
    }

    #[test]
    fn command_words_prefer_known_tools() {
        assert_eq!(
            values("gi", &plain()).first().map(String::as_str),
            Some("git")
        );
        assert!(candidates("", &plain(), DEFAULT_LIMIT)
            .candidates
            .is_empty());
    }

    #[test]
    fn project_files_feed_scripts_targets_and_recipes() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"scripts": {"dev": "vite", "build": "vite build", "test": "vitest"}}"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("Makefile"),
            "CC := clang\n.PHONY: all\nall: build\nbuild test:\n\techo\n%.o: %.c\n\t$(CC)\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("justfile"),
            "set shell := [\"bash\", \"-c\"]\n# Run the app\nrun port=\"8080\":\n  echo\n_hidden:\n  echo\n@lint:\n  echo\n",
        )
        .unwrap();
        let sub = dir.path().join("src");
        fs::create_dir_all(&sub).unwrap();

        let ctx = context(sub.to_str(), "/nonexistent");
        assert_eq!(
            values("npm run ", &ctx),
            ["build", "dev", "test"],
            "nearest package.json above cwd"
        );
        let yarn = candidates("yarn d", &ctx, DEFAULT_LIMIT).candidates;
        assert_eq!(yarn.len(), 1);
        assert_eq!(yarn[0].value, "dev");
        assert_eq!(yarn[0].detail.as_deref(), Some("vite"));
        assert_eq!(
            values("make ", &context(dir.path().to_str(), "/nonexistent")),
            ["all", "build", "test"]
        );
        let recipes = candidates("just ", &ctx, DEFAULT_LIMIT).candidates;
        let names: Vec<&str> = recipes.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(names, ["run", "lint"]);
        assert_eq!(recipes[0].detail.as_deref(), Some("Run the app"));
    }

    #[test]
    fn ssh_hosts_skip_wildcards() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(home.path().join(".ssh")).unwrap();
        fs::write(
            home.path().join(".ssh/config"),
            "Host *\n  User me\nHost web db-1 *.internal\n  HostName x\n",
        )
        .unwrap();
        let ctx = context(None, home.path().to_str().unwrap());
        assert_eq!(values("ssh ", &ctx), ["web", "db-1"]);
        assert!(
            values("ssh -i ", &ctx).iter().all(|v| !v.is_empty()),
            "-i takes a path"
        );
    }

    #[test]
    fn unknown_commands_complete_paths() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("docs")).unwrap();
        fs::write(dir.path().join("notes.md"), "x").unwrap();
        let ctx = context(dir.path().to_str(), "/nonexistent");
        let result = candidates("mytool ", &ctx, DEFAULT_LIMIT);
        let names: Vec<&str> = result.candidates.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(names, ["docs/", "notes.md"]);
        assert!(result.candidates.iter().all(|c| c.kind == "path"));
        assert_eq!(values("cd ", &ctx), ["docs/"], "cd only offers folders");
    }
}
