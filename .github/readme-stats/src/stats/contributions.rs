use super::{Commit, PageInfo, RawCommit, Stats};
use crate::github::Api;
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

pub(super) const FORKS: &str = r#"
query($login: String!, $cursor: String) {
  user(login: $login) {
    repositories(first: 100, after: $cursor, isFork: true, ownerAffiliations: OWNER) {
      nodes { id databaseId isFork pushedAt }
      pageInfo { hasNextPage endCursor }
    }
  }
}"#;

const BRANCHES: &str = r#"
query($repo: ID!, $cursor: String) {
  node(id: $repo) { ... on Repository {
    refs(first: 100, after: $cursor, refPrefix: "refs/heads/") {
      nodes { target { oid } }
      pageInfo { hasNextPage endCursor }
    }
  } }
}"#;

const BRANCH_COMMITS: &str = r#"
query($repo: ID!, $revision: GitObjectID!, $author: ID!, $since: GitTimestamp!,
      $until: GitTimestamp!, $cursor: String) {
  node(id: $repo) { ... on Repository { object(oid: $revision) { ... on Commit {
    history(first: 100, after: $cursor, since: $since, until: $until, author: {id: $author}) {
      nodes { oid committedDate additions deletions parents { totalCount } }
      pageInfo { hasNextPage endCursor }
    }
  } } } }
}"#;

pub(super) const PULL_REQUESTS: &str = r#"
query($login: String!, $cursor: String) {
  user(login: $login) {
    pullRequests(first: 100, after: $cursor, orderBy: {field: UPDATED_AT, direction: DESC}) {
      nodes {
        id createdAt updatedAt state
        repository { id databaseId isFork }
        headRepository { id databaseId isFork }
      }
      pageInfo { hasNextPage endCursor }
    }
  }
}"#;

const PR_COMMITS: &str = r#"
query($pr: ID!, $cursor: String) {
  node(id: $pr) { ... on PullRequest {
    commits(first: 100, after: $cursor) {
      nodes { commit {
        oid committedDate additions deletions parents { totalCount }
        author { user { id } }
      } }
      pageInfo { hasNextPage endCursor }
    }
  } }
}"#;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Repo {
    id: String,
    database_id: u64,
    is_fork: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullRequest {
    id: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    state: PullRequestState,
    repository: Repo,
    head_repository: Option<Repo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum PullRequestState {
    Open,
    Merged,
    Closed,
}

/// Fetch only identifiers, commit metadata and file types; never publish names.
fn pages(
    api: &mut impl Api,
    query: &str,
    mut variables: Value,
    path: &str,
    stop_before: Option<DateTime<Utc>>,
) -> Result<Vec<Value>> {
    let mut output = Vec::new();
    let mut cursor = None;
    loop {
        variables["cursor"] = json!(cursor);
        let response = api.query(query, variables.clone())?;
        let connection = response
            .pointer(path)
            .context("Missing GitHub connection")?;
        let nodes = connection["nodes"]
            .as_array()
            .context("Missing GitHub nodes")?;
        for node in nodes {
            if let Some(since) = stop_before {
                let updated: DateTime<Utc> = serde_json::from_value(node["updatedAt"].clone())?;
                if updated < since {
                    return Ok(output);
                }
            }
            output.push(node.clone());
        }
        let page: PageInfo = serde_json::from_value(connection["pageInfo"].clone())?;
        cursor = page.next(&cursor)?;
        if cursor.is_none() {
            return Ok(output);
        }
    }
}

struct Collector<'a> {
    now: DateTime<Utc>,
    stats: &'a mut Stats,
    index: HashMap<String, usize>,
    fork_sources: BTreeMap<String, u64>,
}

impl Collector<'_> {
    fn record(&mut self, commit: RawCommit, repo: &Repo, from_fork: bool, from_pr: bool) {
        if commit.committed_date < self.now - Duration::days(30)
            || commit.committed_date > self.now
            || commit.parents.total_count > 1
        {
            return;
        }
        let oid = commit.oid;
        let index = *self.index.entry(oid.clone()).or_insert_with(|| {
            let index = self.stats.commits.len();
            self.stats.commits.push(Commit {
                oid: oid.clone(),
                date: commit.committed_date,
                repo: repo.id.clone(),
                additions: commit.additions,
                deletions: commit.deletions,
                from_fork: false,
                from_pr: false,
            });
            index
        });
        self.stats.commits[index].from_fork |= from_fork;
        self.stats.commits[index].from_pr |= from_pr;
        if from_fork {
            self.fork_sources.entry(oid).or_insert(repo.database_id);
        }
    }
}

