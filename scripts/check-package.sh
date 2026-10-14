#!/usr/bin/env bash
# Packages the crate, then builds and runs consumer-test against the
# extracted .crate in a temporary directory. The consumer resolves its own
# dependencies, so missing files and unsupported version ranges show up.
# Arguments are passed to cargo package, such as --allow-dirty to check
# uncommitted work.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

cd "$root"
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
target=$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
cargo package --locked --list "$@"
cargo package --locked "$@"

tar -xzf "$target/package/synthient-$version.crate" -C "$work"
mv "$work/synthient-$version" "$work/synthient"
mkdir "$work/consumer"
cp -R consumer-test/Cargo.toml consumer-test/src "$work/consumer/"

cd "$work/consumer"
# Builds under the target directory, so CI caches keep compiled
# dependencies. The consumer still resolves them without a lockfile.
export CARGO_TARGET_DIR="$target/consumer"
cargo run --quiet
cargo run --quiet --features grpcschema
