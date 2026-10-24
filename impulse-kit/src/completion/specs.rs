//! Completion specs for common command-line tools (the Swift
//! `CompletionSpecs`): subcommands, options and what their arguments are
//! (paths, git branches, package scripts, ...), in the spirit of Fig's
//! autocomplete specs, written out by hand for the tools people run most.

use std::collections::HashMap;
use std::sync::OnceLock;

/// Where an argument's values come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionGenerator {
    Paths,
    Directories,
    GitBranches,
    GitRemotes,
    GitTags,
    /// Branches then tags.
    GitRefs,
    NpmScripts,
    MakeTargets,
    JustRecipes,
    SshHosts,
    Values(&'static [&'static str]),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionOption {
    pub names: &'static [&'static str],
    pub description: &'static str,
    /// What follows the option, when it takes a value.
    pub argument: Option<CompletionGenerator>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub subcommands: Vec<CompletionSpec>,
    pub options: Vec<CompletionOption>,
    /// Positional arguments in order; the last repeats.
    pub arguments: Vec<CompletionGenerator>,
}

impl CompletionSpec {
    /// The generator for the `index`th positional argument.
    pub fn generator(&self, index: usize) -> Option<CompletionGenerator> {
        let last = self.arguments.len().checked_sub(1)?;
        Some(self.arguments[index.min(last)])
    }

    /// The option `name` names, also when it's written `--name=value`.
    pub fn option(&self, name: &str) -> Option<&CompletionOption> {
        let bare = name.split('=').next().unwrap_or(name);
        self.options.iter().find(|o| o.names.contains(&bare))
    }

    fn subs(mut self, subcommands: Vec<CompletionSpec>) -> Self {
        self.subcommands = subcommands;
        self
    }

    fn opts(mut self, options: Vec<CompletionOption>) -> Self {
        self.options = options;
        self
    }

    fn args(mut self, arguments: Vec<CompletionGenerator>) -> Self {
        self.arguments = arguments;
        self
    }
}

impl CompletionOption {
    fn arg(mut self, argument: CompletionGenerator) -> Self {
        self.argument = Some(argument);
        self
    }
}

/// The spec for a command name (a basename, e.g. `git`), aliases included.
pub fn spec(command: &str) -> Option<&'static CompletionSpec> {
    by_name().get(command)
}

/// Every command with a spec (for command-word completion), in order.
pub fn all() -> &'static [CompletionSpec] {
    static SPECS: OnceLock<Vec<CompletionSpec>> = OnceLock::new();
    SPECS.get_or_init(specs)
}

fn by_name() -> &'static HashMap<&'static str, CompletionSpec> {
    static MAP: OnceLock<HashMap<&'static str, CompletionSpec>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut map: HashMap<&'static str, CompletionSpec> =
            all().iter().map(|spec| (spec.name, spec.clone())).collect();
        for (alias, target) in ALIASES {
            if let Some(spec) = map.get(target).cloned() {
                map.insert(
                    alias,
                    CompletionSpec {
                        name: alias,
                        ..spec
                    },
                );
            }
        }
        map
    })
}

const ALIASES: [(&str, &str); 3] = [("mosh", "ssh"), ("kubecolor", "kubectl"), ("pip3", "pip")];

type S = CompletionSpec;
type O = CompletionOption;
use CompletionGenerator::*;

fn s(name: &'static str, description: &'static str) -> S {
    S {
        name,
        description,
        subcommands: Vec::new(),
        options: Vec::new(),
        arguments: Vec::new(),
    }
}

fn o(names: &'static [&'static str], description: &'static str) -> O {
    O {
        names,
        description,
        argument: None,
    }
}

fn package_manager(name: &'static str, scripts_as_subcommands: bool) -> S {
    let mut subcommands = vec![
        s("install", "Install dependencies")
            .opts(vec![o(&["-D", "--save-dev"], "As a dev dependency")]),
        s("add", "Add a dependency").opts(vec![o(&["-D", "--dev"], "As a dev dependency")]),
        s("remove", "Remove a dependency"),
        s("run", "Run a script").args(vec![NpmScripts]),
        s("test", "Run tests"),
        s("update", "Update dependencies"),
        s("outdated", "Show outdated packages"),
        s("init", "Create package.json"),
        s("publish", "Publish the package"),
        s("exec", "Run a package binary"),
    ];
    match name {
        "npm" => subcommands.extend([
            s("i", "Install dependencies"),
            s("ci", "Clean install from the lockfile"),
            s("uninstall", "Remove a dependency"),
            s("audit", "Check for vulnerabilities").subs(vec![s("fix", "Fix what can be fixed")]),
            s("start", "Run start"),
            s("link", "Link a package"),
        ]),
        "bun" => subcommands.extend([s("x", "Run a package binary"), s("build", "Bundle")]),
        "pnpm" => subcommands.extend([
            s("dlx", "Run a package binary"),
            s("i", "Install dependencies"),
        ]),
        _ => {}
    }
    let arguments = if scripts_as_subcommands {
        vec![NpmScripts]
    } else {
        Vec::new()
    };
    s(name, "Package manager").subs(subcommands).args(arguments)
}