pub(super) fn collect(
    api: &mut impl Api,
    login: &str,
    author: &str,
    now: DateTime<Utc>,
    stats: &mut Stats,
) -> Result<()> {
    let index = stats
        .commits
        .iter()
        .enumerate()
        .map(|(i, c)| (c.oid.clone(), i))
        .collect();
    let mut collector = Collector {
        now,
        stats,
        index,
        fork_sources: BTreeMap::new(),
    };
    let since = now - Duration::days(30);
    eprintln!("Reading owned fork branches and recently updated pull requests");
    for fork in pages(
        api,
        FORKS,
        json!({"login": login}),
        "/user/repositories",
        None,
    )? {
        let pushed: Option<DateTime<Utc>> = serde_json::from_value(fork["pushedAt"].clone())?;
        if pushed.is_none_or(|date| date < since) {
            continue;
        }
        let repo: Repo = serde_json::from_value(fork)?;
        for branch in pages(api, BRANCHES, json!({"repo": repo.id}), "/node/refs", None)? {
            let commits = pages(
                api,
                BRANCH_COMMITS,
                json!({"repo": repo.id, "revision": branch["target"]["oid"], "author": author,
                       "since": since, "until": now}),
                "/node/object/history",
                None,
            )?;
            for commit in commits {
                collector.record(serde_json::from_value(commit)?, &repo, true, false);
            }
        }
    }

    let mut opened = 0;
    let mut seen_prs = HashSet::new();
    for pr in pages(
        api,
        PULL_REQUESTS,
        json!({"login": login}),
        "/user/pullRequests",
        Some(since),
    )? {
        let pr: PullRequest = serde_json::from_value(pr)?;
        if !seen_prs.insert(pr.id.clone()) {
            continue;
        }
        if pr.updated_at < since {
            continue;
        }
        let fork =
            pr.repository.is_fork || pr.head_repository.as_ref().is_some_and(|repo| repo.is_fork);
        if pr.created_at >= since && pr.created_at <= now {
            opened += 1;
            let state = match pr.state {
                PullRequestState::Open => 0,
                PullRequestState::Merged => 1,
                PullRequestState::Closed => 2,
            };
            collector.stats.pr_states[state] += 1;
            if fork {
                collector.stats.fork_prs += 1;
            }
        }
        let source = pr.head_repository.as_ref().unwrap_or(&pr.repository);
        for node in pages(api, PR_COMMITS, json!({"pr": pr.id}), "/node/commits", None)? {
            let commit = &node["commit"];
            if commit.pointer("/author/user/id").and_then(Value::as_str) != Some(author) {
                continue;
            }
            collector.record(serde_json::from_value(commit.clone())?, source, fork, true);
        }
    }

    collector
        .stats
        .contributions
        .total_pull_request_contributions = opened;
    eprintln!(
        "Reading changed-file languages for {} unique fork commits",
        collector.fork_sources.len()
    );
    for (oid, repo) in &collector.fork_sources {
        let mut page = 1;
        loop {
            let response = api.get(&format!(
                "/repositories/{repo}/commits/{oid}?per_page=100&page={page}"
            ))?;
            let files = response["files"]
                .as_array()
                .context("Missing commit file details")?;
            for file in files {
                let path = file["filename"]
                    .as_str()
                    .context("Missing changed-file path")?;
                let additions = file["additions"]
                    .as_u64()
                    .context("Missing file additions")?;
                let deletions = file["deletions"]
                    .as_u64()
                    .context("Missing file deletions")?;
                if additions + deletions > 0 {
                    *collector
                        .stats
                        .fork_languages
                        .entry(language(path).into())
                        .or_default() += additions + deletions;
                }
            }
            if files.len() < 100 {
                break;
            }
            page += 1;
        }
    }
    Ok(())
}

