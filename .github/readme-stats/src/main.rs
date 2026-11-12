mod github;
mod stats;

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use clap::Parser;
use github::{Api, GitHub};
use std::{fs, path::PathBuf};

const START: &str = "<!--START_SECTION:github-stats-->";
const END: &str = "<!--END_SECTION:github-stats-->";
const LEGACY_START: &str = "<!--START_SECTION:waka-->";
const LEGACY_END: &str = "<!--END_SECTION:waka-->";

#[derive(Parser)]
#[command(version, about = "Generate profile README stats from GitHub activity")]
struct Args {
    #[arg(long)]
    user: String,
    #[arg(long, default_value = "Europe/Berlin")]
    timezone: Tz,
    #[arg(long, default_value = "README.md")]
    readme: PathBuf,
    #[arg(long)]
    dry_run: bool,
}

fn replace_section(readme: &str, stats: &str) -> Result<String> {
    let (start, end) = if readme.contains(START) || readme.contains(END) {
        (START, END)
    } else {
        (LEGACY_START, LEGACY_END)
    };
    ensure!(
        readme.matches(start).count() == 1 && readme.matches(end).count() == 1,
        "README must contain exactly one matching stats marker pair."
    );
    let (before, remainder) = readme.split_once(start).context("Missing start marker")?;
    let (_, after) = remainder
        .split_once(end)
        .context("README stats markers are out of order.")?;
    Ok(format!("{before}{START}\n{stats}{END}{after}"))
}

fn update_readme(args: &Args, now: DateTime<Utc>, api: &mut impl Api) -> Result<Option<String>> {
    let readme = fs::read_to_string(&args.readme).context("Unable to read README")?;
    replace_section(&readme, "")?; // Validate markers before making API calls.
    let data = stats::collect(api, &args.user, now)?;
    let generated = stats::render(&data, now, args.timezone);
    let updated = replace_section(&readme, &generated)?;
    if args.dry_run {
        Ok(Some(generated))
    } else {
        // Fetching, validation and rendering must all succeed before writing.
        let assets = stats::assets(&data, now, args.timezone);
        let directory = args
            .readme
            .parent()
            .context("README has no parent directory")?
            .join("assets/stats");
        fs::create_dir_all(&directory).context("Unable to create stats asset directory")?;
        for (name, contents) in assets {
            fs::write(directory.join(name), contents).context("Unable to write stats card")?;
        }
        fs::write(&args.readme, updated).context("Unable to write README")?;
        Ok(None)
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    let mut api = GitHub::new()?;
    if let Some(preview) = update_readme(&args, Utc::now(), &mut api)? {
        print!("{preview}");
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Unable to update README stats: {error:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn migrates_markers_preserves_other_content_and_is_idempotent() {
        let original = format!("Intro\n{LEGACY_START}\nOld stats\n{LEGACY_END}\nFooter\n");
        let updated = replace_section(&original, "Fresh stats\n").unwrap();
        assert_eq!(
            updated,
            format!("Intro\n{START}\nFresh stats\n{END}\nFooter\n")
        );
        assert_eq!(replace_section(&updated, "Fresh stats\n").unwrap(), updated);
    }

    #[test]
    fn rejects_missing_duplicate_and_reversed_markers() {
        for original in [
            "No markers".to_owned(),
            START.to_owned(),
            format!("{START}{START}{END}"),
            format!("{END}{START}"),
        ] {
            assert!(replace_section(&original, "New stats").is_err());
        }
    }

    #[test]
    fn api_failure_leaves_readme_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("README.md");
        let original = format!("Intro\n{START}\nExisting stats\n{END}\n");
        fs::write(&path, &original).unwrap();
        let args = Args {
            user: "gabref".into(),
            timezone: chrono_tz::Europe::Berlin,
            readme: path.clone(),
            dry_run: false,
        };
        let mut failing_api = |_: &str, _: Value| anyhow::bail!("GitHub unavailable");
        assert!(update_readme(&args, Utc::now(), &mut failing_api).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), original);
    }
}
