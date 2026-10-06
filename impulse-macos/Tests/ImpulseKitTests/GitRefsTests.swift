#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct GitRefsTests {
    @Test func refNames() {
      for name in ["main", "feature/login", "v1.2.3", "release-2026.10", "fix_#12", "ünïcode"] {
        #expect(GitRefName.isValid(name), "\(name)")
      }
      for name in [
        "", "@", "-x", "/x", "x/", "a..b", "a//b", "x.lock", "x.", "a b", "a~1", "a^", "a:b", "a?", "a*",
        "a[b", "a\\b", "a@{1}", ".hidden", "x/.y", "tab\there",
      ] {
        #expect(!GitRefName.isValid(name), "\(name)")
      }
    }

    @Test func decorations() {
      let remotes = ["origin", "upstream", "my/fork"]
      #expect(RefDecoration.parse("HEAD -> main", remotes: remotes) == .head(branch: "main"))
      #expect(RefDecoration.parse("HEAD", remotes: remotes) == .detachedHead)
      #expect(RefDecoration.parse("tag: v1.0", remotes: remotes) == .tag("v1.0"))
      #expect(
        RefDecoration.parse("origin/feature/x", remotes: remotes) == .remoteBranch(remote: "origin", branch: "feature/x"))
      #expect(RefDecoration.parse("my/fork/main", remotes: remotes) == .remoteBranch(remote: "my/fork", branch: "main"))
      #expect(RefDecoration.parse("feature/x", remotes: remotes) == .localBranch("feature/x"))
      #expect(RefDecoration.parse("origin/HEAD", remotes: remotes) == .remoteBranch(remote: "origin", branch: "HEAD"))
    }

    @Test func remoteWebURLs() throws {
      let cases: [(String, String, RemoteWebURL.Host)] = [
        ("git@github.com:owner/repo.git", "https://github.com/owner/repo", .github),
        ("https://github.com/owner/repo", "https://github.com/owner/repo", .github),
        ("https://user:token@github.com/owner/repo.git/", "https://github.com/owner/repo", .github),
        ("ssh://git@github.com:22/owner/repo.git", "https://github.com/owner/repo", .github),
        ("git@gitlab.com:group/sub/repo.git", "https://gitlab.com/group/sub/repo", .gitlab),
        ("git@bitbucket.org:team/repo.git", "https://bitbucket.org/team/repo", .bitbucket),
        ("https://codeberg.org/me/repo.git", "https://codeberg.org/me/repo", .gitea),
        ("git@ssh.dev.azure.com:v3/org/proj/repo", "https://dev.azure.com/org/proj/_git/repo", .azure),
        ("https://git.example.com:8443/team/repo.git", "https://git.example.com:8443/team/repo", .other),
        ("http://intranet/team/repo.git", "http://intranet/team/repo", .other),
      ]
      for (remote, base, host) in cases {
        let url = try #require(RemoteWebURL(remote: remote), "\(remote)")
        #expect(url.base == base, "\(remote)")
        #expect(url.host == host, "\(remote)")
      }
      for remote in ["/srv/git/repo.git", "../repo", "file:///tmp/repo", "C:\\repos\\x", ""] {
        #expect(RemoteWebURL(remote: remote) == nil, "\(remote)")
      }
    }

    @Test func nextTagNames() {
      #expect(TagNameSuggestion.next(after: ["v1.4.2", "v1.4.1"]) == "v1.4.3")
      #expect(TagNameSuggestion.next(after: ["1.9"]) == "1.10")
      #expect(TagNameSuggestion.next(after: ["release-7"]) == "release-8")
      #expect(TagNameSuggestion.next(after: ["v2.0.0-rc1", "v1.9.9"]) == "v1.9.10", "pre-releases are skipped")
      #expect(TagNameSuggestion.next(after: ["latest", "v3"]) == "v4")
      #expect(TagNameSuggestion.next(after: ["latest"]) == nil)
      #expect(TagNameSuggestion.next(after: []) == nil)
    }

    @Test func remotePages() throws {
      let github = try #require(RemoteWebURL(remote: "git@github.com:o/r.git"))
      #expect(github.commit("abc")?.absoluteString == "https://github.com/o/r/commit/abc")
      #expect(github.tag("v1.0")?.absoluteString == "https://github.com/o/r/releases/tag/v1.0")
      #expect(github.branch("feature/x")?.absoluteString == "https://github.com/o/r/tree/feature/x")
      #expect(github.displayName == "GitHub")
      let gitlab = try #require(RemoteWebURL(remote: "git@gitlab.com:g/r.git"))
      #expect(gitlab.commit("abc")?.absoluteString == "https://gitlab.com/g/r/-/commit/abc")
      #expect(gitlab.tag("v1")?.absoluteString == "https://gitlab.com/g/r/-/tags/v1")
      let bitbucket = try #require(RemoteWebURL(remote: "git@bitbucket.org:t/r.git"))
      #expect(bitbucket.commit("abc")?.absoluteString == "https://bitbucket.org/t/r/commits/abc")
      let codeberg = try #require(RemoteWebURL(remote: "https://codeberg.org/m/r"))
      #expect(codeberg.displayName == "Codeberg")
      #expect(codeberg.tag("v1")?.absoluteString == "https://codeberg.org/m/r/src/tag/v1")
      // Odd characters in a tag stay inside the path.
      #expect(github.tag("a#b")?.absoluteString == "https://github.com/o/r/releases/tag/a%23b")
    }
  }
#endif
