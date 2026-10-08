#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct DependencyChangesTests {
    @Test func movesAreReadFromTheReflog() {
      #expect(DependencyChanges.move(reflog: "commit: Add the cache") == .ownCommit)
      #expect(DependencyChanges.move(reflog: "commit (amend): Add the cache") == .ownCommit)
      #expect(DependencyChanges.move(reflog: "cherry-pick: Fix units") == .ownCommit)
      #expect(DependencyChanges.move(reflog: "pull: Fast-forward") == .merge)
      #expect(DependencyChanges.move(reflog: "pull --rebase (finish): returning to refs/heads/main") == .merge)
      #expect(DependencyChanges.move(reflog: "merge origin/main: Merge made by the 'ort' strategy.") == .merge)
      #expect(DependencyChanges.move(reflog: "rebase (finish): returning to refs/heads/topic") == .merge)
      #expect(DependencyChanges.move(reflog: "checkout: moving from main to upgrade") == .checkout)
      #expect(DependencyChanges.move(reflog: "reset: moving to HEAD~1") == .checkout)
    }

    @Test func rulesMatchFilesPathsAndPatterns() {
      let rules = [
        "composer.lock": "composer install", "web/package-lock.json": "cd web && npm ci", "*.gemspec": "bundle install",
        "Dockerfile": "docker compose build", "unused.lock": "nothing",
      ]
      let changed = ["composer.lock", "web/package-lock.json", "app/Http/Kernel.php", "lib/thing.gemspec"]
      let found = DependencyChanges.matches(rules: rules, changed: changed)
      #expect(found.map(\.file) == ["lib/thing.gemspec", "composer.lock", "web/package-lock.json"])
      #expect(found.map(\.command) == ["bundle install", "composer install", "cd web && npm ci"])
      #expect(DependencyChanges.matches(rules: ["a": "same", "b": "same"], changed: ["a", "b"]).count == 1)
    }

    @Test func knownDependencyFiles() {
      #expect(
        DependencyChanges.knownChanged(["src/a.ts", "package-lock.json", "docker/Dockerfile", "compose.yaml"])
          == ["package-lock.json", "docker/Dockerfile", "compose.yaml"])
    }
  }
#endif