fn git() -> S {
    s("git", "Version control")
        .subs(vec![
            s("add", "Stage changes")
                .opts(vec![
                    o(&["-A", "--all"], "Stage everything"),
                    o(&["-p", "--patch"], "Choose hunks interactively"),
                    o(&["-u", "--update"], "Stage changes to tracked files"),
                    o(&["-N", "--intent-to-add"], "Record the path only"),
                ])
                .args(vec![Paths]),
            s("commit", "Record staged changes").opts(vec![
                o(&["-m", "--message"], "Commit message").arg(Values(&[])),
                o(&["-a", "--all"], "Stage tracked changes first"),
                o(&["--amend"], "Replace the last commit"),
                o(&["--no-verify", "-n"], "Skip hooks"),
                o(&["--fixup"], "Make a fixup! commit").arg(GitRefs),
                o(&["-s", "--signoff"], "Add Signed-off-by"),
                o(&["--allow-empty"], "Allow an empty commit"),
            ]),
            s("push", "Upload commits")
                .opts(vec![
                    o(&["-u", "--set-upstream"], "Track the remote branch"),
                    o(&["--force-with-lease"], "Force, safely"),
                    o(&["-f", "--force"], "Force"),
                    o(&["--tags"], "Push tags"),
                    o(&["-d", "--delete"], "Delete a remote branch"),
                ])
                .args(vec![GitRemotes, GitBranches]),
            s("pull", "Fetch and integrate")
                .opts(vec![
                    o(&["--rebase", "-r"], "Rebase instead of merging"),
                    o(&["--ff-only"], "Only fast-forward"),
                    o(&["--autostash"], "Stash local changes around it"),
                ])
                .args(vec![GitRemotes, GitBranches]),
            s("fetch", "Download refs and objects")
                .opts(vec![
                    o(&["--all"], "Fetch every remote"),
                    o(&["-p", "--prune"], "Remove deleted remote branches"),
                    o(&["--tags"], "Fetch tags"),
                ])
                .args(vec![GitRemotes, GitBranches]),
            s("checkout", "Switch branches or restore files")
                .opts(vec![
                    o(&["-b"], "Create and switch to a branch").arg(Values(&[])),
                    o(&["-B"], "Create or reset a branch").arg(Values(&[])),
                    o(&["--"], "Restore paths"),
                ])
                .args(vec![GitRefs]),
            s("switch", "Switch branches")
                .opts(vec![
                    o(&["-c", "--create"], "Create a branch").arg(Values(&[])),
                    o(&["-d", "--detach"], "Detach at a commit"),
                    o(&["-"], "The previous branch"),
                ])
                .args(vec![GitBranches]),
            s("branch", "List, create or delete branches")
                .opts(vec![
                    o(&["-d", "--delete"], "Delete a merged branch").arg(GitBranches),
                    o(&["-D"], "Delete a branch").arg(GitBranches),
                    o(&["-m", "--move"], "Rename a branch").arg(GitBranches),
                    o(&["-a", "--all"], "Include remote branches"),
                    o(&["-r", "--remotes"], "Remote branches"),
                    o(&["-v", "--verbose"], "Show last commit"),
                    o(&["--merged"], "Only merged branches"),
                    o(&["-u", "--set-upstream-to"], "Set the upstream").arg(GitBranches),
                ])
                .args(vec![GitBranches]),
            s("merge", "Join histories")
                .opts(vec![
                    o(&["--no-ff"], "Always make a merge commit"),
                    o(&["--squash"], "Squash into one change"),
                    o(&["--abort"], "Abort the merge"),
                    o(&["--continue"], "Continue after resolving"),
                    o(&["--ff-only"], "Only fast-forward"),
                ])
                .args(vec![GitRefs]),
            s("rebase", "Reapply commits on another base")
                .opts(vec![
                    o(&["-i", "--interactive"], "Edit the commit list"),
                    o(&["--continue"], "Continue after resolving"),
                    o(&["--abort"], "Abort the rebase"),
                    o(&["--skip"], "Skip the current commit"),
                    o(&["--onto"], "Rebase onto").arg(GitRefs),
                    o(&["--autosquash"], "Apply fixup! commits"),
                    o(&["--autostash"], "Stash local changes around it"),
                ])
                .args(vec![GitRefs]),
            s("status", "Show the working tree status").opts(vec![
                o(&["-s", "--short"], "Short format"),
                o(&["-b", "--branch"], "Show the branch"),
            ]),
            s("log", "Show commit history")
                .opts(vec![
                    o(&["--oneline"], "One line per commit"),
                    o(&["--graph"], "Draw the graph"),
                    o(&["-p", "--patch"], "Show diffs"),
                    o(&["--stat"], "Show changed files"),
                    o(&["-n", "--max-count"], "Limit commits").arg(Values(&[])),
                    o(&["--author"], "Filter by author").arg(Values(&[])),
                    o(&["--since"], "Since a date").arg(Values(&[])),
                    o(&["--all"], "Every ref"),
                    o(&["--follow"], "Follow renames"),
                ])
                .args(vec![GitRefs]),
            s("diff", "Show changes")
                .opts(vec![
                    o(&["--staged", "--cached"], "Staged changes"),
                    o(&["--stat"], "Summary"),
                    o(&["--name-only"], "File names only"),
                    o(&["-w", "--ignore-all-space"], "Ignore whitespace"),
                    o(&["--word-diff"], "Word diff"),
                ])
                .args(vec![GitRefs]),
            s("stash", "Set changes aside").subs(vec![
                s("push", "Stash changes")
                    .opts(vec![
                        o(&["-m", "--message"], "Message").arg(Values(&[])),
                        o(&["-u", "--include-untracked"], "Include untracked files"),
                        o(&["-k", "--keep-index"], "Keep staged changes"),
                    ])
                    .args(vec![Paths]),
                s("pop", "Apply and drop the latest stash"),
                s("apply", "Apply a stash"),
                s("list", "List stashes"),
                s("drop", "Delete a stash"),
                s("show", "Show a stash").opts(vec![o(&["-p"], "As a patch")]),
                s("clear", "Delete every stash"),
            ]),
            s("reset", "Move HEAD")
                .opts(vec![
                    o(&["--soft"], "Keep changes staged"),
                    o(&["--mixed"], "Keep changes unstaged"),
                    o(&["--hard"], "Discard changes"),
                ])
                .args(vec![GitRefs]),
            s("restore", "Restore files")
                .opts(vec![
                    o(&["-S", "--staged"], "Unstage"),
                    o(&["-W", "--worktree"], "Working tree"),
                    o(&["-s", "--source"], "From a commit").arg(GitRefs),
                ])
                .args(vec![Paths]),
            s("remote", "Manage remotes").subs(vec![
                s("add", "Add a remote"),
                s("remove", "Remove a remote").args(vec![GitRemotes]),
                s("rename", "Rename a remote").args(vec![GitRemotes]),
                s("set-url", "Change a remote's URL").args(vec![GitRemotes]),
                s("-v", "List with URLs"),
            ]),
            s("tag", "Create or list tags")
                .opts(vec![
                    o(&["-a", "--annotate"], "Annotated tag"),
                    o(&["-d", "--delete"], "Delete a tag").arg(GitTags),
                    o(&["-m", "--message"], "Message").arg(Values(&[])),
                    o(&["-l", "--list"], "List tags"),
                ])
                .args(vec![GitTags]),
            s("clone", "Copy a repository").opts(vec![
                o(&["--depth"], "Shallow history").arg(Values(&["1"])),
                o(&["-b", "--branch"], "Branch to check out").arg(Values(&[])),
                o(&["--recurse-submodules"], "Clone submodules too"),
            ]),
            s("cherry-pick", "Apply commits")
                .opts(vec![
                    o(&["--continue"], "Continue"),
                    o(&["--abort"], "Abort"),
                    o(&["-x"], "Note the source commit"),
                ])
                .args(vec![GitRefs]),
            s("revert", "Undo commits with new ones")
                .opts(vec![o(&["--no-edit"], "Keep the message")])
                .args(vec![GitRefs]),
            s("show", "Show an object")
                .opts(vec![o(&["--stat"], "Summary")])
                .args(vec![GitRefs]),
            s("worktree", "Manage worktrees").subs(vec![
                s("add", "Add a worktree")
                    .opts(vec![o(&["-b"], "New branch").arg(Values(&[]))])
                    .args(vec![Directories, GitRefs]),
                s("list", "List worktrees"),
                s("remove", "Remove a worktree").args(vec![Directories]),
                s("prune", "Prune stale worktrees"),
            ]),
            s("blame", "Who changed each line").args(vec![Paths]),
            s("init", "Create a repository"),
            s("grep", "Search tracked files")
                .opts(vec![o(&["-n"], "Line numbers"), o(&["-i"], "Ignore case")]),
            s("mv", "Move or rename").args(vec![Paths]),
            s("rm", "Remove files")
                .opts(vec![o(&["--cached"], "Keep the file")])
                .args(vec![Paths]),
            s("bisect", "Find a bad commit").subs(vec![
                s("start", ""),
                s("good", ""),
                s("bad", ""),
                s("reset", ""),
                s("skip", ""),
            ]),
            s("submodule", "Manage submodules").subs(vec![
                s("update", "").opts(vec![o(&["--init"], ""), o(&["--recursive"], "")]),
                s("add", ""),
                s("status", ""),
            ]),
        ])
        .opts(vec![
            o(&["-C"], "Run in a directory").arg(Directories),
            o(&["--version"], "Show the version"),
        ])
}

