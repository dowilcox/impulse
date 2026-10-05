// Completion specs for common command-line tools: subcommands, options and
// what their arguments are (paths, git branches, package scripts, …), in
// the spirit of Fig's autocomplete specs, written out by hand for the tools
// people run most.

import Foundation

/// Where an argument's values come from.
public enum CompletionGenerator: Equatable, Sendable {
  case paths
  case directories
  case gitBranches
  case gitRemotes
  case gitTags
  /// Branches then tags.
  case gitRefs
  case npmScripts
  case makeTargets
  case justRecipes
  case sshHosts
  case values([String])
}

public struct CompletionOption: Sendable {
  public let names: [String]
  public let description: String
  /// What follows the option, when it takes a value.
  public let argument: CompletionGenerator?

  public init(_ names: [String], _ description: String, argument: CompletionGenerator? = nil) {
    self.names = names
    self.description = description
    self.argument = argument
  }
}

public struct CompletionSpec: Sendable {
  public let name: String
  public let description: String
  public let subcommands: [CompletionSpec]
  public let options: [CompletionOption]
  /// Positional arguments in order; the last repeats.
  public let arguments: [CompletionGenerator]

  public init(
    _ name: String, _ description: String = "", subcommands: [CompletionSpec] = [],
    options: [CompletionOption] = [], arguments: [CompletionGenerator] = []
  ) {
    self.name = name
    self.description = description
    self.subcommands = subcommands
    self.options = options
    self.arguments = arguments
  }

  /// The generator for the `index`th positional argument.
  func generator(at index: Int) -> CompletionGenerator? {
    guard !arguments.isEmpty else { return nil }
    return arguments[min(index, arguments.count - 1)]
  }

  func option(named name: String) -> CompletionOption? {
    let bare = name.split(separator: "=", maxSplits: 1).first.map(String.init) ?? name
    return options.first { $0.names.contains(bare) }
  }
}

public enum CompletionSpecs {
  public static func spec(for command: String) -> CompletionSpec? { byName[command] }

  /// Every command with a spec (for command-word completion).
  public static var all: [CompletionSpec] { specs }

  private static let byName: [String: CompletionSpec] = {
    var map: [String: CompletionSpec] = [:]
    for spec in specs { map[spec.name] = spec }
    for (alias, target) in aliases { map[alias] = map[target].map { renamed($0, alias) } }
    return map
  }()

  private static func renamed(_ spec: CompletionSpec, _ name: String) -> CompletionSpec {
    CompletionSpec(
      name, spec.description, subcommands: spec.subcommands, options: spec.options, arguments: spec.arguments)
  }

  private static let aliases = ["mosh": "ssh", "kubecolor": "kubectl", "pip3": "pip"]

  // MARK: Helpers

  private typealias S = CompletionSpec
  private typealias O = CompletionOption

  private static let help = O(["-h", "--help"], "Show help")

  // MARK: git

