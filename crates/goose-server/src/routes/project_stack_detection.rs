//! Read-only stack discovery from a project's connected local code folder.
//! Only known manifests are read; no commands, network requests or .env files.
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Read;
use std::path::Path;

const MAX_ENTRIES: usize = 4096;
const MAX_FILES: usize = 128;
const MAX_BYTES: u64 = 512 * 1024;
const MAX_DEPTH: usize = 5;

#[derive(Debug, Serialize)]
pub struct DetectedEntry {
    pub service_name: String,
    pub category: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Detection {
    pub entries: Vec<DetectedEntry>,
    pub scanned_files: usize,
    pub incomplete: bool,
}

/// A declaration proves a dependency/configuration, not a deployed service or
/// account identity. Never echo manifest values (URLs may contain credentials).
pub fn detect(root: &Path) -> Result<Detection, String> {
    let root = root
        .canonicalize()
        .map_err(|_| "The connected code folder could not be read.")?;
    if !root.is_dir() {
        return Err("The connected code folder is not a directory.".into());
    }
    let mut queue = VecDeque::from([(root.clone(), 0)]);
    let mut found: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    let mut scanned_files = 0;
    let mut visited = 0;
    let mut incomplete = false;
    while let Some((dir, depth)) = queue.pop_front() {
        let children = match dir.read_dir() {
            Ok(items) => items,
            Err(_) => {
                incomplete = true;
                continue;
            }
        };
        for child in children {
            visited += 1;
            if visited > MAX_ENTRIES || scanned_files >= MAX_FILES {
                incomplete = true;
                break;
            }
            let Ok(child) = child else {
                incomplete = true;
                continue;
            };
            let Ok(kind) = child.file_type() else {
                incomplete = true;
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            let name = child.file_name().to_string_lossy().into_owned();
            if kind.is_dir() {
                if name.starts_with('.')
                    || [
                        "node_modules",
                        "target",
                        "vendor",
                        "dist",
                        "build",
                        "coverage",
                        "__pycache__",
                        "venv",
                    ]
                    .contains(&name.as_str())
                {
                    continue;
                }
                if depth < MAX_DEPTH {
                    queue.push_back((child.path(), depth + 1));
                } else {
                    incomplete = true;
                }
                continue;
            }
            if !kind.is_file() || !is_manifest(&name) {
                continue;
            }
            let path = child.path();
            // Recheck the resolved boundary before opening, including parents.
            let Ok(canonical) = path.canonicalize() else {
                incomplete = true;
                continue;
            };
            if !canonical.starts_with(&root) {
                continue;
            }
            let text = match std::fs::File::open(&canonical).and_then(|file| {
                let mut bytes = Vec::new();
                file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
                Ok(bytes)
            }) {
                Ok(bytes) if bytes.len() <= MAX_BYTES as usize => String::from_utf8(bytes).ok(),
                _ => None,
            };
            let Some(text) = text else {
                incomplete = true;
                continue;
            };
            scanned_files += 1;
            let evidence = path
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let mut add = |service: &str, category: &str| {
                found
                    .entry((category.to_string(), service.to_string()))
                    .or_default()
                    .insert(evidence.clone());
            };
            if !inspect(&name, &text, &mut add) {
                incomplete = true;
            }
        }
        if visited > MAX_ENTRIES || scanned_files >= MAX_FILES {
            incomplete |= !queue.is_empty();
            break;
        }
    }
    Ok(Detection {
        entries: found
            .into_iter()
            .map(|((category, service_name), evidence)| DetectedEntry {
                service_name,
                category,
                evidence: evidence.into_iter().collect(),
            })
            .collect(),
        scanned_files,
        incomplete,
    })
}

fn is_manifest(name: &str) -> bool {
    matches!(
        name,
        "package.json"
            | "Cargo.toml"
            | "pyproject.toml"
            | "requirements.txt"
            | "go.mod"
            | "composer.json"
            | "Gemfile"
            | "vercel.json"
            | "netlify.toml"
            | "wrangler.toml"
            | "wrangler.json"
            | "wrangler.jsonc"
            | "railway.json"
            | "railway.toml"
            | "Dockerfile"
            | "docker-compose.yml"
            | "docker-compose.yaml"
            | "compose.yaml"
            | "compose.yml"
    )
}

fn dependency(name: &str, add: &mut impl FnMut(&str, &str)) {
    let known = match name.to_ascii_lowercase().as_str() {
        "next" => Some(("Next.js", "backend")),
        "react" => Some(("React", "other")),
        "vue" => Some(("Vue", "other")),
        "svelte" | "@sveltejs/kit" => Some(("Svelte", "other")),
        "typescript" => Some(("TypeScript", "other")),
        "vite" => Some(("Vite", "other")),
        "tailwindcss" => Some(("Tailwind CSS", "other")),
        "tauri" | "@tauri-apps/api" => Some(("Tauri", "other")),
        "react-native" => Some(("React Native", "other")),
        "expo" => Some(("Expo", "other")),
        "express" => Some(("Express", "backend")),
        "fastapi" => Some(("FastAPI", "backend")),
        "django" => Some(("Django", "backend")),
        "flask" => Some(("Flask", "backend")),
        "axum" => Some(("Axum", "backend")),
        "actix-web" => Some(("Actix Web", "backend")),
        "@supabase/supabase-js" | "supabase" => Some(("Supabase", "database")),
        "@neondatabase/serverless" => Some(("Neon", "database")),
        "pg" | "postgres" | "tokio-postgres" | "psycopg" | "psycopg2" | "psycopg2-binary" => {
            Some(("PostgreSQL", "database"))
        }
        "mongodb" | "mongoose" | "pymongo" => Some(("MongoDB", "database")),
        "redis" | "ioredis" => Some(("Redis", "database")),
        "better-sqlite3" | "sqlite3" | "rusqlite" => Some(("SQLite", "database")),
        "@prisma/client" | "prisma" => Some(("Prisma", "database")),
        "drizzle-orm" => Some(("Drizzle", "database")),
        "@clerk/nextjs" | "@clerk/clerk-react" => Some(("Clerk", "auth")),
        "next-auth" | "@auth/core" => Some(("Auth.js", "auth")),
        "@auth0/nextjs-auth0" | "@auth0/auth0-react" => Some(("Auth0", "auth")),
        "posthog-js" | "posthog-node" => Some(("PostHog", "analytics")),
        "@vercel/analytics" => Some(("Vercel Analytics", "analytics")),
        "stripe" | "@stripe/stripe-js" => Some(("Stripe", "other")),
        "resend" => Some(("Resend", "other")),
        "laravel/framework" => Some(("Laravel", "backend")),
        _ => None,
    };
    if let Some((service, category)) = known {
        add(service, category);
    }
}

fn inspect(name: &str, text: &str, add: &mut impl FnMut(&str, &str)) -> bool {
    match name {
        "package.json" | "composer.json" => {
            let Ok(doc) = serde_json::from_str::<Value>(text) else {
                return false;
            };
            add(
                if name == "package.json" {
                    "Node.js"
                } else {
                    "PHP"
                },
                "backend",
            );
            for section in [
                "dependencies",
                "devDependencies",
                "optionalDependencies",
                "require",
                "require-dev",
            ] {
                if let Some(deps) = doc.get(section).and_then(Value::as_object) {
                    for key in deps.keys() {
                        dependency(key, add);
                    }
                }
            }
        }
        "Cargo.toml" | "pyproject.toml" => {
            let parsed = config::Config::builder()
                .add_source(config::File::from_str(text, config::FileFormat::Toml))
                .build();
            let Ok(doc) = parsed.and_then(|config| config.try_deserialize::<Value>()) else {
                return false;
            };
            add(
                if name == "Cargo.toml" {
                    "Rust"
                } else {
                    "Python"
                },
                "backend",
            );
            for pointer in [
                "/dependencies",
                "/dev-dependencies",
                "/workspace/dependencies",
                "/tool/poetry/dependencies",
            ] {
                if let Some(deps) = doc.pointer(pointer).and_then(Value::as_object) {
                    for (key, value) in deps {
                        dependency(
                            value.get("package").and_then(Value::as_str).unwrap_or(key),
                            add,
                        );
                    }
                }
            }
            if let Some(deps) = doc
                .pointer("/project/dependencies")
                .and_then(Value::as_array)
            {
                for dep in deps.iter().filter_map(Value::as_str) {
                    dependency(dependency_name(dep), add);
                }
            }
        }
        "requirements.txt" => {
            add("Python", "backend");
            for line in text
                .lines()
                .map(str::trim)
                .filter(|line| !line.starts_with(['#', '-']))
            {
                dependency(dependency_name(line), add);
            }
        }
        "go.mod" => add("Go", "backend"),
        "Gemfile" => add("Ruby", "backend"),
        "vercel.json" => add("Vercel", "hosting"),
        "netlify.toml" => add("Netlify", "hosting"),
        "wrangler.toml" | "wrangler.json" | "wrangler.jsonc" => {
            add("Cloudflare Workers", "hosting")
        }
        "railway.json" | "railway.toml" => add("Railway", "hosting"),
        "Dockerfile"
        | "docker-compose.yml"
        | "docker-compose.yaml"
        | "compose.yaml"
        | "compose.yml" => add("Docker", "hosting"),
        _ => {}
    }
    true
}

fn dependency_name(value: &str) -> &str {
    value
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_' && c != '.')
        .next()
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_nested_projects_without_exposing_manifest_values_or_vendored_packages() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("apps/site")).unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules/noise")).unwrap();
        std::fs::write(dir.path().join("apps/site/package.json"), r#"{"dependencies":{"next":"16","@supabase/supabase-js":"https://secret@example.invalid","react":"19"}}"#).unwrap();
        std::fs::write(
            dir.path().join("node_modules/noise/package.json"),
            r#"{"dependencies":{"stripe":"1"}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join(".env"), "PRIVATE_TOKEN=secret").unwrap();
        let result = detect(dir.path()).unwrap();
        assert_eq!(result.scanned_files, 1);
        assert!(result
            .entries
            .iter()
            .any(|e| e.service_name == "Next.js" && e.evidence == ["apps/site/package.json"]));
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("secret"));
        assert!(!encoded.contains("Stripe"));
    }
    #[test]
    fn parses_rust_aliases_and_python_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[dependencies]\nweb = { package = 'axum', version = '0.8' }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("pyproject.toml"),
            "[project]\nname = 'sample'\ndependencies = ['fastapi>=0.1', 'psycopg[binary]>=3']\n",
        )
        .unwrap();
        let result = detect(dir.path()).unwrap();
        for expected in ["Rust", "Axum", "Python", "FastAPI", "PostgreSQL"] {
            assert!(
                result.entries.iter().any(|e| e.service_name == expected),
                "{expected}"
            );
        }
    }
    #[test]
    fn malformed_and_oversized_manifests_are_reported_as_incomplete() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), "not json").unwrap();
        std::fs::write(
            dir.path().join("requirements.txt"),
            vec![b'x'; MAX_BYTES as usize + 1],
        )
        .unwrap();
        let result = detect(dir.path()).unwrap();
        assert!(result.incomplete);
        assert!(result.entries.is_empty());
        assert!(detect(&dir.path().join("missing")).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn does_not_follow_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("package.json"), "{}").unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("package.json"),
            dir.path().join("package.json"),
        )
        .unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("linked")).unwrap();
        assert!(detect(dir.path()).unwrap().entries.is_empty());
    }

    #[test]
    fn bounds_large_workspaces_and_reports_remaining_folders() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..=MAX_FILES {
            let package = dir.path().join(format!("package-{i}"));
            std::fs::create_dir(&package).unwrap();
            std::fs::write(package.join("package.json"), "{}").unwrap();
        }
        let result = detect(dir.path()).unwrap();
        assert_eq!(result.scanned_files, MAX_FILES);
        assert!(result.incomplete);
    }
}
