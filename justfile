_list:
    @just --list

toolchain := ""

# Check project.
check: && clippy
    just --unstable --fmt --check
    # nixpkgs-fmt --check .
    fd --hidden --type=file -e=md -e=yml --exec-batch prettier --check
    fd --hidden -e=toml --exec-batch taplo format --check
    fd --hidden -e=toml --exec-batch taplo lint
    cargo +nightly fmt -- --check

# Format project.
fmt: update-readmes
    just --unstable --fmt
    # nixpkgs-fmt .
    fd --hidden --type=file -e=md -e=yml --exec-batch prettier --write
    fd --type=file --hidden -e=toml --exec-batch taplo format
    cargo +nightly fmt

# Update READMEs from crate root documentation.
update-readmes:
    cd ./resolvconf && cargo rdme --force
    fd README.md --exec-batch prettier --write

# Downgrade dependencies necessary to run MSRV checks/tests.
[private]
downgrade-for-msrv:

msrv := ```
    cargo metadata --format-version=1 \
    | jq -r 'first(.packages[] | select(.source == null and .rust_version)) | .rust_version' \
    | sed -E 's/^1\.([0-9]{2})$/1\.\1\.0/'
```
msrv_rustup := "+" + msrv
all_crate_features := "--all-features"

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
    cargo {{ toolchain }} nextest run --no-tests=warn --workspace --exclude=actix-macros {{ all_crate_features }}

# Test workspace using MSRV.
test-msrv: downgrade-for-msrv
    @just toolchain={{ msrv_rustup }} test

# Test workspace docs.
test-docs: && doc
    cargo {{ toolchain }} test --doc --workspace {{ all_crate_features }} --no-fail-fast -- --nocapture

# Test workspace.
test-all: test test-docs

# Validate benchmark outputs without collecting timings.
bench-test:
    cargo bench -p bytestring --bench utf8 --all-features --locked -- --test

# Build benchmarks for CPU and memory reporting.
bench-build:
    cargo codspeed build -p bytestring --all-features --locked -m simulation -m memory

# Run the benchmarks built with CodSpeed.
bench-run:
    cargo codspeed run

# Build benchmarks for local walltime measurements.
bench-build-walltime *args:
    cargo codspeed build -p bytestring --locked -m walltime {{ args }}

# Measure walltime and upload the results to CodSpeed.
bench-run-walltime *args:
    codspeed run -m walltime -- cargo codspeed run -m walltime {{ args }}

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

# Check security advisories.
check-advisories:
    cargo deny --log-level warn --workspace --all-features check advisories

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