  private static let git = S(
    "git", "Version control",
    subcommands: [
      S("add", "Stage changes", options: [
        O(["-A", "--all"], "Stage everything"), O(["-p", "--patch"], "Choose hunks interactively"),
        O(["-u", "--update"], "Stage changes to tracked files"), O(["-N", "--intent-to-add"], "Record the path only"),
      ], arguments: [.paths]),
      S("commit", "Record staged changes", options: [
        O(["-m", "--message"], "Commit message", argument: .values([])), O(["-a", "--all"], "Stage tracked changes first"),
        O(["--amend"], "Replace the last commit"), O(["--no-verify", "-n"], "Skip hooks"),
        O(["--fixup"], "Make a fixup! commit", argument: .gitRefs), O(["-s", "--signoff"], "Add Signed-off-by"),
        O(["--allow-empty"], "Allow an empty commit"),
      ]),
      S("push", "Upload commits", options: [
        O(["-u", "--set-upstream"], "Track the remote branch"), O(["--force-with-lease"], "Force, safely"),
        O(["-f", "--force"], "Force"), O(["--tags"], "Push tags"), O(["-d", "--delete"], "Delete a remote branch"),
      ], arguments: [.gitRemotes, .gitBranches]),
      S("pull", "Fetch and integrate", options: [
        O(["--rebase", "-r"], "Rebase instead of merging"), O(["--ff-only"], "Only fast-forward"),
        O(["--autostash"], "Stash local changes around it"),
      ], arguments: [.gitRemotes, .gitBranches]),
      S("fetch", "Download refs and objects", options: [
        O(["--all"], "Fetch every remote"), O(["-p", "--prune"], "Remove deleted remote branches"),
        O(["--tags"], "Fetch tags"),
      ], arguments: [.gitRemotes, .gitBranches]),
      S("checkout", "Switch branches or restore files", options: [
        O(["-b"], "Create and switch to a branch", argument: .values([])), O(["-B"], "Create or reset a branch", argument: .values([])),
        O(["--"], "Restore paths"),
      ], arguments: [.gitRefs]),
      S("switch", "Switch branches", options: [
        O(["-c", "--create"], "Create a branch", argument: .values([])), O(["-d", "--detach"], "Detach at a commit"),
        O(["-"], "The previous branch"),
      ], arguments: [.gitBranches]),
      S("branch", "List, create or delete branches", options: [
        O(["-d", "--delete"], "Delete a merged branch", argument: .gitBranches),
        O(["-D"], "Delete a branch", argument: .gitBranches), O(["-m", "--move"], "Rename a branch", argument: .gitBranches),
        O(["-a", "--all"], "Include remote branches"), O(["-r", "--remotes"], "Remote branches"),
        O(["-v", "--verbose"], "Show last commit"), O(["--merged"], "Only merged branches"),
        O(["-u", "--set-upstream-to"], "Set the upstream", argument: .gitBranches),
      ], arguments: [.gitBranches]),
      S("merge", "Join histories", options: [
        O(["--no-ff"], "Always make a merge commit"), O(["--squash"], "Squash into one change"),
        O(["--abort"], "Abort the merge"), O(["--continue"], "Continue after resolving"),
        O(["--ff-only"], "Only fast-forward"),
      ], arguments: [.gitRefs]),
      S("rebase", "Reapply commits on another base", options: [
        O(["-i", "--interactive"], "Edit the commit list"), O(["--continue"], "Continue after resolving"),
        O(["--abort"], "Abort the rebase"), O(["--skip"], "Skip the current commit"),
        O(["--onto"], "Rebase onto", argument: .gitRefs), O(["--autosquash"], "Apply fixup! commits"),
        O(["--autostash"], "Stash local changes around it"),
      ], arguments: [.gitRefs]),
      S("status", "Show the working tree status", options: [
        O(["-s", "--short"], "Short format"), O(["-b", "--branch"], "Show the branch"),
      ]),
      S("log", "Show commit history", options: [
        O(["--oneline"], "One line per commit"), O(["--graph"], "Draw the graph"), O(["-p", "--patch"], "Show diffs"),
        O(["--stat"], "Show changed files"), O(["-n", "--max-count"], "Limit commits", argument: .values([])),
        O(["--author"], "Filter by author", argument: .values([])), O(["--since"], "Since a date", argument: .values([])),
        O(["--all"], "Every ref"), O(["--follow"], "Follow renames"),
      ], arguments: [.gitRefs]),
      S("diff", "Show changes", options: [
        O(["--staged", "--cached"], "Staged changes"), O(["--stat"], "Summary"), O(["--name-only"], "File names only"),
        O(["-w", "--ignore-all-space"], "Ignore whitespace"), O(["--word-diff"], "Word diff"),
      ], arguments: [.gitRefs]),
      S("stash", "Set changes aside", subcommands: [
        S("push", "Stash changes", options: [
          O(["-m", "--message"], "Message", argument: .values([])), O(["-u", "--include-untracked"], "Include untracked files"),
          O(["-k", "--keep-index"], "Keep staged changes"),
        ], arguments: [.paths]),
        S("pop", "Apply and drop the latest stash"), S("apply", "Apply a stash"), S("list", "List stashes"),
        S("drop", "Delete a stash"), S("show", "Show a stash", options: [O(["-p"], "As a patch")]),
        S("clear", "Delete every stash"),
      ]),
      S("reset", "Move HEAD", options: [
        O(["--soft"], "Keep changes staged"), O(["--mixed"], "Keep changes unstaged"),
        O(["--hard"], "Discard changes"),
      ], arguments: [.gitRefs]),
      S("restore", "Restore files", options: [
        O(["-S", "--staged"], "Unstage"), O(["-W", "--worktree"], "Working tree"),
        O(["-s", "--source"], "From a commit", argument: .gitRefs),
      ], arguments: [.paths]),
      S("remote", "Manage remotes", subcommands: [
        S("add", "Add a remote"), S("remove", "Remove a remote", arguments: [.gitRemotes]),
        S("rename", "Rename a remote", arguments: [.gitRemotes]), S("set-url", "Change a remote's URL", arguments: [.gitRemotes]),
        S("-v", "List with URLs"),
      ]),
      S("tag", "Create or list tags", options: [
        O(["-a", "--annotate"], "Annotated tag"), O(["-d", "--delete"], "Delete a tag", argument: .gitTags),
        O(["-m", "--message"], "Message", argument: .values([])), O(["-l", "--list"], "List tags"),
      ], arguments: [.gitTags]),
      S("clone", "Copy a repository", options: [
        O(["--depth"], "Shallow history", argument: .values(["1"])), O(["-b", "--branch"], "Branch to check out", argument: .values([])),
        O(["--recurse-submodules"], "Clone submodules too"),
      ]),
      S("cherry-pick", "Apply commits", options: [
        O(["--continue"], "Continue"), O(["--abort"], "Abort"), O(["-x"], "Note the source commit"),
      ], arguments: [.gitRefs]),
      S("revert", "Undo commits with new ones", options: [O(["--no-edit"], "Keep the message")], arguments: [.gitRefs]),
      S("show", "Show an object", options: [O(["--stat"], "Summary")], arguments: [.gitRefs]),
      S("worktree", "Manage worktrees", subcommands: [
        S("add", "Add a worktree", options: [O(["-b"], "New branch", argument: .values([]))], arguments: [.directories, .gitRefs]),
        S("list", "List worktrees"), S("remove", "Remove a worktree", arguments: [.directories]),
        S("prune", "Prune stale worktrees"),
      ]),
      S("blame", "Who changed each line", arguments: [.paths]),
      S("init", "Create a repository"),
      S("grep", "Search tracked files", options: [O(["-n"], "Line numbers"), O(["-i"], "Ignore case")]),
      S("mv", "Move or rename", arguments: [.paths]), S("rm", "Remove files", options: [O(["--cached"], "Keep the file")], arguments: [.paths]),
      S("bisect", "Find a bad commit", subcommands: [S("start"), S("good"), S("bad"), S("reset"), S("skip")]),
      S("submodule", "Manage submodules", subcommands: [S("update", options: [O(["--init"], ""), O(["--recursive"], "")]), S("add"), S("status")]),
    ], options: [O(["-C"], "Run in a directory", argument: .directories), O(["--version"], "Show the version")])

