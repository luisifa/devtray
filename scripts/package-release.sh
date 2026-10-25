#!/usr/bin/env bash
# Build the portable Linux x86_64 release and package it without installing it.
set -euo pipefail
cd "$(dirname "$0")/.."

release_target=x86_64-unknown-linux-musl
release_version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
release_name="devtray-${release_version}-${release_target}"
release_stage="target/release-staging/${release_name}"

cargo build --release --locked --target "$release_target"
python3 scripts/generate-third-party-notices.py --target "$release_target"
if [ -d "$release_stage" ]; then rm -r "$release_stage"; fi
mkdir -p dist "$release_stage"
cargo deb --no-build --locked --target "$release_target" \
  --output "dist/devtray_${release_version}-1_amd64.deb"
cargo generate-rpm --target "$release_target" \
  --output "dist/devtray-${release_version}-1.x86_64.rpm"

install -m755 "target/$release_target/release/devtray" "$release_stage/devtray"
strip "$release_stage/devtray"
cp README.md README.zh-CN.md CHANGELOG.md CHANGELOG.zh-CN.md Cargo.lock "$release_stage/"
cp LICENSE THIRD_PARTY_NOTICES.md "$release_stage/"
cp -R licenses "$release_stage/"
cp -R examples assets docs packaging "$release_stage/"
tar -C target/release-staging -czf "dist/$release_name.tar.gz" "$release_name"
(
  cd dist
  sha256sum "devtray_${release_version}-1_amd64.deb" \
    "devtray-${release_version}-1.x86_64.rpm" \
    "$release_name.tar.gz" > SHA256SUMS
)
echo "Release artifacts: $PWD/dist"
