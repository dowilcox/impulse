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
      let cases: [(String, String)] = [
        ("git@github.com:owner/repo.git", "https://github.com/owner/repo"),
        ("https://github.com/owner/repo", "https://github.com/owner/repo"),
        ("https://user:token@github.com/owner/repo.git/", "https://github.com/owner/repo"),
        ("ssh://git@github.com:22/owner/repo.git", "https://github.com/owner/repo"),
        ("git@gitlab.com:group/sub/repo.git", "https://gitlab.com/group/sub/repo"),
        ("git@git.example.edu:web/site.git", "https://git.example.edu/web/site"),
        ("https://git.example.com:8443/team/repo.git", "https://git.example.com:8443/team/repo"),
        ("http://intranet/team/repo.git", "http://intranet/team/repo"),
      ]
      for (remote, base) in cases {
        let url = try #require(RemoteWebURL(remote: remote), "\(remote)")
        #expect(url.base == base, "\(remote)")
        #expect(url.repository?.absoluteString == base, "\(remote)")
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
  }
#endif