fn specs() -> Vec<S> {
    vec![
        git(),
        package_manager("npm", false),
        package_manager("pnpm", true),
        package_manager("yarn", true),
        package_manager("bun", true),
        s("cargo", "Rust package manager").subs(vec![
            s("build", "Compile").opts(cargo_common()),
            s("run", "Build and run").opts(cargo_common()),
            s("test", "Run tests").opts(cargo_common()),
            s("check", "Check without building").opts(cargo_common()),
            s("clippy", "Lint").opts(cargo_common()),
            s("fmt", "Format").opts(vec![o(&["--check"], "Only check")]),
            s("add", "Add a dependency"),
            s("remove", "Remove a dependency"),
            s("update", "Update the lockfile"),
            s("doc", "Build docs").opts(vec![o(&["--open"], "Open them")]),
            s("bench", "Run benchmarks"),
            s("new", "New package"),
            s("init", "Package here"),
            s("install", "Install a binary"),
            s("publish", "Publish"),
            s("clean", "Remove target/"),
            s("tree", "Dependency tree"),
        ]),
        s("make", "Build with a Makefile")
            .opts(vec![
                o(&["-j"], "Parallel jobs").arg(Values(&["4", "8"])),
                o(&["-C"], "In a directory").arg(Directories),
                o(&["-n"], "Dry run"),
                o(&["-B"], "Rebuild everything"),
            ])
            .args(vec![MakeTargets]),
        s("just", "Run a recipe")
            .opts(vec![o(&["-l", "--list"], "List recipes")])
            .args(vec![JustRecipes]),
        s("ssh", "Remote login")
            .opts(vec![
                o(&["-i"], "Identity file").arg(Paths),
                o(&["-p"], "Port").arg(Values(&[])),
                o(&["-L"], "Forward a local port").arg(Values(&[])),
                o(&["-A"], "Forward the agent"),
            ])
            .args(vec![SshHosts]),
        s("cd", "Change directory").args(vec![Directories]),
        s("docker", "Containers").subs(vec![
            s("ps", "List containers").opts(vec![o(&["-a", "--all"], "Include stopped")]),
            s("images", "List images"),
            s("run", "Run a container").opts(vec![
                o(&["-it"], "Interactive with a TTY"),
                o(&["--rm"], "Remove on exit"),
                o(&["-d", "--detach"], "In the background"),
                o(&["-p", "--publish"], "Publish a port").arg(Values(&[])),
                o(&["-v", "--volume"], "Mount a volume").arg(Values(&[])),
                o(&["-e", "--env"], "Environment variable").arg(Values(&[])),
                o(&["--name"], "Container name").arg(Values(&[])),
            ]),
            s("exec", "Run in a container").opts(vec![o(&["-it"], "Interactive with a TTY")]),
            s("build", "Build an image")
                .opts(vec![
                    o(&["-t", "--tag"], "Name and tag").arg(Values(&[])),
                    o(&["-f", "--file"], "Dockerfile").arg(Paths),
                    o(&["--no-cache"], "Don't use the cache"),
                ])
                .args(vec![Directories]),
            s("logs", "Container logs").opts(vec![o(&["-f", "--follow"], "Follow")]),
            s("stop", "Stop containers"),
            s("start", "Start containers"),
            s("restart", "Restart containers"),
            s("rm", "Remove containers"),
            s("rmi", "Remove images"),
            s("pull", "Pull an image"),
            s("push", "Push an image"),
            s("compose", "Multi-container apps").subs(vec![
                s("up", "Create and start").opts(vec![
                    o(&["-d", "--detach"], "In the background"),
                    o(&["--build"], "Build first"),
                ]),
                s("down", "Stop and remove").opts(vec![o(&["-v", "--volumes"], "Remove volumes")]),
                s("logs", "Logs").opts(vec![o(&["-f", "--follow"], "Follow")]),
                s("ps", "List"),
                s("build", "Build"),
                s("exec", "Run in a service"),
                s("restart", "Restart"),
                s("pull", "Pull images"),
            ]),
            s("system", "Docker system").subs(vec![
                s("prune", "Remove unused data"),
                s("df", "Disk usage"),
            ]),
            s("volume", "Volumes").subs(vec![s("ls", ""), s("rm", ""), s("prune", "")]),
            s("network", "Networks").subs(vec![s("ls", ""), s("rm", ""), s("create", "")]),
        ]),
        s("kubectl", "Kubernetes")
            .subs(vec![
                s("get", "List resources").args(vec![Values(&[
                    "pods",
                    "deployments",
                    "services",
                    "nodes",
                    "namespaces",
                    "ingress",
                    "configmaps",
                    "secrets",
                    "jobs",
                    "events",
                ])]),
                s("describe", "Describe a resource").args(vec![Values(&[
                    "pod",
                    "deployment",
                    "service",
                    "node",
                ])]),
                s("logs", "Pod logs").opts(vec![
                    o(&["-f", "--follow"], "Follow"),
                    o(&["--tail"], "Lines").arg(Values(&["100"])),
                ]),
                s("apply", "Apply a configuration")
                    .opts(vec![o(&["-f", "--filename"], "File").arg(Paths)]),
                s("delete", "Delete resources")
                    .opts(vec![o(&["-f", "--filename"], "File").arg(Paths)]),
                s("exec", "Run in a container").opts(vec![o(&["-it"], "Interactive with a TTY")]),
                s("port-forward", "Forward ports"),
                s("rollout", "Manage rollouts").subs(vec![
                    s("status", ""),
                    s("restart", ""),
                    s("undo", ""),
                ]),
                s("scale", "Scale a deployment"),
                s("config", "kubeconfig").subs(vec![
                    s("use-context", ""),
                    s("get-contexts", ""),
                    s("current-context", ""),
                ]),
            ])
            .opts(vec![
                o(&["-n", "--namespace"], "Namespace").arg(Values(&[])),
                o(&["-o", "--output"], "Output format")
                    .arg(Values(&["yaml", "json", "wide", "name"])),
                o(&["-A", "--all-namespaces"], "Every namespace"),
            ]),
        s("swift", "Swift").subs(vec![
            s("build", "Build")
                .opts(vec![o(&["-c", "--configuration"], "Configuration")
                    .arg(Values(&["debug", "release"]))]),
            s("test", "Run tests").opts(vec![
                o(&["--filter"], "Only matching tests").arg(Values(&[])),
                o(&["--parallel"], "In parallel"),
            ]),
            s("run", "Build and run")
                .opts(vec![o(&["-c", "--configuration"], "Configuration")
                    .arg(Values(&["debug", "release"]))]),
            s("package", "Manage the package").subs(vec![
                s("init", "New package"),
                s("update", "Update dependencies"),
                s("resolve", "Resolve dependencies"),
                s("clean", "Remove build artifacts"),
                s("reset", "Reset the cache"),
                s("dump-package", "Print the manifest"),
            ]),
        ]),
        s("brew", "Homebrew")
            .subs(vec![
                s("install", "Install"),
                s("uninstall", "Uninstall"),
                s("upgrade", "Upgrade"),
                s("update", "Update Homebrew"),
                s("search", "Search"),
                s("info", "Show details"),
                s("list", "List installed"),
                s("outdated", "Show outdated"),
                s("doctor", "Check the setup"),
                s("cleanup", "Remove old versions"),
                s("services", "Background services").subs(vec![
                    s("list", ""),
                    s("start", ""),
                    s("stop", ""),
                    s("restart", ""),
                ]),
            ])
            .opts(vec![o(&["--cask"], "macOS apps")]),
        s("gh", "GitHub CLI").subs(vec![
            s("pr", "Pull requests").subs(vec![
                s("create", "Open a pull request").opts(vec![
                    o(&["--draft"], "As a draft"),
                    o(&["--fill"], "Title and body from commits"),
                    o(&["--web"], "In the browser"),
                ]),
                s("checkout", "Check out a pull request"),
                s("list", "List pull requests"),
                s("view", "View a pull request").opts(vec![o(&["--web"], "In the browser")]),
                s("merge", "Merge a pull request").opts(vec![
                    o(&["--squash"], "Squash"),
                    o(&["--rebase"], "Rebase"),
                    o(&["--auto"], "When checks pass"),
                ]),
                s("status", "Your pull requests"),
                s("diff", "Show the diff"),
                s("checks", "CI checks"),
            ]),
            s("issue", "Issues").subs(vec![
                s("create", ""),
                s("list", ""),
                s("view", ""),
                s("close", ""),
            ]),
            s("repo", "Repositories").subs(vec![
                s("clone", ""),
                s("view", ""),
                s("create", ""),
                s("fork", ""),
            ]),
            s("run", "Workflow runs").subs(vec![
                s("list", ""),
                s("view", ""),
                s("watch", ""),
                s("rerun", ""),
            ]),
            s("auth", "Authentication").subs(vec![
                s("login", ""),
                s("status", ""),
                s("logout", ""),
            ]),
        ]),
        s("go", "Go").subs(vec![
            s("build", "Compile"),
            s("run", "Compile and run").args(vec![Paths]),
            s("test", "Test").opts(vec![
                o(&["-v"], "Verbose"),
                o(&["-run"], "Only matching tests").arg(Values(&[])),
            ]),
            s("mod", "Modules").subs(vec![
                s("tidy", "Add and remove requirements"),
                s("init", "New module"),
                s("download", ""),
            ]),
            s("get", "Add dependencies"),
            s("fmt", "Format"),
            s("vet", "Report suspicious code"),
            s("install", "Install"),
        ]),
        s("terraform", "Terraform").subs(vec![
            s("init", ""),
            s("plan", ""),
            s("apply", "").opts(vec![o(&["-auto-approve"], "Skip approval")]),
            s("destroy", ""),
            s("fmt", ""),
            s("validate", ""),
            s("output", ""),
            s("state", "").subs(vec![s("list", ""), s("show", ""), s("mv", ""), s("rm", "")]),
        ]),
        s("pip", "Python packages").subs(vec![
            s("install", "Install").opts(vec![
                o(&["-r", "--requirement"], "From a file").arg(Paths),
                o(&["-U", "--upgrade"], "Upgrade"),
                o(&["-e", "--editable"], "Editable").arg(Directories),
            ]),
            s("uninstall", "Uninstall"),
            s("list", "List installed"),
            s("freeze", "Requirements format"),
            s("show", "Show details"),
        ]),
    ]
}