fn language(path: &str) -> &'static str {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name {
        "Dockerfile" => return "Dockerfile",
        "Makefile" => return "Makefile",
        "Cargo.lock" => return "TOML",
        ".gitignore" | ".gitattributes" => return "Configuration",
        _ => {}
    }
    match name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "rs" => "Rust",
        "ts" | "tsx" | "mts" | "cts" => "TypeScript",
        "js" | "jsx" | "mjs" | "cjs" => "JavaScript",
        "py" | "pyi" => "Python",
        "go" => "Go",
        "c" | "h" => "C",
        "cpp" | "cc" | "cxx" | "hpp" => "C++",
        "cs" => "C#",
        "java" => "Java",
        "kt" | "kts" => "Kotlin",
        "swift" => "Swift",
        "rb" => "Ruby",
        "php" => "PHP",
        "lua" => "Lua",
        "sh" | "bash" | "zsh" => "Shell",
        "html" | "htm" => "HTML",
        "css" | "scss" | "sass" | "less" => "CSS",
        "vue" => "Vue",
        "svelte" => "Svelte",
        "sql" => "SQL",
        "toml" => "TOML",
        "json" | "jsonc" | "json5" => "JSON",
        "yml" | "yaml" => "YAML",
        "md" | "mdx" | "rst" => "Documentation",
        "svg" | "xml" => "XML",
        "dart" => "Dart",
        "ex" | "exs" => "Elixir",
        "zig" => "Zig",
        "nix" => "Nix",
        _ => "Other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_fork_and_pr_work_once_with_paginated_file_languages() {
        struct Fake {
            now: DateTime<Utc>,
            rest_calls: usize,
        }
        fn connection(nodes: Vec<Value>, cursor: Option<&str>) -> Value {
            json!({"nodes":nodes,"pageInfo":{"hasNextPage":cursor.is_some(),"endCursor":cursor}})
        }
        impl Api for Fake {
            fn query(&mut self, query: &str, variables: Value) -> Result<Value> {
                let repo =
                    json!({"id":"fork-id","databaseId":11,"isFork":true,"pushedAt":self.now});
                let base = json!({"id":"upstream-id","databaseId":22,"isFork":false});
                if query == FORKS {
                    return Ok(json!({"user":{"repositories":connection(vec![repo],None)}}));
                }
                if query == BRANCHES {
                    return Ok(
                        json!({"node":{"refs":connection(vec![json!({"target":{"oid":"head"}})],None)}}),
                    );
                }
                let commit = |oid: &str, author: &str| {
                    json!({"oid":oid,"committedDate":self.now,
                    "additions":101,"deletions":1,"parents":{"totalCount":1},"author":{"user":{"id":author}}})
                };
                if query == BRANCH_COMMITS {
                    return Ok(
                        json!({"node":{"object":{"history":connection(vec![commit("shared","me")],None)}}}),
                    );
                }
                if query == PULL_REQUESTS {
                    let next = if variables["cursor"].is_null() {
                        Some("next-pr")
                    } else {
                        None
                    };
                    return Ok(json!({"user":{"pullRequests":connection(vec![json!({
                            "id":if variables["cursor"].is_null() {"pr-1"} else {"pr-2"},"createdAt":self.now,"updatedAt":self.now,"state":if variables["cursor"].is_null() {"OPEN"} else {"MERGED"},
                        "repository":base,"headRepository":repo
                    })],next)}}));
                }
                assert_eq!(query, PR_COMMITS);
                Ok(
                    json!({"node":{"commits":connection(vec![json!({"commit":commit("shared","me")}),
                    json!({"commit":commit("pr-only","me")}),json!({"commit":commit("foreign","someone-else")})],None)}}),
                )
            }
            fn get(&mut self, path: &str) -> Result<Value> {
                self.rest_calls += 1;
                assert!(path.starts_with("/repositories/11/commits/"));
                if path.ends_with("page=1") {
                    Ok(
                        json!({"files":(0..100).map(|i|json!({"filename":format!("private-project/module{i}.rs"),"additions":1,"deletions":0})).collect::<Vec<_>>()}),
                    )
                } else {
                    assert!(path.ends_with("page=2"));
                    Ok(
                        json!({"files":[{"filename":"private-project/ui.tsx","additions":1,"deletions":1}]}),
                    )
                }
            }
        }
        let now = "2026-11-12T12:00:00Z".parse().unwrap();
        let mut api = Fake { now, rest_calls: 0 };
        let mut stats = Stats::default();
        collect(&mut api, "gabref", "me", now, &mut stats).unwrap();
        assert_eq!(stats.commits.len(), 2);
        assert!(stats.commits.iter().all(|c| c.from_fork && c.from_pr));
        assert_eq!(stats.fork_prs, 2);
        assert_eq!(stats.contributions.total_pull_request_contributions, 2);
        assert_eq!(stats.pr_states, [1, 1, 0]);
        assert_eq!(stats.fork_languages["Rust"], 200);
        assert_eq!(stats.fork_languages["TypeScript"], 4);
        assert_eq!(api.rest_calls, 4);
        for (_, card) in super::super::assets(&stats, now, chrono_tz::Europe::Berlin) {
            assert!(!card.contains("private-project"));
            assert!(!card.contains("fork-id"));
            assert!(!card.contains("someone-else"));
        }
    }

    #[test]
    fn attributes_changed_files_without_revealing_paths() {
        assert_eq!(language("private/customer-secret/src/lib.rs"), "Rust");
        assert_eq!(language("ui/App.tsx"), "TypeScript");
        assert_eq!(language("Cargo.lock"), "TOML");
        assert_eq!(language(".github/workflows/main.yml"), "YAML");
        assert_eq!(language("unknown.asset"), "Other");
    }

    #[test]
    fn records_duplicate_pr_commits_once_and_preserves_source_flags() {
        let now = "2026-11-12T12:00:00Z".parse().unwrap();
        let mut stats = Stats::default();
        let repo = Repo {
            id: "private-id".into(),
            database_id: 123,
            is_fork: true,
        };
        let mut collector = Collector {
            now,
            stats: &mut stats,
            index: HashMap::new(),
            fork_sources: BTreeMap::new(),
        };
        for (fork, pr) in [(true, false), (true, true)] {
            collector.record(
                serde_json::from_value(json!({"oid":"same", "committedDate":now,
                "additions":12,"deletions":4,"parents":{"totalCount":1}}))
                .unwrap(),
                &repo,
                fork,
                pr,
            );
        }
        assert_eq!(collector.stats.commits.len(), 1);
        assert!(collector.stats.commits[0].from_fork);
        assert!(collector.stats.commits[0].from_pr);
        assert_eq!(collector.fork_sources.len(), 1);
    }
}