  // MARK: JavaScript

  private static func packageManager(_ name: String, scriptsAsSubcommands: Bool) -> S {
    var subcommands = [
      S("install", "Install dependencies", options: [O(["-D", "--save-dev"], "As a dev dependency")]),
      S("add", "Add a dependency", options: [O(["-D", "--dev"], "As a dev dependency")]),
      S("remove", "Remove a dependency"), S("run", "Run a script", arguments: [.npmScripts]),
      S("test", "Run tests"), S("update", "Update dependencies"), S("outdated", "Show outdated packages"),
      S("init", "Create package.json"), S("publish", "Publish the package"), S("exec", "Run a package binary"),
    ]
    if name == "npm" {
      subcommands += [
        S("i", "Install dependencies"), S("ci", "Clean install from the lockfile"), S("uninstall", "Remove a dependency"),
        S("audit", "Check for vulnerabilities", subcommands: [S("fix", "Fix what can be fixed")]), S("start", "Run start"),
        S("link", "Link a package"),
      ]
    }
    if name == "bun" { subcommands += [S("x", "Run a package binary"), S("build", "Bundle")] }
    if name == "pnpm" { subcommands += [S("dlx", "Run a package binary"), S("i", "Install dependencies")] }
    return S(name, "Package manager", subcommands: subcommands, arguments: scriptsAsSubcommands ? [.npmScripts] : [])
  }

  // MARK: Others

  private static let cargoCommon: [O] = [
    O(["--release", "-r"], "Optimized build"), O(["-p", "--package"], "Package", argument: .values([])),
    O(["--all-features"], "Every feature"), O(["-F", "--features"], "Features", argument: .values([])),
    O(["--workspace"], "Every workspace member"),
  ]