fn cargo_common() -> Vec<O> {
    vec![
        o(&["--release", "-r"], "Optimized build"),
        o(&["-p", "--package"], "Package").arg(Values(&[])),
        o(&["--all-features"], "Every feature"),
        o(&["-F", "--features"], "Features").arg(Values(&[])),
        o(&["--workspace"], "Every workspace member"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_subcommand(name: &str) -> &'static CompletionSpec {
        spec("git")
            .unwrap()
            .subcommands
            .iter()
            .find(|s| s.name == name)
            .unwrap()
    }

    #[test]
    fn aliases_share_their_targets_spec() {
        let mosh = spec("mosh").unwrap();
        assert_eq!(mosh.name, "mosh");
        assert_eq!(mosh.arguments, spec("ssh").unwrap().arguments);
        assert_eq!(
            spec("pip3").unwrap().subcommands,
            spec("pip").unwrap().subcommands
        );
        assert!(all().iter().all(|s| s.name != "mosh"));
        assert!(spec("frobnicate").is_none());
    }

    #[test]
    fn options_and_positional_arguments() {
        let clone = git_subcommand("clone");
        assert_eq!(
            clone.option("--depth=1").map(|o| o.argument),
            Some(Some(Values(&["1"])))
        );
        assert!(clone.option("--nope").is_none());
        assert_eq!(clone.generator(0), None);
        let push = git_subcommand("push");
        assert_eq!(push.generator(0), Some(GitRemotes));
        assert_eq!(push.generator(5), Some(GitBranches), "the last repeats");
    }
}
