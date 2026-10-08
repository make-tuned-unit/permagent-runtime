#[allow(dead_code)]
#[path = "src/runtime_target.rs"]
mod runtime_target;

use std::{env, fs, path::Path};

// A cached web build uses /ui/; the native custom protocol serves dist at /.
// Validate the known Vite-emitted script/link format even when beforeBuild is skipped.
fn validate_native_assets() {
    if env::var_os("CARGO_FEATURE_CUSTOM_PROTOCOL").is_none() {
        return;
    }
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../command-center/dist");
    let index = dist.join("index.html");
    println!("cargo:rerun-if-changed={}", index.display());
    let html = fs::read_to_string(&index).expect("native frontend index.html is missing");
    for tag in html.split('<') {
        let attribute = if tag.starts_with("script ") {
            "src"
        } else if tag.starts_with("link ") {
            "href"
        } else {
            continue;
        };
        let tag = tag.split('>').next().unwrap_or(tag);
        let value = ['\"', '\''].into_iter().find_map(|quote| {
            let prefix = format!("{attribute}={quote}");
            tag.split_once(&prefix)
                .and_then(|(_, rest)| rest.split(quote).next())
        });
        let Some(value) = value else { continue };
        if value.starts_with("https://") || value.starts_with("http://") || value.starts_with("//")
        {
            continue;
        }
        let path = value.split(['?', '#']).next().unwrap_or(value);
        assert!(
            !path.starts_with("/ui/"),
            "native frontend has web-base asset {value}; rebuild with TAURI_ENV_PLATFORM=darwin"
        );
        let relative = Path::new(path.trim_start_matches('/'));
        assert!(
            relative.components().all(|component| matches!(
                component,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )),
            "invalid native asset path {value}"
        );
        let asset = dist.join(relative);
        assert!(
            asset.is_file(),
            "native frontend asset {value} is missing at {}",
            asset.display()
        );
        println!("cargo:rerun-if-changed={}", asset.display());
    }
}

fn main() {
    for name in [
        "PERMAGENT_REVIEW_ROOT",
        "PERMAGENT_REVIEW_PORT",
        "VITE_DAEMON_URL",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let root = env::var("PERMAGENT_REVIEW_ROOT").ok();
    let port = env::var("PERMAGENT_REVIEW_PORT").ok();
    let home = env::var("HOME").expect("HOME is required for build target validation");
    let target = runtime_target::RuntimeTarget::parse(
        root.as_deref(),
        port.as_deref(),
        env::var("PROFILE").as_deref() == Ok("debug"),
        Path::new(&home),
    )
    .expect("invalid native review build target");
    if target.review {
        target
            .validate_review_root()
            .expect("invalid isolated review root");
        assert_eq!(
            env::var("VITE_DAEMON_URL").ok().as_deref(),
            Some(target.base_url().as_str()),
            "review frontend API target must match native target during the build"
        );
    }
    validate_native_assets();
    tauri_build::build()
}
