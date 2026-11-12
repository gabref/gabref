use crate::github::Api;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

pub(super) const TOOLS: [&str; 8] = [
    "Tauri", "Android", "Linux", "Neovim", "React", "Vue", "Astro", "Terminal",
];

pub(super) struct Source {
    pub id: String,
    pub revision: Option<String>,
    pub topics: Vec<String>,
}

const FILES: [(&str, &str, bool); 15] = [
    ("package", "package.json", true),
    ("cargo", "Cargo.toml", true),
    ("tauriCargo", "src-tauri/Cargo.toml", true),
    ("tauriConfig", "src-tauri/tauri.conf.json", false),
    ("androidApp", "app/src/main/AndroidManifest.xml", false),
    ("androidLib", "android/src/main/AndroidManifest.xml", false),
    (
        "androidApplication",
        "android/app/src/main/AndroidManifest.xml",
        false,
    ),
    ("gradle", "build.gradle", true),
    ("gradleKts", "build.gradle.kts", true),
    ("androidGradle", "android/build.gradle", true),
    ("androidGradleKts", "android/build.gradle.kts", true),
    ("neovimConfig", ".config/nvim/init.lua", false),
    ("neovimNested", "nvim/init.lua", false),
    ("initLua", "init.lua", true),
    ("pyproject", "pyproject.toml", true),
];

fn dependency(manifest: &str, name: &str) -> bool {
    let mut dependencies = false;
    for line in manifest.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            dependencies = line.contains("dependencies");
            if dependencies && line.ends_with(&format!(".{name}]")) {
                return true;
            }
        } else if dependencies
            && let Some((key, _)) = line.split_once('=')
            && key.trim().trim_matches(['\'', '"']) == name
        {
            return true;
        }
    }
    false
}

fn signals(source: &Source, manifests: &Value) -> Result<HashSet<&'static str>> {
    let topics: HashSet<_> = source
        .topics
        .iter()
        .map(|topic| topic.to_lowercase())
        .collect();
    let topic = |names: &[&str]| names.iter().any(|name| topics.contains(*name));
    let text = |key: &str| manifests[key]["text"].as_str().unwrap_or("");
    let exists = |key: &str| {
        manifests[key]["byteSize"]
            .as_u64()
            .is_some_and(|size| size > 0)
    };
    let package: Value = if text("package").trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(text("package")).context("A package manifest is invalid JSON")?
    };
    let npm = |name: &str| {
        [
            "dependencies",
            "devDependencies",
            "peerDependencies",
            "optionalDependencies",
        ]
        .iter()
        .any(|section| package[*section].get(name).is_some())
    };
    let cargos = [text("cargo"), text("tauriCargo")];
    let target = |os: &str| {
        cargos.iter().any(|cargo| {
            cargo.lines().any(|line| {
                let line = line.trim();
                line.starts_with("[target.") && line.contains(os)
            })
        })
    };
    let mut found = HashSet::new();
    if topic(&["tauri", "tauri-app", "tauri-plugin"])
        || npm("@tauri-apps/api")
        || npm("@tauri-apps/cli")
        || exists("tauriConfig")
        || cargos.iter().any(|cargo| dependency(cargo, "tauri"))
    {
        found.insert("Tauri");
    }
    if topic(&["android", "android-app", "android-library"])
        || target("android")
        || ["androidApp", "androidLib", "androidApplication"]
            .iter()
            .any(|key| exists(key))
        || ["gradle", "gradleKts", "androidGradle", "androidGradleKts"]
            .iter()
            .any(|key| text(key).contains("com.android."))
    {
        found.insert("Android");
    }
    if topic(&[
        "linux",
        "linux-kernel",
        "nixos",
        "archlinux",
        "linux-desktop",
    ]) || target("linux")
    {
        found.insert("Linux");
    }
    if topic(&["neovim", "nvim", "neovim-plugin", "neovim-config"])
        || exists("neovimConfig")
        || exists("neovimNested")
        || text("initLua").contains("vim.")
        || text("initLua").contains("require(\"lazy\")")
    {
        found.insert("Neovim");
    }
    if topic(&["react", "reactjs", "nextjs"]) || npm("react") || npm("next") {
        found.insert("React");
    }
    if topic(&["vue", "vuejs", "nuxt"]) || npm("vue") || npm("nuxt") {
        found.insert("Vue");
    }
    if topic(&["astro", "astrojs"]) || npm("astro") {
        found.insert("Astro");
    }
    if topic(&[
        "cli",
        "tui",
        "terminal",
        "command-line",
        "command-line-tool",
    ]) || package.get("bin").is_some()
        || cargos.iter().any(|cargo| {
            ["clap", "ratatui", "crossterm", "termion", "argh"]
                .iter()
                .any(|name| dependency(cargo, name))
        })
        || text("pyproject").contains("[project.scripts]")
    {
        found.insert("Terminal");
    }
    Ok(found)
}

