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
    validate_native_assets();
    tauri_build::build()
}
