#!/bin/sh
# Builds crates/hura-mobile -- hura-core's tracker client -- as
# HuraKit/Build/HuraCore.xcframework, which HuraKit links. Run it once after a
# checkout and again after changing the Rust; the app and `swift test` fail
# to link without it.
#
# Needs the iOS targets: rustup target add aarch64-apple-ios aarch64-apple-ios-sim
set -eu
cd "$(dirname "$0")/../../.."
out=apps/ios/HuraKit/Build
profile=${HURA_CORE_PROFILE:-release}
flag=$([ "$profile" = release ] && echo --release || echo)

# The phone, the simulator on Apple silicon, and the Mac for `swift test`.
targets="aarch64-apple-ios aarch64-apple-ios-sim aarch64-apple-darwin"
for t in $targets; do
  # The deployment targets the app and HuraKit build for, so the linker
  # doesn't warn about every object.
  case $t in
    *-ios*) IPHONEOS_DEPLOYMENT_TARGET=17.0 cargo build -p hura-mobile $flag --target "$t" ;;
    *) MACOSX_DEPLOYMENT_TARGET=14.0 cargo build -p hura-mobile $flag --target "$t" ;;
  esac
done

rm -rf "$out/HuraCore.xcframework"
mkdir -p "$out"
args=""
for t in $targets; do
  args="$args -library target/$t/$profile/libhura_mobile.a -headers crates/hura-mobile/include"
done
# shellcheck disable=SC2086
xcodebuild -create-xcframework $args -output "$out/HuraCore.xcframework" >/dev/null
echo "built $out/HuraCore.xcframework"
