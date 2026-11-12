use super::{Stats, activity, number, pulse};
use chrono::{DateTime, Datelike, Duration, Timelike, Utc};
use chrono_tz::Tz;

fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn frame(width: u32, height: u32, title: &str, body: &str) -> String {
    let title = xml(title);
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" role="img" aria-labelledby="title">
<title id="title">{title}</title>
<defs><linearGradient id="bg" x2="1" y2="1"><stop stop-color="#121C2D"/><stop offset="1" stop-color="#0B111D"/></linearGradient>
<linearGradient id="accent"><stop stop-color="#53E1C1"/><stop offset="1" stop-color="#8B9CFF"/></linearGradient></defs>
<style>text{{font-family:'Segoe UI',Arial,sans-serif;fill:#EDF3FF}}.muted{{fill:#9AABC4}}.accent{{fill:#53E1C1}}.eyebrow{{font-size:11px;letter-spacing:2px;font-weight:700}}.heading{{font-size:25px;font-weight:700}}.value{{font-size:48px;font-weight:700;letter-spacing:-1px}}</style>
<rect x=".5" y=".5" width="{}" height="{}" rx="18" fill="url(#bg)" stroke="#29364C"/>
{body}
</svg>
"##,
        width - 1,
        height - 1
    )
}

fn color(name: &str) -> &'static str {
    match name {
        "Rust" => "#DEA584",
        "TypeScript" => "#60A5FA",
        "JavaScript" => "#F2D96B",
        "Python" => "#73A4D1",
        "C" | "C++" => "#A3A6D3",
        "HTML" => "#F08267",
        "CSS" => "#B29AFF",
        "Go" => "#50C8D6",
        "Vue" => "#53D39C",
        "Shell" => "#B6D979",
        "Java" => "#ECAF79",
        "Lua" => "#939CF1",
        "C#" => "#9BD473",
        "Tauri" => "#FFC131",
        "Android" => "#A4C639",
        "Linux" => "#F5CE75",
        "Neovim" => "#73C99B",
        "React" => "#61DAFB",
        "Astro" => "#FF826B",
        "Terminal" => "#53E1C1",
        "JSON" | "TOML" | "YAML" => "#C998DE",
        _ => "#8496B4",
    }
}

fn ranked(languages: &std::collections::BTreeMap<String, u64>, limit: usize) -> Vec<(String, u64)> {
    let mut values = languages
        .iter()
        .map(|(name, count)| (name.clone(), *count))
        .collect::<Vec<_>>();
    values.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    if values.len() > limit {
        let other = values[limit..].iter().map(|(_, value)| value).sum();
        values.truncate(limit);
        values.push(("Other".into(), other));
    }
    values
}

