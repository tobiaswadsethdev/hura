#!/bin/sh
# Archives the app and uploads it to TestFlight, for the internal group.
#
# Run from a clean checkout of what is to ship, usually `main` after a merge:
#
#   apps/ios/Tools/testflight.sh
#
# Needs Xcode signed in to an account on the team in Config/Hura.xcconfig
# (Xcode > Settings > Accounts), which is what -allowProvisioningUpdates and
# the upload both use; xcodegen; and the Rust iOS targets build-core.sh needs.
# App Store Connect gives the build its number. Once it has processed (a few
# minutes; App Store Connect mails when it is done), the internal testers get
# it without anything more being done.
#
# HURA_TESTFLIGHT_ANY_BRANCH=1 uploads from a branch other than main, or with
# uncommitted changes, for the rare build that has to go out first.
set -eu
cd "$(dirname "$0")/.."
ios=$(pwd)

if [ "${HURA_TESTFLIGHT_ANY_BRANCH:-}" != 1 ]; then
  branch=$(git rev-parse --abbrev-ref HEAD)
  if [ "$branch" != main ]; then
    echo "on $branch, not main; HURA_TESTFLIGHT_ANY_BRANCH=1 to upload it anyway" >&2
    exit 1
  fi
  if [ -n "$(git status --porcelain)" ]; then
    echo "the checkout has uncommitted changes, which would ship unreviewed" >&2
    exit 1
  fi
fi

# The archive is built from the Rust as it is now, not a stale framework from
# the last time the tests ran.
Tools/build-core.sh
xcodegen --quiet

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

echo "archiving $(git rev-parse --short HEAD)"
xcodebuild archive \
  -project Hura.xcodeproj -scheme Hura -configuration Release \
  -destination generic/platform=iOS \
  -archivePath "$work/Hura.xcarchive" \
  -allowProvisioningUpdates -skipPackagePluginValidation \
  >"$work/archive.log" 2>&1 || {
  tail -40 "$work/archive.log" >&2
  exit 1
}

echo "uploading to App Store Connect"
xcodebuild -exportArchive \
  -archivePath "$work/Hura.xcarchive" \
  -exportOptionsPlist "$ios/Config/ExportOptions.plist" \
  -exportPath "$work/export" \
  -allowProvisioningUpdates \
  >"$work/export.log" 2>&1 || {
  tail -40 "$work/export.log" >&2
  exit 1
}
grep -E "Upload succeeded|EXPORT SUCCEEDED" "$work/export.log" || true
echo "uploaded; TestFlight has it once App Store Connect has processed it"
