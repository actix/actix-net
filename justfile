_list:
    @just --list

# Downgrade dev-dependencies necessary to run MSRV checks/tests.
[private]
downgrade-for-msrv:
    cargo update -p=ciborium --precise=0.2.1
    cargo update -p=ciborium-ll --precise=0.2.1
    cargo update -p=time --precise=0.3.16
    cargo update -p=clap --precise=4.3.24
    cargo update -p=clap_lex --precise=0.5.0
    cargo update -p=anstyle --precise=1.0.2
    cargo update -p=trybuild --precise=1.0.89

msrv := ```
    cargo metadata --format-version=1 \
    | jq -r 'first(.packages[] | select(.source == null and .name == "actix-tls")) | .rust_version'
```
msrv_full := msrv + ".0" # comment out if the MSRV has a patch version specified
msrv_rustup := "+" + msrv_full

non_linux_all_features_list := ```
    cargo metadata --format-version=1 \
    | jq '.packages[] | select(.source == null) | .features | keys' \
    | jq -r --slurp \
        --arg exclusions "tokio-uring,io-uring" \
        'add | unique | . - ($exclusions | split(",")) | join(",")'
```

all_crate_features := if os() == "linux" {
    "--all-features"
} else {
    "--features='" + non_linux_all_features_list + "'"
}

# Test workspace code.
test toolchain="":
    cargo {{ toolchain }} test --lib --tests --package=actix-macros
    cargo {{ toolchain }} nextest run --workspace --exclude=actix-macros --no-default-features
    cargo {{ toolchain }} nextest run --workspace --exclude=actix-macros {{ all_crate_features }}

# Test workspace using MSRV.
test-msrv: downgrade-for-msrv (test msrv_rustup)

# Test workspace docs.
test-docs toolchain="": && doc
    cargo {{ toolchain }} test --doc --workspace {{ all_crate_features }} --no-fail-fast -- --nocapture

# Test workspace.
test-all toolchain="": (test toolchain) (test-docs)

# Document crates in workspace.
doc *args:
    RUSTDOCFLAGS="--cfg=docsrs -Dwarnings" cargo +nightly doc --no-deps --workspace {{ all_crate_features }} {{ args }}

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