  private static let specs: [S] = [
    git,
    packageManager("npm", scriptsAsSubcommands: false),
    packageManager("pnpm", scriptsAsSubcommands: true),
    packageManager("yarn", scriptsAsSubcommands: true),
    packageManager("bun", scriptsAsSubcommands: true),
    S("cargo", "Rust package manager", subcommands: [
      S("build", "Compile", options: cargoCommon), S("run", "Build and run", options: cargoCommon),
      S("test", "Run tests", options: cargoCommon), S("check", "Check without building", options: cargoCommon),
      S("clippy", "Lint", options: cargoCommon), S("fmt", "Format", options: [O(["--check"], "Only check")]),
      S("add", "Add a dependency"), S("remove", "Remove a dependency"), S("update", "Update the lockfile"),
      S("doc", "Build docs", options: [O(["--open"], "Open them")]), S("bench", "Run benchmarks"),
      S("new", "New package"), S("init", "Package here"), S("install", "Install a binary"), S("publish", "Publish"),
      S("clean", "Remove target/"), S("tree", "Dependency tree"),
    ]),
    S("make", "Build with a Makefile", options: [
      O(["-j"], "Parallel jobs", argument: .values(["4", "8"])), O(["-C"], "In a directory", argument: .directories),
      O(["-n"], "Dry run"), O(["-B"], "Rebuild everything"),
    ], arguments: [.makeTargets]),
    S("just", "Run a recipe", options: [O(["-l", "--list"], "List recipes")], arguments: [.justRecipes]),
    S("ssh", "Remote login", options: [
      O(["-i"], "Identity file", argument: .paths), O(["-p"], "Port", argument: .values([])),
      O(["-L"], "Forward a local port", argument: .values([])), O(["-A"], "Forward the agent"),
    ], arguments: [.sshHosts]),
    S("cd", "Change directory", arguments: [.directories]),
    S("docker", "Containers", subcommands: [
      S("ps", "List containers", options: [O(["-a", "--all"], "Include stopped")]), S("images", "List images"),
      S("run", "Run a container", options: [
        O(["-it"], "Interactive with a TTY"), O(["--rm"], "Remove on exit"), O(["-d", "--detach"], "In the background"),
        O(["-p", "--publish"], "Publish a port", argument: .values([])), O(["-v", "--volume"], "Mount a volume", argument: .values([])),
        O(["-e", "--env"], "Environment variable", argument: .values([])), O(["--name"], "Container name", argument: .values([])),
      ]),
      S("exec", "Run in a container", options: [O(["-it"], "Interactive with a TTY")]),
      S("build", "Build an image", options: [
        O(["-t", "--tag"], "Name and tag", argument: .values([])), O(["-f", "--file"], "Dockerfile", argument: .paths),
        O(["--no-cache"], "Don't use the cache"),
      ], arguments: [.directories]),
      S("logs", "Container logs", options: [O(["-f", "--follow"], "Follow")]), S("stop", "Stop containers"),
      S("start", "Start containers"), S("restart", "Restart containers"), S("rm", "Remove containers"),
      S("rmi", "Remove images"), S("pull", "Pull an image"), S("push", "Push an image"),
      S("compose", "Multi-container apps", subcommands: [
        S("up", "Create and start", options: [O(["-d", "--detach"], "In the background"), O(["--build"], "Build first")]),
        S("down", "Stop and remove", options: [O(["-v", "--volumes"], "Remove volumes")]),
        S("logs", "Logs", options: [O(["-f", "--follow"], "Follow")]), S("ps", "List"), S("build", "Build"),
        S("exec", "Run in a service"), S("restart", "Restart"), S("pull", "Pull images"),
      ]),
      S("system", "Docker system", subcommands: [S("prune", "Remove unused data"), S("df", "Disk usage")]),
      S("volume", "Volumes", subcommands: [S("ls"), S("rm"), S("prune")]),
      S("network", "Networks", subcommands: [S("ls"), S("rm"), S("create")]),
    ]),
    S("kubectl", "Kubernetes", subcommands: [
      S("get", "List resources", arguments: [.values(["pods", "deployments", "services", "nodes", "namespaces", "ingress", "configmaps", "secrets", "jobs", "events"])]),
      S("describe", "Describe a resource", arguments: [.values(["pod", "deployment", "service", "node"])]),
      S("logs", "Pod logs", options: [O(["-f", "--follow"], "Follow"), O(["--tail"], "Lines", argument: .values(["100"]))]),
      S("apply", "Apply a configuration", options: [O(["-f", "--filename"], "File", argument: .paths)]),
      S("delete", "Delete resources", options: [O(["-f", "--filename"], "File", argument: .paths)]),
      S("exec", "Run in a container", options: [O(["-it"], "Interactive with a TTY")]),
      S("port-forward", "Forward ports"), S("rollout", "Manage rollouts", subcommands: [S("status"), S("restart"), S("undo")]),
      S("scale", "Scale a deployment"),
      S("config", "kubeconfig", subcommands: [S("use-context"), S("get-contexts"), S("current-context")]),
    ], options: [
      O(["-n", "--namespace"], "Namespace", argument: .values([])), O(["-o", "--output"], "Output format", argument: .values(["yaml", "json", "wide", "name"])),
      O(["-A", "--all-namespaces"], "Every namespace"),
    ]),
    S("swift", "Swift", subcommands: [
      S("build", "Build", options: [O(["-c", "--configuration"], "Configuration", argument: .values(["debug", "release"]))]),
      S("test", "Run tests", options: [O(["--filter"], "Only matching tests", argument: .values([])), O(["--parallel"], "In parallel")]),
      S("run", "Build and run", options: [O(["-c", "--configuration"], "Configuration", argument: .values(["debug", "release"]))]),
      S("package", "Manage the package", subcommands: [
        S("init", "New package"), S("update", "Update dependencies"), S("resolve", "Resolve dependencies"),
        S("clean", "Remove build artifacts"), S("reset", "Reset the cache"), S("dump-package", "Print the manifest"),
      ]),
    ]),
    S("brew", "Homebrew", subcommands: [
      S("install", "Install"), S("uninstall", "Uninstall"), S("upgrade", "Upgrade"), S("update", "Update Homebrew"),
      S("search", "Search"), S("info", "Show details"), S("list", "List installed"), S("outdated", "Show outdated"),
      S("doctor", "Check the setup"), S("cleanup", "Remove old versions"),
      S("services", "Background services", subcommands: [S("list"), S("start"), S("stop"), S("restart")]),
    ], options: [O(["--cask"], "macOS apps")]),
    S("gh", "GitHub CLI", subcommands: [
      S("pr", "Pull requests", subcommands: [
        S("create", "Open a pull request", options: [O(["--draft"], "As a draft"), O(["--fill"], "Title and body from commits"), O(["--web"], "In the browser")]),
        S("checkout", "Check out a pull request"), S("list", "List pull requests"), S("view", "View a pull request", options: [O(["--web"], "In the browser")]),
        S("merge", "Merge a pull request", options: [O(["--squash"], "Squash"), O(["--rebase"], "Rebase"), O(["--auto"], "When checks pass")]),
        S("status", "Your pull requests"), S("diff", "Show the diff"), S("checks", "CI checks"),
      ]),
      S("issue", "Issues", subcommands: [S("create"), S("list"), S("view"), S("close")]),
      S("repo", "Repositories", subcommands: [S("clone"), S("view"), S("create"), S("fork")]),
      S("run", "Workflow runs", subcommands: [S("list"), S("view"), S("watch"), S("rerun")]),
      S("auth", "Authentication", subcommands: [S("login"), S("status"), S("logout")]),
    ]),
    S("go", "Go", subcommands: [
      S("build", "Compile"), S("run", "Compile and run", arguments: [.paths]), S("test", "Test", options: [O(["-v"], "Verbose"), O(["-run"], "Only matching tests", argument: .values([]))]),
      S("mod", "Modules", subcommands: [S("tidy", "Add and remove requirements"), S("init", "New module"), S("download")]),
      S("get", "Add dependencies"), S("fmt", "Format"), S("vet", "Report suspicious code"), S("install", "Install"),
    ]),
    S("terraform", "Terraform", subcommands: [
      S("init"), S("plan"), S("apply", options: [O(["-auto-approve"], "Skip approval")]), S("destroy"), S("fmt"), S("validate"),
      S("output"), S("state", subcommands: [S("list"), S("show"), S("mv"), S("rm")]),
    ]),
    S("pip", "Python packages", subcommands: [
      S("install", "Install", options: [O(["-r", "--requirement"], "From a file", argument: .paths), O(["-U", "--upgrade"], "Upgrade"), O(["-e", "--editable"], "Editable", argument: .directories)]),
      S("uninstall", "Uninstall"), S("list", "List installed"), S("freeze", "Requirements format"), S("show", "Show details"),
    ]),
  ]
}
