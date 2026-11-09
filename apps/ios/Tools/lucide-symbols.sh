#!/bin/sh
# Makes the app's Lucide icons into custom symbols in the asset catalog.
#
#   apps/ios/Tools/lucide-symbols.sh
#
# Reads the icon names from Hura/Views/Glyph.swift -- each `case x = "name"` is
# a Lucide icon -- fetches Lucide's SVGs at the version the desktop pins, and
# writes Hura/Assets.xcassets/Lucide/lucide.<name>.symbolset for each, with the
# ultralight and black weights made from the stroke width so a symbol takes the
# weight of the text beside it. Symbols no longer named in Glyph.swift are
# removed. The output is checked in, so building the app needs none of this.
#
# Needs npm and SwiftDraw (`brew install swiftdraw`).
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
glyphs="$here/Hura/Views/Glyph.swift"
out="$here/Hura/Assets.xcassets/Lucide"
# The desktop's lucide-react, so both apps draw the same icon for a name.
version=$(sed -n 's/.*"lucide-react": "[^0-9]*\([0-9.]*\)".*/\1/p' "$here/../desktop/package.json")

command -v swiftdraw >/dev/null || { echo "needs SwiftDraw: brew install swiftdraw" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
(cd "$work" && npm pack --silent "lucide-static@$version" >/dev/null && tar xzf lucide-static-*.tgz)
svgs="$work/package/icons"

# Lucide's licence asks for its notice to travel with the icons: beside the
# app's sources, and so in the app bundle.
cp "$work/package/LICENSE" "$here/Hura/Lucide-LICENSE.txt"

mkdir -p "$out"
cat > "$out/Contents.json" <<'JSON'
{
  "info" : { "author" : "xcode", "version" : 1 }
}
JSON

# Whether a symbol's three weights have the same paths, command for command.
consistent() {
    python3 -I - "$1" <<'PY'
import re, sys
s = open(sys.argv[1]).read()
def shape(group):
    g = re.search(r'id="%s".*?</g>' % group, s, re.S).group(0)
    return [re.sub(r"[^A-Za-z]", "", d) for d in re.findall(r' d="([^"]+)"', g)]
sys.exit(0 if shape("Ultralight-S") == shape("Regular-S") == shape("Black-S") else 1)
PY
}

names=$(sed -n 's/^ *case [A-Za-z]* = "\([a-z0-9-]*\)"$/\1/p' "$glyphs" | sort -u)
for set in "$out"/*.symbolset; do
    [ -e "$set" ] || continue
    name=$(basename "$set" .symbolset)
    echo "$names" | grep -qx "${name#lucide.}" || rm -rf "$set"
done
for name in $names; do
    [ -f "$svgs/$name.svg" ] || { echo "no Lucide icon named $name (lucide-static $version)" >&2; exit 1; }
    set="$out/lucide.$name.symbolset"
    mkdir -p "$set"
    # Lucide draws on a 24-unit grid inside a 2-unit margin. Cropping the
    # margin puts its live area on the cap height, which is where SF Symbols
    # sit; `auto` kept the margin and drew every icon at two thirds the size
    # of the text beside it.
    swiftdraw "$svgs/$name.svg" --format sfsymbol --insets 2,2,2,2 \
        --ultralight-stroke-width 50% --black-stroke-width 200% \
        --output "$set/lucide.$name.svg" >/dev/null
    # A symbol's weights have to be drawn with the same paths, point for
    # point, for iOS to blend between them -- and when they are not, it draws
    # nothing at all. Outlining a thinner or thicker stroke sometimes splits a
    # path differently, so check, and where it did, give the icon one weight.
    if ! consistent "$set/lucide.$name.svg"; then
        swiftdraw "$svgs/$name.svg" --format sfsymbol --insets 2,2,2,2 \
            --output "$set/lucide.$name.svg" >/dev/null
        echo "lucide.$name: one weight only (its weights could not be drawn alike)"
    fi
    cat > "$set/Contents.json" <<JSON
{
  "info" : { "author" : "xcode", "version" : 1 },
  "symbols" : [ { "filename" : "lucide.$name.svg", "idiom" : "universal" } ]
}
JSON
done
echo "$(echo "$names" | wc -l | tr -d ' ') symbols from lucide-static $version in $out"
