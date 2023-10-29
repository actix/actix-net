_list:
    @just --list

# Document crates in workspace.
doc:
    RUSTDOCFLAGS="--cfg=docsrs" cargo +nightly doc --no-deps --workspace --features=rustls,openssl

# Document crates in workspace and watch for changes.
doc-watch:
    RUSTDOCFLAGS="--cfg=docsrs" cargo +nightly doc --no-deps --workspace --features=rustls,openssl --open
    cargo watch -- RUSTDOCFLAGS="--cfg=docsrs" cargo +nightly doc --no-deps --workspace --features=rustls,openssl

# Check for unintentional external type exposure on all crates in workspace.
check-external-types-all:
    #!/usr/bin/env bash
    set -euo pipefail
    for f in $(find . -mindepth 2 -maxdepth 2 -name Cargo.toml); do
        echo "Checking for $f"
        just check-external-types-manifest "$f" || true
        echo
        echo
    done

# Check for unintentional external type exposure on all crates in workspace.
check-external-types-all-table:
    #!/usr/bin/env bash
    set -euo pipefail
    for f in $(find . -mindepth 2 -maxdepth 2 -name Cargo.toml); do
        echo
        echo "Checking for $f"
        just check-external-types-manifest "$f" --output-format=markdown-table
    done

# Check for unintentional external type exposure on a crate.
check-external-types-manifest manifest_path *extra_args:
    cargo +nightly check-external-types --manifest-path "{{manifest_path}}" {{extra_args}}
