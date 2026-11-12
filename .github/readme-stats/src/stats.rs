use crate::github::Api;
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Datelike, Duration, Timelike, Utc};
use chrono_tz::Tz;
use serde::Deserialize;
use serde_json::json;
use std::collections::{BTreeMap, HashSet};

mod cards;
mod contributions;

const PROFILE: &str = include_str!("profile.graphql");
const COMMITS: &str = include_str!("commits.graphql");
const LANGUAGES: &str = include_str!("languages.graphql");
const PRIVATE_REPOSITORIES: &str = include_str!("private-repositories.graphql");

#[derive(Default)]
pub struct Stats {
    commits: Vec<Commit>,
    languages: BTreeMap<String, u64>,
    repo_count: u64,
    private_repo_count: u64,
    fork_languages: BTreeMap<String, u64>,
    fork_prs: u64,
    contributions: Contributions,
}

struct Commit {
    oid: String,
    date: DateTime<Utc>,
    repo: String,
    additions: u64,
    deletions: u64,
    from_fork: bool,
    from_pr: bool,
}

#[derive(Deserialize)]
struct Profile {
    user: Option<User>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct User {
    id: String,
    contributions_collection: Contributions,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Contributions {
    total_repositories_with_contributed_commits: usize,
    total_pull_request_contributions: u64,
    total_pull_request_review_contributions: u64,
    total_issue_contributions: u64,
    #[serde(default)]
    restricted_contributions_count: u64,
    commit_contributions_by_repository: Vec<RepositoryContribution>,
}

#[derive(Deserialize)]
struct RepositoryContribution {
    repository: Repository,
}

#[derive(Deserialize)]
struct Repository {
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

impl PageInfo {
    fn next(self, previous: &Option<String>) -> Result<Option<String>> {
        if !self.has_next_page {
            return Ok(None);
        }
        ensure!(
            self.end_cursor.is_some() && &self.end_cursor != previous,
            "GitHub pagination did not advance."
        );
        Ok(self.end_cursor)
    }
}

#[derive(Deserialize)]
struct HistoryResponse {
    node: Option<HistoryRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryRepository {
    default_branch_ref: Option<Branch>,
}

#[derive(Deserialize)]
struct Branch {
    target: HistoryTarget,
}

#[derive(Deserialize)]
struct HistoryTarget {
    history: CommitPage,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommitPage {
    nodes: Vec<RawCommit>,
    page_info: PageInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCommit {
    oid: String,
    committed_date: DateTime<Utc>,
    additions: u64,
    deletions: u64,
    parents: Count,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Count {
    total_count: usize,
}

#[derive(Deserialize)]
struct LanguagesResponse {
    user: LanguageUser,
}

#[derive(Deserialize)]
struct LanguageUser {
    repositories: RepositoryPage,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryPage {
    nodes: Vec<LanguageRepository>,
    page_info: PageInfo,
}

#[derive(Deserialize)]
struct LanguageRepository {
    #[serde(rename = "isPrivate")]
    is_private: bool,
    languages: LanguagePage,
}

#[derive(Deserialize)]
struct PrivateResponse {
    user: PrivateUser,
}

#[derive(Deserialize)]
struct PrivateUser {
    repositories: PrivatePage,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrivatePage {
    nodes: Vec<PrivateRepository>,
    page_info: PageInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrivateRepository {
    id: String,
    pushed_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LanguagePage {
    total_count: usize,
    edges: Vec<LanguageEdge>,
}

#[derive(Deserialize)]
struct LanguageEdge {
    size: u64,
    node: Language,
}

#[derive(Deserialize)]
struct Language {
    name: String,
}

pub fn collect(api: &mut impl Api, login: &str, now: DateTime<Utc>) -> Result<Stats> {
    let since = now - Duration::days(30);
    let profile: Profile = serde_json::from_value(api.query(
        PROFILE,
        json!({"login": login, "since": since, "until": now}),
    )?)
    .context("Invalid GitHub profile response")?;
    let user = profile.user.context("GitHub user was not found.")?;
    let contributions = user.contributions_collection;
    ensure!(
        contributions.total_repositories_with_contributed_commits
            <= contributions.commit_contributions_by_repository.len(),
        "GitHub's 100-repository contribution limit was exceeded."
    );

    let mut repositories: Vec<String> = contributions
        .commit_contributions_by_repository
        .iter()
        .map(|item| item.repository.id.clone())
        .collect();
    // The contribution feed can mask private repositories even for the owner.
    // Discover accessible private repositories explicitly, without fetching names.
    let mut cursor = None;
    loop {
        let response: PrivateResponse = serde_json::from_value(api.query(
            PRIVATE_REPOSITORIES,
            json!({"login": login, "cursor": cursor}),
        )?)
        .context("Invalid private repository response")?;
        let page = response.user.repositories;
        for repo in page.nodes {
            if repo.pushed_at.is_some_and(|date| date >= since) && !repositories.contains(&repo.id)
            {
                repositories.push(repo.id);
            }
        }
        cursor = page.page_info.next(&cursor)?;
        if cursor.is_none() {
            break;
        }
    }

    eprintln!(
        "Reading authored default-branch commits from {} accessible repositories",
        repositories.len()
    );
    let mut result = Stats::default();
    let mut seen = HashSet::new();
    for repo in &repositories {
        let mut cursor = None;
        loop {
            let response: HistoryResponse = serde_json::from_value(api.query(
                COMMITS,
                json!({"repo": repo, "author": user.id, "since": since,
                                "until": now, "cursor": cursor}),
            )?)
            .context("Invalid GitHub commit history response")?;
            let repository = response
                .node
                .context("A contribution repository could not be read.")?;
            let Some(branch) = repository.default_branch_ref else {
                break; // Empty repositories have no default branch.
            };
            let history = branch.target.history;
            for commit in history.nodes {
                // Avoid counting merge diffs and copies of the same commit twice.
                if commit.committed_date >= since
                    && commit.committed_date <= now
                    && commit.parents.total_count <= 1
                    && seen.insert(commit.oid.clone())
                {
                    result.commits.push(Commit {
                        oid: commit.oid,
                        date: commit.committed_date,
                        repo: repo.clone(),
                        additions: commit.additions,
                        deletions: commit.deletions,
                        from_fork: false,
                        from_pr: false,
                    });
                }
            }
            cursor = history.page_info.next(&cursor)?;
            if cursor.is_none() {
                break;
            }
        }
    }
    result.contributions = contributions;

    let mut cursor = None;
    loop {
        let response: LanguagesResponse = serde_json::from_value(
            api.query(LANGUAGES, json!({"login": login, "cursor": cursor}))?,
        )
        .context("Invalid GitHub language response")?;
        let repositories = response.user.repositories;
        for repo in repositories.nodes {
            ensure!(
                repo.languages.total_count <= repo.languages.edges.len(),
                "A repository exceeded GitHub's 100-language limit."
            );
            result.repo_count += 1;
            result.private_repo_count += u64::from(repo.is_private);
            for language in repo.languages.edges {
                *result.languages.entry(language.node.name).or_default() += language.size;
            }
        }
        cursor = repositories.page_info.next(&cursor)?;
        if cursor.is_none() {
            break;
        }
    }
    eprintln!(
        "Language coverage: {} public + {} private owned repositories",
        result.repo_count - result.private_repo_count,
        result.private_repo_count
    );
    contributions::collect(api, login, &user.id, now, &mut result)?;
    Ok(result)
}

fn number(value: u64) -> String {
    let digits = value.to_string();
    let mut formatted = String::new();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            formatted.push(',');
        }
        formatted.push(ch);
    }
    formatted
}

fn distribution(counts: &[(String, u64)]) -> String {
    let sanitized: Vec<_> = counts
        .iter()
        .map(|(label, count)| (label.replace('`', "").replace(['\n', '\r'], " "), *count))
        .collect();
    let label_width = sanitized
        .iter()
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or(0)
        .max(22);
    let value_width = counts
        .iter()
        .map(|(_, count)| number(*count).len())
        .max()
        .unwrap_or(0)
        .max(8);
    let total: u64 = counts.iter().map(|(_, count)| count).sum();
    let mut lines = vec!["```text".to_owned()];
    for (label, count) in sanitized {
        let percent = if total > 0 {
            count as f64 / total as f64 * 100.0
        } else {
            0.0
        };
        let filled = (percent / 100.0 * 20.0).round() as usize;
        let bar = format!("{}{}", "█".repeat(filled), "░".repeat(20 - filled));
        let count = number(count);
        lines.push(format!(
            "{label:<label_width$} {count:>value_width$}  {bar} {percent:>6.2}%"
        ));
    }
    lines.push("```".into());
    lines.join("\n")
}

fn activity(commits: &[&Commit], zone: Tz) -> [u64; 5] {
    [
        commits.len() as u64,
        commits
            .iter()
            .map(|commit| commit.date.with_timezone(&zone).date_naive())
            .collect::<HashSet<_>>()
            .len() as u64,
        commits
            .iter()
            .map(|commit| &commit.repo)
            .collect::<HashSet<_>>()
            .len() as u64,
        commits.iter().map(|commit| commit.additions).sum(),
        commits.iter().map(|commit| commit.deletions).sum(),
    ]
}

pub fn render(stats: &Stats, now: DateTime<Utc>, zone: Tz) -> String {
    let weekly = stats
        .commits
        .iter()
        .filter(|commit| commit.date >= now - Duration::days(7))
        .collect::<Vec<_>>();
    let monthly = stats.commits.iter().collect::<Vec<_>>();
    let week = activity(&weekly, zone);
    let month = activity(&monthly, zone);
    let mut summary = vec![
        "| Activity | Past 7 days | Past 30 days |".to_owned(),
        "| :--- | ---: | ---: |".to_owned(),
    ];
    for (index, name) in [
        "Commits",
        "Active days",
        "Repositories",
        "Lines added",
        "Lines removed",
    ]
    .iter()
    .enumerate()
    {
        summary.push(format!(
            "| {name} | {} | {} |",
            number(week[index]),
            number(month[index])
        ));
    }

    let mut days = [0; 7];
    let mut hours = [0; 4];
    for commit in &stats.commits {
        let date = commit.date.with_timezone(&zone);
        days[date.weekday().num_days_from_monday() as usize] += 1;
        hours[(date.hour() / 6) as usize] += 1;
    }
    let day_counts = [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ]
    .into_iter()
    .zip(days)
    .map(|(name, count)| (name.to_owned(), count))
    .collect::<Vec<_>>();
    let hour_counts = [
        "Night (00:00–05:59)",
        "Morning (06:00–11:59)",
        "Afternoon (12:00–17:59)",
        "Evening (18:00–23:59)",
    ]
    .into_iter()
    .zip(hours)
    .map(|(name, count)| (name.to_owned(), count))
    .collect::<Vec<_>>();
    let contributions = &stats.contributions;
    let mut sections = vec![
        "## 📊 GitHub activity".to_owned(),
        "<p align=\"center\">\n  <img src=\"assets/stats/activity.svg\" width=\"820\" alt=\"GitHub activity overview for the past 30 days\">\n</p>\n<p align=\"center\">\n  <img src=\"assets/stats/languages.svg\" width=\"400\" alt=\"Languages across public and private repositories\">\n  <img src=\"assets/stats/rhythm.svg\" width=\"400\" alt=\"Commit activity by weekday and time of day\">\n</p>".to_owned(),
        "<p align=\"center\">\n  <img src=\"assets/stats/forks.svg\" width=\"820\" alt=\"My authored commits, changed lines and pull requests involving forks\">\n</p>".to_owned(),
        "<details>\n<summary>Explore the numbers</summary>".to_owned(),
        summary.join("\n"),
        format!(
            "**Also in the past 30 days:** Pull requests opened: {} · Reviews submitted: {} · Issues opened: {}",
            number(contributions.total_pull_request_contributions),
            number(contributions.total_pull_request_review_contributions),
            number(contributions.total_issue_contributions)
        ),
        format!("**Commit activity by day** · past 30 days · {zone}"),
        distribution(&day_counts),
        "**Commit activity by time of day** · past 30 days".to_owned(),
        distribution(&hour_counts),
        format!(
            "**Languages across {} repositories** · public + private · source size in bytes",
            number(stats.repo_count)
        ),
    ];
    let mut languages: Vec<_> = stats
        .languages
        .iter()
        .map(|(name, size)| (name.clone(), *size))
        .collect();
    languages.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    if languages.is_empty() {
        sections.push("No language data available.".into());
    } else {
        if languages.len() > 8 {
            let other = languages[8..].iter().map(|(_, size)| size).sum();
            languages.truncate(8);
            languages.push(("Other".into(), other));
        }
        sections.push(distribution(&languages));
    }
    sections.push(format!("**Repository coverage:** {} public + {} private owned, non-fork repositories.\n\n**Private contributions reported by GitHub:** {} (includes contribution types beyond commits; not added to the commit total).",
        number(stats.repo_count - stats.private_repo_count), number(stats.private_repo_count), number(contributions.restricted_contributions_count)));
    let fork_commits = stats
        .commits
        .iter()
        .filter(|commit| commit.from_fork)
        .collect::<Vec<_>>();
    sections.push(format!("**Forks & pull requests · past 30 days**\n\n| Authored fork commits | Lines added | Lines removed | PRs opened involving forks |\n| ---: | ---: | ---: | ---: |\n| {} | {} | {} | {} |",
        number(fork_commits.len() as u64), number(fork_commits.iter().map(|c| c.additions).sum()), number(fork_commits.iter().map(|c| c.deletions).sum()), number(stats.fork_prs)));
    if !stats.fork_languages.is_empty() {
        sections.push("**Languages in fork contributions** · changed lines by file type".into());
        let mut fork_languages = stats
            .fork_languages
            .iter()
            .map(|(name, count)| (name.clone(), *count))
            .collect::<Vec<_>>();
        fork_languages.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        sections.push(distribution(&fork_languages));
    }
    sections.push("</details>".into());
    sections.push(
        concat!(
            "<details>\n<summary>About these stats</summary>\n\n",
            "Activity covers my authored commits on default branches, owned fork branches, ",
            "and pull requests I opened, in repositories accessible to the automation. ",
            "Merge commits and duplicate commit hashes are excluded. ",
            "Private activity is aggregated without repository names. ",
            "Line counts describe commit diffs, including generated files. ",
            "Private repository discovery includes accessible collaborations. ",
            "Languages cover my public and private, owned, non-fork repositories.\n\n</details>"
        )
        .into(),
    );
    sections.push(format!(
        "Updated on {} UTC · Generated by [my own automation](.github/readme-stats)",
        now.format("%Y-%m-%d")
    ));
    sections.join("\n\n") + "\n"
}

pub fn assets(stats: &Stats, now: DateTime<Utc>, zone: Tz) -> Vec<(&'static str, String)> {
    cards::render(stats, now, zone)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn now() -> DateTime<Utc> {
        "2026-11-12T12:00:00Z".parse().unwrap()
    }

    fn contribution_json(repos: &[&str]) -> Value {
        json!({
            "totalRepositoriesWithContributedCommits": repos.len(),
            "totalPullRequestContributions": 4, "totalPullRequestReviewContributions": 2,
            "totalIssueContributions": 1,
            "commitContributionsByRepository": repos.iter().map(|id| json!({"repository": {"id": id}})).collect::<Vec<_>>()
        })
    }

    fn raw_commit(oid: &str, days: i64, parents: u64) -> Value {
        json!({"oid": oid, "committedDate": now() - Duration::days(days),
               "additions": 10, "deletions": 3, "parents": {"totalCount": parents}})
    }

    fn page_info(next: Option<&str>) -> Value {
        json!({"hasNextPage": next.is_some(), "endCursor": next})
    }

    #[test]
    fn paginates_commits_and_languages_excludes_merges_and_deduplicates() {
        let mut api = |query: &str, variables: Value| -> Result<Value> {
            if query == PROFILE {
                return Ok(
                    json!({"user": {"id": "user-id", "contributionsCollection": contribution_json(&["a", "b"])}}),
                );
            }
            if query == COMMITS {
                assert_eq!(variables["since"], json!(now() - Duration::days(30)));
                assert_eq!(variables["until"], json!(now()));
                let (nodes, next) = if variables["repo"] == "private-only" {
                    (vec![raw_commit("private-commit", 2, 1)], None)
                } else if variables["repo"] == "b" {
                    (
                        vec![raw_commit("shared", 1, 1), raw_commit("second-repo", 1, 1)],
                        None,
                    )
                } else if variables["cursor"].is_null() {
                    (
                        vec![raw_commit("shared", 1, 1), raw_commit("merge", 1, 2)],
                        Some("next-commit"),
                    )
                } else {
                    assert_eq!(variables["cursor"], "next-commit");
                    (
                        vec![
                            raw_commit("older", 20, 1),
                            raw_commit("boundary", 30, 1),
                            raw_commit("too-old", 31, 1),
                            raw_commit("future", -1, 1),
                        ],
                        None,
                    )
                };
                return Ok(json!({"node": {"defaultBranchRef": {"target": {"history": {
                    "nodes": nodes, "pageInfo": page_info(next)
                }}}}}));
            }
            if query == PRIVATE_REPOSITORIES {
                return Ok(json!({"user":{"repositories":{"nodes":[
                    {"id":"private-only","pushedAt":now()},
                    {"id":"inactive-private","pushedAt":now()-Duration::days(31)}
                ],"pageInfo":page_info(None)}}}));
            }
            if query == contributions::FORKS {
                return Ok(
                    json!({"user":{"repositories":{"nodes":[],"pageInfo":page_info(None)}}}),
                );
            }
            if query == contributions::PULL_REQUESTS {
                return Ok(
                    json!({"user":{"pullRequests":{"nodes":[],"pageInfo":page_info(None)}}}),
                );
            }
            assert_eq!(query, LANGUAGES);
            let (name, size, next) = if variables["cursor"].is_null() {
                ("Python", 100, Some("next-repo"))
            } else {
                assert_eq!(variables["cursor"], "next-repo");
                ("Rust", 300, None)
            };
            Ok(json!({"user": {"repositories": {
                "nodes": [{"isPrivate": name == "Rust", "languages": {"totalCount": 1, "edges": [{"size": size, "node": {"name": name}}]}}],
                "pageInfo": page_info(next)
            }}}))
        };
        let stats = collect(&mut api, "gabref", now()).unwrap();
        assert_eq!(stats.commits.len(), 5);
        assert_eq!(stats.commits.iter().map(|c| c.additions).sum::<u64>(), 50);
        assert_eq!(stats.repo_count, 2);
        assert_eq!(stats.private_repo_count, 1);
        assert_eq!(stats.languages["Python"], 100);
        assert_eq!(stats.languages["Rust"], 300);
    }

    #[test]
    fn rejects_truncated_repository_discovery() {
        let mut values = contribution_json(&["a"]);
        values["totalRepositoriesWithContributedCommits"] = json!(101);
        let mut api =
            |_: &str, _: Value| Ok(json!({"user": {"id": "u", "contributionsCollection": values}}));
        assert!(
            collect(&mut api, "gabref", now())
                .is_err_and(|error| error.to_string().contains("100-repository"))
        );
    }

    #[test]
    fn rejects_nonadvancing_pagination() {
        for cursor in [None, Some("same".into())] {
            let page = PageInfo {
                has_next_page: true,
                end_cursor: cursor,
            };
            assert!(page.next(&Some("same".into())).is_err());
        }
    }

    #[test]
    fn renders_windows_timezone_and_private_aggregates() {
        let stats = Stats {
            commits: vec![
                Commit {
                    oid: "a".into(),
                    date: "2026-11-08T23:30:00Z".parse().unwrap(),
                    repo: "private-repo-id".into(),
                    additions: 10,
                    deletions: 3,
                    from_fork: true,
                    from_pr: true,
                },
                Commit {
                    oid: "b".into(),
                    date: now() - Duration::days(15),
                    repo: "another-private-id".into(),
                    additions: 10,
                    deletions: 3,
                    from_fork: false,
                    from_pr: false,
                },
            ],
            languages: BTreeMap::from([("Rust".into(), 100)]),
            repo_count: 3,
            ..Stats::default()
        };
        let output = render(&stats, now(), chrono_tz::Europe::Berlin);
        assert!(output.contains("| Commits | 1 | 2 |"));
        assert!(output.contains("| Lines added | 10 | 20 |"));
        assert!(output.contains("| Lines removed | 3 | 6 |"));
        assert!(
            output
                .lines()
                .any(|line| line.starts_with("Monday")
                    && line.split_whitespace().nth(1) == Some("1"))
        );
        assert!(
            output
                .lines()
                .any(|line| line.starts_with("Night") && line.contains("1  "))
        );
        assert!(!output.contains("private-repo-id"));
        assert!(!output.contains("another-private-id"));
    }

    #[test]
    fn observes_berlin_daylight_saving_time() {
        let summer: DateTime<Utc> = "2026-07-01T04:30:00Z".parse().unwrap();
        let winter: DateTime<Utc> = "2026-11-01T04:30:00Z".parse().unwrap();
        assert_eq!(summer.with_timezone(&chrono_tz::Europe::Berlin).hour(), 6);
        assert_eq!(winter.with_timezone(&chrono_tz::Europe::Berlin).hour(), 5);
    }

    #[test]
    fn empty_activity_renders_without_invalid_percentages() {
        let output = render(&Stats::default(), now(), chrono_tz::Europe::Berlin);
        assert!(output.contains("| Commits | 0 | 0 |"));
        assert!(output.contains("No language data available."));
        assert!(!output.contains("NaN"));
    }

    #[test]
    fn keeps_other_languages_in_percentage_denominator() {
        let stats = Stats {
            languages: (0..10).map(|index| (format!("Lang{index}"), 10)).collect(),
            ..Stats::default()
        };
        let output = render(&stats, now(), chrono_tz::Europe::Berlin);
        assert!(
            output
                .lines()
                .any(|line| line.starts_with("Lang0") && line.ends_with("10.00%"))
        );
        assert!(
            output
                .lines()
                .any(|line| line.starts_with("Other") && line.ends_with("20.00%"))
        );
    }

    #[test]
    fn formats_large_numbers_and_sanitizes_labels() {
        assert_eq!(number(0), "0");
        assert_eq!(number(1_234_567), "1,234,567");
        let output = distribution(&[("Language```\nInjected".into(), 1_234_567)]);
        assert_eq!(output.matches("```").count(), 2);
        assert!(output.contains("Language Injected"));
    }
}
