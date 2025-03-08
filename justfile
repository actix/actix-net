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

# Downgrade dev-dependencies necessary to run MSRV checks/tests.
[private]
downgrade-for-msrv:
    cargo update -p=clap --precise=4.4.18
    cargo update -p=native-tls --precise=0.2.13
    cargo update -p=litemap --precise=0.7.4
    cargo update -p=zerofrom --precise=0.1.5

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

# Run Clippy over workspace.
clippy toolchain="":
    cargo {{ toolchain }} clippy --workspace --all-targets {{ all_crate_features }}

# Test workspace code.
[macos]
[windows]
test toolchain="":
    cargo {{ toolchain }} test --lib --tests --package=actix-macros
    cargo {{ toolchain }} nextest run --no-tests=warn --workspace --exclude=actix-macros --no-default-features
    cargo {{ toolchain }} nextest run --no-tests=warn --workspace --exclude=actix-macros {{ all_crate_features }}

# Test workspace code.
[linux]
test toolchain="":
    cargo {{ toolchain }} test --lib --tests --package=actix-macros
    cargo {{ toolchain }} nextest run --no-tests=warn --workspace --exclude=actix-macros --no-default-features
    cargo {{ toolchain }} nextest run --no-tests=warn --workspace --exclude=actix-macros {{ non_linux_all_features_list }}
    cargo {{ toolchain }} nextest run --no-tests=warn --workspace --exclude=actix-macros {{ all_crate_features }}

# Test workspace using MSRV.
test-msrv: downgrade-for-msrv (test msrv_rustup)

# Test workspace docs.
test-docs toolchain="": && doc
    cargo {{ toolchain }} test --doc --workspace {{ all_crate_features }} --no-fail-fast -- --nocapture

# Test workspace.
test-all toolchain="": (test toolchain) (test-docs toolchain)

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
check-external-types-all toolchain="+nightly":
    #!/usr/bin/env bash
    set -euo pipefail
    exit=0
    for f in $(find . -mindepth 2 -maxdepth 2 -name Cargo.toml | grep -vE "\-codegen/|\-derive/|\-macros/"); do
        if ! just check-external-types-manifest "$f" {{ toolchain }}; then exit=1; fi
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
        just check-external-types-manifest "$f" {{ toolchain }} --output-format=markdown-table
    done

# Check for unintentional external type exposure on a crate.
check-external-types-manifest manifest_path toolchain="+nightly" *extra_args="":
    cargo {{ toolchain }} check-external-types --manifest-path "{{ manifest_path }}" {{ extra_args }}