fn overview(stats: &Stats, now: DateTime<Utc>, zone: Tz) -> String {
    let monthly = stats.commits.iter().collect::<Vec<_>>();
    let weekly = stats
        .commits
        .iter()
        .filter(|c| c.date >= now - Duration::days(7))
        .collect::<Vec<_>>();
    let month = activity(&monthly, zone);
    let week = activity(&weekly, zone);
    let mut body = format!(
        r##"<circle cx="795" cy="20" r="130" fill="#8B9CFF" opacity=".04"/>
<text x="28" y="35" class="accent eyebrow">GITHUB / LAST 30 DAYS</text>
<rect x="637" y="17" width="175" height="26" rx="13" fill="#53E1C1" opacity=".10"/>
<text x="724" y="35" text-anchor="middle" class="accent" font-size="11" font-weight="700">PUBLIC + PRIVATE</text>
<path d="M286 68V144M558 68V144M28 171H812" stroke="#29364C"/>
<text x="28" y="204" font-size="14" class="muted">LAST 7 DAYS</text>
<text x="167" y="204" font-size="16">{} commits · {} active days · {} repositories</text>
<text x="28" y="240" class="muted" font-size="12">{} public + {} private owned repositories in the language scan</text>
<text x="812" y="240" text-anchor="end" class="muted" font-size="12">{}</text>"##,
        number(week[0]),
        number(week[1]),
        number(week[2]),
        number(stats.repo_count - stats.private_repo_count),
        number(stats.private_repo_count),
        now.format("%d %b %Y")
    );
    for (x, value, label) in [
        (28, month[0], "Authored commits"),
        (307, month[1], "Active days"),
        (
            579,
            stats.contributions.total_pull_request_contributions,
            "Pull requests opened",
        ),
    ] {
        body.push_str(&format!(r##"<text x="{x}" y="115" class="value">{}</text><text x="{x}" y="143" class="muted" font-size="15">{label}</text>"##, number(value)));
    }
    frame(
        840,
        264,
        "GitHub activity: authored commits, active days and pull requests",
        &body,
    )
}

fn languages(stats: &Stats) -> String {
    let values = ranked(&stats.languages, 6);
    let total: u64 = values.iter().map(|(_, value)| value).sum();
    let mut body = format!(
        r##"<text x="24" y="33" class="accent eyebrow">CODE / LANGUAGES</text>
<text x="24" y="69" class="heading">What I build with</text>
<text x="24" y="93" class="muted" font-size="13">{} owned repositories · public + private</text>
<defs><clipPath id="bar"><rect x="24" y="115" width="352" height="12" rx="6"/></clipPath></defs>"##,
        number(stats.repo_count)
    );
    let mut x = 24.0;
    for (index, (name, count)) in values.iter().enumerate() {
        let share = if total > 0 {
            *count as f64 / total as f64
        } else {
            0.0
        };
        let width = share * 352.0;
        let fill = color(name);
        let y = 169 + index * 32;
        let name = xml(name);
        body.push_str(&format!(r##"<rect x="{x:.2}" y="115" width="{width:.2}" height="12" fill="{fill}" clip-path="url(#bar)"/>
<circle cx="29" cy="{}" r="4" fill="{fill}"/><text x="42" y="{y}" font-size="16">{name}</text>
<text x="376" y="{y}" text-anchor="end" class="muted" font-size="15">{:.1}%</text>"##, y - 5, share * 100.0));
        x += width;
    }
    if values.is_empty() {
        body.push_str(
            r##"<text x="24" y="180" class="muted" font-size="16">No language data yet.</text>"##,
        );
    }
    body.push_str(r##"<path d="M24 382H376" stroke="#29364C"/><text x="24" y="405" class="muted" font-size="11">By source size · forks counted separately below</text>"##);
    frame(
        400,
        424,
        "Language distribution across public and private repositories",
        &body,
    )
}

fn rhythm(stats: &Stats, zone: Tz) -> String {
    let mut days = [0_u64; 7];
    let mut hours = [0_u64; 4];
    for commit in &stats.commits {
        let date = commit.date.with_timezone(&zone);
        days[date.weekday().num_days_from_monday() as usize] += 1;
        hours[(date.hour() / 6) as usize] += 1;
    }
    let maximum = days.iter().copied().max().unwrap_or(0).max(1);
    let mut body = format!(
        r##"<text x="24" y="33" class="accent eyebrow">FLOW / COMMIT RHYTHM</text>
<text x="24" y="69" class="heading">When I ship</text>
<text x="24" y="93" class="muted" font-size="13">Past 30 days · {}</text>
<path d="M24 222H376M24 175H376M24 128H376" stroke="#29364C" stroke-dasharray="3 5"/>"##,
        xml(&zone.to_string())
    );
    for (index, (name, count)) in ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
        .into_iter()
        .zip(days)
        .enumerate()
    {
        let x = 31 + index * 50;
        let height = count as f64 / maximum as f64 * 90.0;
        let y = 222.0 - height;
        let fill = if count == maximum {
            "#53E1C1"
        } else {
            "#818CF8"
        };
        body.push_str(&format!(
            r##"<rect x="{x}" y="{y:.1}" width="28" height="{height:.1}" rx="5" fill="{fill}"/>
<text x="{}" y="{:.1}" text-anchor="middle" class="muted" font-size="12">{count}</text>
<text x="{}" y="247" text-anchor="middle" class="muted" font-size="12">{name}</text>"##,
            x + 14,
            y - 10.0,
            x + 14
        ));
    }
    body.push_str(r##"<path d="M24 273H376" stroke="#29364C"/><text x="24" y="298" class="accent eyebrow">TIME OF DAY</text>"##);
    for (index, (name, count)) in ["Night", "Morning", "Afternoon", "Evening"]
        .into_iter()
        .zip(hours)
        .enumerate()
    {
        let x = 24 + (index % 2) * 190;
        let y = 329 + (index / 2) * 32;
        body.push_str(&format!(r##"<text x="{x}" y="{y}" class="muted" font-size="14">{name}</text><text x="{}" y="{y}" text-anchor="end" font-size="14">{}</text>"##, x + 160, number(count)));
    }
    body.push_str(r##"<path d="M24 382H376" stroke="#29364C"/><text x="24" y="405" class="muted" font-size="11">Local commit times · daylight saving time included</text>"##);
    frame(
        400,
        424,
        "Commit activity by weekday and local time of day",
        &body,
    )
}

fn forks(stats: &Stats) -> String {
    let commits = stats
        .commits
        .iter()
        .filter(|c| c.from_fork)
        .collect::<Vec<_>>();
    let additions: u64 = commits.iter().map(|c| c.additions).sum();
    let deletions: u64 = commits.iter().map(|c| c.deletions).sum();
    let mut body = r##"<text x="28" y="35" class="accent eyebrow">CONTRIBUTIONS / FORKS &amp; PULL REQUESTS</text><text x="28" y="72" class="heading">Beyond my own repositories</text><path d="M28 173H812" stroke="#29364C"/>"##.to_owned();
    for (x, value, label, fill) in [
        (28, commits.len() as u64, "Authored fork commits", "#EDF3FF"),
        (246, additions, "Lines added", "#53E1C1"),
        (447, deletions, "Lines removed", "#FDA4AF"),
        (640, stats.fork_prs, "Fork PRs opened", "#EDF3FF"),
    ] {
        body.push_str(&format!(r##"<text x="{x}" y="124" font-size="36" font-weight="700" style="fill:{fill}">{}</text><text x="{x}" y="151" class="muted" font-size="13">{label}</text>"##, number(value)));
    }
    let values = ranked(&stats.fork_languages, 4);
    let total: u64 = values.iter().map(|(_, size)| size).sum();
    let top = values.iter().take(4).enumerate().map(|(index, (name, size))| {
        let x = 28 + index * 198;
        let percent = if total > 0 { *size as f64 / total as f64 * 100.0 } else { 0.0 };
        format!(r##"<circle cx="{x}" cy="200" r="4" fill="{}"/><text x="{}" y="205" font-size="14">{} <tspan class="muted">{percent:.0}%</tspan></text>"##, color(name), x + 12, xml(name))
    }).collect::<String>();
    body.push_str(&top);
    body.push_str(r##"<text x="28" y="241" class="muted" font-size="11">Past 30 days · changed lines by file type · commits counted once across branches and PRs</text>"##);
    frame(
        840,
        264,
        "Authored fork commits, changed lines and pull requests",
        &body,
    )
}

fn frameworks(stats: &Stats) -> String {
    let mut body = r##"<text x="24" y="33" class="accent eyebrow">STACK / REPOSITORY SIGNALS</text><text x="24" y="69" class="heading">Frameworks &amp; tooling</text><text x="24" y="93" class="muted" font-size="13">Matched repos · public + private</text>"##.to_owned();
    for (index, name) in super::frameworks::TOOLS.into_iter().enumerate() {
        let x = 24 + (index % 2) * 190;
        let y = 143 + (index / 2) * 62;
        let value = number(*stats.frameworks.get(name).unwrap_or(&0));
        body.push_str(&format!(r##"<text x="{x}" y="{y}" font-size="30" font-weight="700" style="fill:{}">{value}</text><text x="{x}" y="{}" class="muted" font-size="14">{name}</text>"##, color(name), y + 22));
    }
    body.push_str(r##"<path d="M24 382H376" stroke="#29364C"/><text x="24" y="405" class="muted" font-size="11">Detected from manifests, config paths &amp; topics</text>"##);
    frame(
        400,
        424,
        "Framework and tooling signals in owned repositories",
        &body,
    )
}

fn shipping(stats: &Stats, now: DateTime<Utc>, zone: Tz) -> String {
    let (days, current, longest) = pulse(stats, now, zone);
    let mut body = r##"<text x="24" y="33" class="accent eyebrow">MOMENTUM / LAST 30 DAYS</text><text x="24" y="69" class="heading">Shipping &amp; consistency</text><text x="24" y="93" class="muted" font-size="13">Outcomes of PRs opened in this period</text>"##.to_owned();
    let mut x = 24.0;
    let total: u64 = stats.pr_states.iter().sum();
    for (index, (name, fill)) in [
        ("Open", "#8B9CFF"),
        ("Merged", "#53E1C1"),
        ("Closed", "#FDA4AF"),
    ]
    .into_iter()
    .enumerate()
    {
        let width = if total > 0 {
            stats.pr_states[index] as f64 / total as f64 * 352.0
        } else {
            0.0
        };
        let y = 151 + index * 30;
        body.push_str(&format!(r##"<rect x="{x:.2}" y="112" width="{width:.2}" height="10" fill="{fill}"/><circle cx="28" cy="{}" r="4" fill="{fill}"/><text x="42" y="{y}" font-size="15">{name}</text><text x="376" y="{y}" text-anchor="end" class="muted" font-size="16">{}</text>"##, y - 5, number(stats.pr_states[index])));
        x += width;
    }
    body.push_str(&format!(r##"<path d="M24 236H376" stroke="#29364C"/><text x="24" y="260" class="accent eyebrow">COMMIT CONSISTENCY</text><text x="24" y="301" font-size="30" font-weight="700">{current} days</text><text x="214" y="301" font-size="30" font-weight="700">{longest} days</text><text x="24" y="324" class="muted" font-size="12">Current streak</text><text x="214" y="324" class="muted" font-size="12">Longest in window</text>"##));
    let maximum = days.iter().copied().max().unwrap_or(0).max(1);
    for (index, count) in days.into_iter().enumerate() {
        let x = 24 + (index % 15) * 24;
        let y = 343 + (index / 15) * 20;
        let opacity = if count > 0 {
            0.3 + count as f64 / maximum as f64 * 0.7
        } else {
            0.08
        };
        body.push_str(&format!(r##"<rect x="{x}" y="{y}" width="18" height="15" rx="3" fill="#53E1C1" opacity="{opacity:.2}"><title>{count} commits</title></rect>"##));
    }
    body.push_str(r##"<text x="24" y="405" class="muted" font-size="11">Streaks and pulse cover the last 30 local dates</text>"##);
    frame(
        400,
        424,
        "Pull request outcomes and commit consistency over 30 days",
        &body,
    )
}

fn pair(left: &str, right: &str, stacked: bool, title: &str) -> String {
    let namespace = |svg: &str, prefix: &str| {
        svg.replace("id=\"", &format!("id=\"{prefix}-"))
            .replace("url(#", &format!("url(#{prefix}-"))
            .replace(
                "aria-labelledby=\"",
                &format!("aria-labelledby=\"{prefix}-"),
            )
    };
    let (width, height, right_x, right_y, scale) = if stacked {
        (400, 868, 0, 444, 1.0)
    } else {
        (840, 434, 432, 0, 1.02)
    };
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" role="img" aria-label="{}"><g transform="scale({scale})">{}</g><g transform="translate({right_x} {right_y}) scale({scale})">{}</g></svg>"##,
        xml(title),
        namespace(left, "left"),
        namespace(right, "right")
    )
}

fn overview_compact(stats: &Stats, now: DateTime<Utc>, zone: Tz) -> String {
    let month = activity(&stats.commits.iter().collect::<Vec<_>>(), zone);
    let week = stats
        .commits
        .iter()
        .filter(|c| c.date >= now - Duration::days(7))
        .count() as u64;
    let mut body = r##"<text x="24" y="33" class="accent eyebrow">GITHUB / ACTIVITY</text><text x="24" y="65" class="muted" font-size="13">Public + private · past 30 days</text>"##.to_owned();
    for (index, (value, label)) in [
        (month[0], "Authored commits"),
        (month[1], "Active days"),
        (
            stats.contributions.total_pull_request_contributions,
            "PRs opened",
        ),
        (week, "Commits / last 7 days"),
    ]
    .into_iter()
    .enumerate()
    {
        let x = 24 + (index % 2) * 190;
        let y = 121 + (index / 2) * 84;
        body.push_str(&format!(r##"<text x="{x}" y="{y}" font-size="36" font-weight="700">{}</text><text x="{x}" y="{}" class="muted" font-size="13">{label}</text>"##,number(value),y+24));
    }
    body.push_str(&format!(r##"<path d="M24 255H376" stroke="#29364C"/><text x="24" y="281" class="muted" font-size="12">{} public + {} private owned repos</text><text x="24" y="309" class="muted" font-size="11">Updated {}</text>"##,number(stats.repo_count-stats.private_repo_count),number(stats.private_repo_count),now.format("%d %b %Y")));
    frame(400, 332, "GitHub activity overview, compact layout", &body)
}

fn forks_compact(stats: &Stats) -> String {
    let commits = stats
        .commits
        .iter()
        .filter(|c| c.from_fork)
        .collect::<Vec<_>>();
    let mut body = r##"<text x="24" y="33" class="accent eyebrow">CONTRIBUTIONS / LAST 30 DAYS</text><text x="24" y="69" class="heading">Forks &amp; pull requests</text>"##.to_owned();
    for (index, (value, label, fill)) in [
        (commits.len() as u64, "Authored fork commits", "#EDF3FF"),
        (stats.fork_prs, "Fork PRs opened", "#EDF3FF"),
        (
            commits.iter().map(|c| c.additions).sum(),
            "Lines added",
            "#53E1C1",
        ),
        (
            commits.iter().map(|c| c.deletions).sum(),
            "Lines removed",
            "#FDA4AF",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let x = 24 + (index % 2) * 190;
        let y = 125 + (index / 2) * 84;
        body.push_str(&format!(r##"<text x="{x}" y="{y}" font-size="34" font-weight="700" style="fill:{fill}">{}</text><text x="{x}" y="{}" class="muted" font-size="12">{label}</text>"##,number(value),y+24));
    }
    body.push_str(r##"<path d="M24 256H376" stroke="#29364C"/>"##);
    let values = ranked(&stats.fork_languages, 4);
    let total: u64 = values.iter().map(|(_, n)| n).sum();
    for (index, (name, count)) in values.iter().take(4).enumerate() {
        let x = 24 + (index % 2) * 190;
        let y = 286 + (index / 2) * 28;
        let percent = if total > 0 {
            *count as f64 / total as f64 * 100.0
        } else {
            0.0
        };
        body.push_str(&format!(r##"<circle cx="{x}" cy="{}" r="4" fill="{}"/><text x="{}" y="{y}" font-size="13">{} <tspan class="muted">{percent:.0}%</tspan></text>"##,y-5,color(name),x+12,xml(name)));
    }
    body.push_str(r##"<text x="24" y="353" class="muted" font-size="11">Changed lines by file type · unique commit hashes</text>"##);
    frame(
        400,
        376,
        "Fork contributions and changed lines, compact layout",
        &body,
    )
}

pub(super) fn render(stats: &Stats, now: DateTime<Utc>, zone: Tz) -> Vec<(&'static str, String)> {
    let languages = languages(stats);
    let rhythm = rhythm(stats, zone);
    let frameworks = frameworks(stats);
    let shipping = shipping(stats, now, zone);
    vec![
        ("activity.svg", overview(stats, now, zone)),
        ("languages.svg", languages.clone()),
        ("rhythm.svg", rhythm.clone()),
        ("forks.svg", forks(stats)),
        ("activity-compact.svg", overview_compact(stats, now, zone)),
        ("forks-compact.svg", forks_compact(stats)),
        ("frameworks.svg", frameworks.clone()),
        ("shipping.svg", shipping.clone()),
        (
            "languages-rhythm.svg",
            pair(&languages, &rhythm, false, "Languages and commit rhythm"),
        ),
        (
            "languages-rhythm-stacked.svg",
            pair(&languages, &rhythm, true, "Languages and commit rhythm"),
        ),
        (
            "stack-shipping.svg",
            pair(
                &frameworks,
                &shipping,
                false,
                "Frameworks and shipping consistency",
            ),
        ),
        (
            "stack-shipping-stacked.svg",
            pair(
                &frameworks,
                &shipping,
                true,
                "Frameworks and shipping consistency",
            ),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_escape_labels_and_never_expose_repository_identifiers() {
        let mut stats = Stats::default();
        stats.languages.insert("<script>&\"'".into(), 10);
        let cards = render(&stats, Utc::now(), chrono_tz::Europe::Berlin);
        assert_eq!(cards.len(), 12);
        for (_, card) in &cards {
            assert!(!card.contains("<script>"));
        }
        assert!(cards[1].1.contains("&lt;script&gt;&amp;&quot;&apos;"));
    }
}
