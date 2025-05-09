_list:
    @just --list

# Check project.
check: && clippy
    just --unstable --fmt --check
    # nixpkgs-fmt --check .
    fd --hidden --type=file -e=md -e=yml --exec-batch prettier --check
    fd --hidden -e=toml --exec-batch taplo format --check
    fd --hidden -e=toml --exec-batch taplo lint
    cargo +nightly fmt -- --check

# Format project.
fmt:
    just --unstable --fmt
    # nixpkgs-fmt .
    fd --hidden --type=file -e=md -e=yml --exec-batch prettier --write
    fd --type=file --hidden -e=toml --exec-batch taplo format
    cargo +nightly fmt

# Downgrade dependencies necessary to run MSRV checks/tests.
[private]
downgrade-for-msrv:
    cargo {{ toolchain }} update -p=native-tls --precise=0.2.13 # next ver: 1.80.0
    cargo {{ toolchain }} update -p=idna_adapter --precise=1.2.0 # next ver: 1.82.0
    cargo {{ toolchain }} update -p=litemap --precise=0.7.4 # next ver: 1.81.0
    cargo {{ toolchain }} update -p=zerofrom --precise=0.1.5 # next ver: 1.81.0
    cargo {{ toolchain }} update -p=half --precise=2.4.1 # next ver: 1.81.0

msrv := ```
    cargo metadata --format-version=1 \
    | jq -r 'first(.packages[] | select(.source == null and .rust_version)) | .rust_version' \
    | sed -E 's/^1\.([0-9]{2})$/1\.\1\.0/'
```
msrv_rustup := "+" + msrv
non_linux_all_features_list := ```
    cargo metadata --format-version=1 \
    | jq '.packages[] | select(.source == null) | .features | keys' \
    | jq -r --slurp \
        --arg exclusions "tokio-uring,io-uring" \
        'add | unique | . - ($exclusions | split(",")) | join(",")'
```
all_crate_features := if os() == "linux" { "--all-features" } else { "--features='" + non_linux_all_features_list + "'" }

toolchain := ""

# Run Clippy over workspace.
clippy:
    cargo {{ toolchain }} clippy --workspace --all-targets {{ all_crate_features }}

# Run Clippy using MSRV.
clippy-msrv: downgrade-for-msrv
    @just toolchain={{ msrv_rustup }} clippy

# Test workspace code.
[macos]
[windows]
test:
    cargo {{ toolchain }} test --lib --tests --package=actix-macros
    cargo {{ toolchain }} nextest run --no-tests=warn --workspace --exclude=actix-macros --no-default-features
    cargo {{ toolchain }} nextest run --no-tests=warn --workspace --exclude=actix-macros {{ all_crate_features }}

# Test workspace code.
[linux]
test:
    cargo {{ toolchain }} test --lib --tests --package=actix-macros
    cargo {{ toolchain }} nextest run --no-tests=warn --workspace --exclude=actix-macros --no-default-features
    cargo {{ toolchain }} nextest run --no-tests=warn --workspace --exclude=actix-macros {{ non_linux_all_features_list }}
    cargo {{ toolchain }} nextest run --no-tests=warn --workspace --exclude=actix-macros {{ all_crate_features }}

# Test workspace using MSRV.
test-msrv: downgrade-for-msrv
    @just toolchain={{ msrv_rustup }} test

# Test workspace docs.
test-docs: && doc
    cargo {{ toolchain }} test --doc --workspace {{ all_crate_features }} --no-fail-fast -- --nocapture

# Test workspace.
test-all: test test-docs

# Document crates in workspace.
doc *args: && doc-set-workspace-crates
    rm -f "$(cargo metadata --format-version=1 | jq -r '.target_directory')/doc/crates.js"
    RUSTDOCFLAGS="--cfg=docsrs -Dwarnings" cargo +nightly doc --no-deps --workspace {{ all_crate_features }} {{ args }}

[private]
doc-set-workspace-crates:
    #!/usr/bin/env bash
    (
        echo "window.ALL_CRATES ="
        cargo metadata --format-version=1 \
        | jq '[.packages[] | select(.source == null) | .targets | map(select(.doc) | .name)] | flatten'
        echo ";"
    ) > "$(cargo metadata --format-version=1 | jq -r '.target_directory')/doc/crates.js"

# Document crates in workspace and watch for changes.
doc-watch:
    @just doc --open
    cargo watch -- just doc

# Check for unintentional external type exposure on all crates in workspace.
check-external-types-all:
    #!/usr/bin/env bash
    set -euo pipefail
    exit=0
    for f in $(find . -mindepth 2 -maxdepth 2 -name Cargo.toml | grep -vE "\-codegen/|\-derive/|\-macros/"); do
        if ! just toolchain="+nightly" check-external-types-manifest "$f"; then exit=1; fi
        echo
        echo
    done
    exit $exit

# Check for unintentional external type exposure on all crates in workspace.
check-external-types-all-table toolchain="+nightly":
    #!/usr/bin/env bash
    set -euo pipefail
    for f in $(find . -mindepth 2 -maxdepth 2 -name Cargo.toml | grep -vE "\-codegen/|\-derive/|\-macros/"); do
        echo
        echo "Checking for $f"
        just toolchain="+nightly" check-external-types-manifest "$f" --output-format=markdown-table
    done

# Check for unintentional external type exposure on a crate.
check-external-types-manifest manifest_path *extra_args="":
    cargo {{ toolchain }} check-external-types --manifest-path "{{ manifest_path }}" {{ extra_args }}
