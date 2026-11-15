#!/bin/sh
# Archives the app and uploads it to TestFlight, for the internal group.
#
# Run from a clean checkout of what is to ship, usually `main` after a merge:
#
#   apps/ios/Tools/testflight.sh
#
# Needs Xcode signed in to an account on the team in Config/Hura.xcconfig
# (Xcode > Settings > Accounts), which is what -allowProvisioningUpdates and
# the upload both use; and xcodegen.
#
# The version is MARKETING_VERSION in Config/Version.xcconfig, moved by the
# "Tag a release" workflow. The build number is the number of commits in what
# is archived, so it goes up with every merge to main and says which commit a
# tester is running; the upload is tagged `ios-v<version>+<build>` and the tag
# pushed. Once App Store Connect has processed it (a few minutes; it mails when
# it is done), the internal testers get it without anything more being done.
#
# HURA_TESTFLIGHT_ANY_BRANCH=1 uploads from a branch other than main, or with
# uncommitted changes, for the rare build that has to go out first. That build
# is not tagged, and its number is its branch's commit count: a later main
# build with the same count is refused by App Store Connect, so merge first and
# upload again from main.
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

# A shallow clone counts the commits it has, not the ones there are, and would
# hand App Store Connect a number lower than the last upload's.
if [ "$(git rev-parse --is-shallow-repository)" = true ]; then
  echo "the checkout is shallow, so it cannot count commits; git fetch --unshallow" >&2
  exit 1
fi
version=$(sed -n 's/^MARKETING_VERSION = \(.*\)$/\1/p' Config/Version.xcconfig)
build=$(git rev-list --count HEAD)
tag="ios-v${version}+${build}"
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  echo "$tag was already uploaded, and App Store Connect takes a build number once" >&2
  exit 1
fi

xcodegen --quiet

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

echo "archiving $(git rev-parse --short HEAD) as ${version} (${build})"
xcodebuild archive \
  -project Hura.xcodeproj -scheme Hura -configuration Release \
  -destination generic/platform=iOS \
  -archivePath "$work/Hura.xcarchive" \
  -allowProvisioningUpdates -skipPackagePluginValidation \
  CURRENT_PROJECT_VERSION="$build" \
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
echo "uploaded ${version} (${build}); TestFlight has it once App Store Connect has processed it"

# What was uploaded, findable from the number a tester reads off the About
# screen. Only for a build of main: anything else may never land as it is.
if [ "${HURA_TESTFLIGHT_ANY_BRANCH:-}" != 1 ]; then
  git tag -a "$tag" -m "hura for iOS ${version} (${build})"
  git push --quiet origin "refs/tags/$tag" && echo "tagged $tag" ||
    echo "tagged $tag here, but could not push it: git push origin $tag" >&2
fi
