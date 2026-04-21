# Justfile — Permagent Runtime (forked from Goose)

# list all tasks
default:
  @just --list

# Build release binaries (permagentd + permagent)
release-binary:
    @echo "Building release version..."
    cargo build --release -p permagent-daemon -p permagent-cli

# Run the daemon (permagentd) locally
run-server:
    @echo "Running daemon..."
    cargo run -p permagent-daemon --bin permagentd agent

# Run all style checks and formatting
check-everything:
    @echo "Running all style checks..."
    @echo "  → Formatting Rust code..."
    cargo fmt --all
    @echo "  → Running clippy linting..."
    cargo clippy --all-targets -- -D warnings
    @echo ""
    @echo "All style checks passed!"

# Build for Intel Mac
release-intel:
    @echo "Building release version for Intel Mac..."
    cargo build --release --target x86_64-apple-darwin -p permagent-daemon -p permagent-cli

# Generate OpenAPI specification
generate-openapi:
    @echo "Generating OpenAPI schema..."
    cargo run -p permagent-daemon --bin generate_schema

# Generate manpages for the CLI
generate-manpages:
    @echo "Generating manpages..."
    cargo run -p permagent-cli --bin generate_manpages
    @echo "Manpages generated at target/man/"

# Run Docusaurus server for documentation
run-docs:
    @echo "Running docs server..."
    cd documentation && yarn && yarn start

# validate the version is semver, and not the current version
validate version:
    #!/usr/bin/env bash
    if [[ ! "{{ version }}" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-.*)?$ ]]; then
      echo "[error]: invalid version '{{ version }}'."
      echo "  expected: semver format major.minor.patch or major.minor.patch-<suffix>"
      exit 1
    fi

    current_version=$(just get-tag-version)
    if [[ "{{ version }}" == "$current_version" ]]; then
      echo "[error]: current_version '$current_version' is the same as target version '{{ version }}'"
      echo "  expected: new version in semver format"
      exit 1
    fi

# extract version from Cargo.toml
get-tag-version:
    @uvx --from=toml-cli toml get --toml-path=Cargo.toml "workspace.package.version"

# update version numbers in all manifests
bump-version version:
    @just validate {{ version }} || exit 1
    @uvx --from=toml-cli toml set --toml-path=Cargo.toml "workspace.package.version" {{ version }}
    @cargo update --workspace

# rebuild canonical model registry and mapping report from models.dev
build-canonical-models:
    @cargo run --bin build_canonical_models

# bump version, rebuild canonical models, and commit
prepare-release version:
    @just bump-version {{ version }}
    @just build-canonical-models
    @git add \
        Cargo.toml \
        Cargo.lock
    @git commit --message "chore(release): release version {{ version }}"

# create the git tag from Cargo.toml
tag:
    git tag v$(just get-tag-version)

# create tag and push to origin
tag-push: tag
    git push origin tag v$(just get-tag-version)

# generate release notes from git commits
release-notes old:
    #!/usr/bin/env bash
    git log --pretty=format:"- %s" {{ old }}..v$(just get-tag-version)

# Build test tools
build-test-tools:
    cargo build -p goose-test

# Record MCP test fixtures
record-mcp-tests: build-test-tools
    GOOSE_RECORD_MCP=1 cargo test --package permagent --test mcp_integration_test
    git add crates/goose/tests/mcp_replays/
