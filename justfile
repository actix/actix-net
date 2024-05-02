_list:
    @just --list

# Downgrade dev-dependencies necessary to run MSRV checks/tests.
[private]
downgrade-msrv:
    cargo update -p=ciborium --precise=0.2.1
    cargo update -p=ciborium-ll --precise=0.2.1
    cargo update -p=time --precise=0.3.16
    cargo update -p=clap --precise=4.3.24
    cargo update -p=clap_lex --precise=0.5.0
    cargo update -p=anstyle --precise=1.0.2
    cargo update -p=trybuild --precise=1.0.89

# Document crates in workspace.
doc:
    RUSTDOCFLAGS="--cfg=docsrs" cargo +nightly doc --no-deps --workspace --features=rustls,openssl

# Document crates in workspace and watch for changes.
doc-watch:
    RUSTDOCFLAGS="--cfg=docsrs"                cargo +nightly doc --no-deps --workspace --features=rustls-0_20,rustls-0_21,rustls-0_20-native-roots,rustls-0_21-native-roots,openssl --open
    cargo watch -- RUSTDOCFLAGS="--cfg=docsrs" cargo +nightly doc --no-deps --workspace --features=rustls-0_20,rustls-0_21,rustls-0_20-native-roots,rustls-0_21-native-roots,openssl

# Check for unintentional external type exposure on all crates in workspace.
check-external-types-all toolchain="+nightly":
    #!/usr/bin/env bash
    set -euo pipefail
    exit=0
    for f in $(find . -mindepth 2 -maxdepth 2 -name Cargo.toml | grep -vE "\-codegen/|\-derive/|\-macros/"); do
        if ! just check-external-types-manifest "$f" {{toolchain}}; then exit=1; fi
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
        just check-external-types-manifest "$f" {{toolchain}} --output-format=markdown-table
    done

# Check for unintentional external type exposure on a crate.
check-external-types-manifest manifest_path toolchain="+nightly" *extra_args="":
    cargo {{toolchain}} check-external-types --manifest-path "{{manifest_path}}" {{extra_args}}