pub(super) fn collect(api: &mut impl Api, sources: &[Source]) -> Result<BTreeMap<String, u64>> {
    let mut matches = sources
        .iter()
        .map(|source| signals(source, &Value::Null))
        .collect::<Result<Vec<_>>>()?;
    let active = sources
        .iter()
        .enumerate()
        .filter(|(_, source)| source.revision.is_some())
        .collect::<Vec<_>>();
    eprintln!(
        "Scanning framework and tooling signals across {} owned repositories",
        sources.len()
    );
    for batch in active.chunks(8) {
        let mut query = String::from("query {");
        for (alias, (_, source)) in batch.iter().enumerate() {
            query.push_str(&format!(
                "r{alias}: node(id: {}) {{ ... on Repository {{",
                serde_json::to_string(&source.id)?
            ));
            for (name, path, content) in FILES {
                let expression =
                    format!("{}:{path}", source.revision.as_deref().unwrap_or_default());
                query.push_str(&format!(
                    "{name}: object(expression: {}) {{ ... on Blob {{ byteSize {} }} }}",
                    serde_json::to_string(&expression)?,
                    if content { "text" } else { "" }
                ));
            }
            query.push_str("} }");
        }
        query.push('}');
        let response = api.query(&query, json!({}))?;
        for (alias, (index, source)) in batch.iter().enumerate() {
            let manifests = &response[format!("r{alias}")];
            ensure!(
                !manifests.is_null(),
                "A repository could not be inspected for framework signals"
            );
            matches[*index].extend(signals(source, manifests)?);
        }
    }
    let mut counts: BTreeMap<_, _> = TOOLS.into_iter().map(|name| (name.to_owned(), 0)).collect();
    for found in matches {
        for name in found {
            *counts.entry(name.to_owned()).or_default() += 1;
        }
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_actual_dependencies_and_configs_without_treating_languages_as_frameworks() {
        let source = Source {
            id: "private-id".into(),
            revision: None,
            topics: vec!["linux".into()],
        };
        let found = signals(&source, &json!({
            "package":{"text":r#"{"dependencies":{"@tauri-apps/api":"2","react":"19","astro":"5"}}"#},
            "cargo":{"text":"[package]\nname = 'private-project'\n[dependencies]\nratatui = '0.29'\n# vue = '1'"},
            "androidApp":{"byteSize":80}, "neovimConfig":{"byteSize":40}
        })).unwrap();
        for name in [
            "Tauri", "Android", "Linux", "Neovim", "React", "Astro", "Terminal",
        ] {
            assert!(found.contains(name));
        }
        assert!(!found.contains("Vue"));
        assert!(!dependency(
            "[package]\nname = 'tauri'\n# tauri = '2'",
            "tauri"
        ));
        assert!(dependency(
            "[workspace.dependencies.tauri]\nversion='2'",
            "tauri"
        ));
    }

    #[test]
    fn counts_a_repository_once_and_keeps_its_identity_private() {
        let sources = vec![Source {
            id: "private-id".into(),
            revision: Some("abc123".into()),
            topics: vec!["tauri".into()],
        }];
        let mut api = |query: &str, _: Value| {
            assert!(query.contains("abc123:package.json"));
            Ok(
                json!({"r0":{"tauriConfig":{"byteSize":50},"cargo":{"text":"[dependencies]\ntauri='2'"}}}),
            )
        };
        let counts = collect(&mut api, &sources).unwrap();
        assert_eq!(counts["Tauri"], 1);
        assert_eq!(counts["Linux"], 0);
        assert!(!format!("{counts:?}").contains("private-id"));
    }
}
