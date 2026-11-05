#!/bin/sh
# Runs scripts/build-linux.sh inside the Debian 12 build image
# (packaging/linux/build.Dockerfile), so the release executable needs only
# glibc 2.36 instead of the host's (issue #1026). The checkout, the target
# directory, HEX_RELEASE_DIST, and the host's Rust toolchain are mounted at
# their own paths and the build runs as the calling user, so later steps on
# the host find every file where a native build leaves it. Prints only the
# executable's path, like build-linux.sh.
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
dist=${HEX_RELEASE_DIST:-"$root/dist"}
cargo_home=${CARGO_HOME:-"$HOME/.cargo"}
rustup_home=${RUSTUP_HOME:-"$HOME/.rustup"}
dockerfile="$root/packaging/linux/build.Dockerfile"
# Tagged by the Dockerfile's digest, so an unchanged image is never rebuilt.
image=hex-linux-build:$(
  { sha256sum "$dockerfile" 2>/dev/null || shasum -a 256 "$dockerfile"; } | cut -c1-16
)
# HEX_LINUX_BUILD_IMAGE_ARCHIVE: a zstd image archive CI restores from and
# saves to its cache, since a fresh runner has no image.
archive=${HEX_LINUX_BUILD_IMAGE_ARCHIVE:-}

for command in docker rustup; do
  command -v "$command" >/dev/null 2>&1 || {
    echo "Linux release builds require $command." >&2
    exit 1
  }
done
mkdir -p "$dist" "$cargo_home" "$rustup_home"
dist=$(realpath "$dist") cargo_home=$(realpath "$cargo_home") rustup_home=$(realpath "$rustup_home")
# The toolchain's own directory, so the build needs no rustup proxy in PATH.
toolchain=$(cd "$root" && dirname "$(rustup which cargo)")
if ! docker image inspect "$image" >/dev/null 2>&1 && [ -n "$archive" ] && [ -f "$archive" ]; then
  zstd -dcq "$archive" | docker load --quiet >&2
fi
if ! docker image inspect "$image" >/dev/null 2>&1; then
  docker build --quiet --tag "$image" --file "$dockerfile" "$root/packaging/linux" >&2
  if [ -n "$archive" ]; then
    mkdir -p "$(dirname "$archive")"
    docker save "$image" | zstd -T0 -3 -q -o "$archive"
  fi
fi
home=$(mktemp -d)
trap 'rm -rf "$home"' EXIT HUP INT TERM
docker run --rm --user "$(id -u):$(id -g)" \
  --volume "$root:$root" --volume "$dist:$dist" \
  --volume "$cargo_home:$cargo_home" --volume "$rustup_home:$rustup_home" \
  --volume "$home:$home" \
  --env HOME="$home" --env CARGO_HOME="$cargo_home" --env RUSTUP_HOME="$rustup_home" \
  --volume "$toolchain:$toolchain:ro" \
  --env PATH="$toolchain:$cargo_home/bin:/usr/local/bin:/usr/bin:/bin" \
  --env HEX_RELEASE_DIST="$dist" \
  --env HEX_VERSION --env HEX_BUILD_CHANNEL --env HEX_TEST_BUILD \
  --env CARGO_BUILD_JOBS --env OPENSSL_NO_VENDOR --env CC --env CXX \
  --workdir "$root" "$image" scripts/build-linux.sh
