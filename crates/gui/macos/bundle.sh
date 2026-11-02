#!/bin/sh
# Build target/release/bundle/lastissue.app (GUI-SPEC §10, M6).
#
# Unsigned local build: the bundle gets an ad-hoc signature only (no
# Developer ID, no notarization). Nothing is installed or registered with
# LaunchServices; copy the .app to /Applications yourself to install it
# (that also registers the lastissue:// URL scheme).
set -eu

here=$(cd "$(dirname "$0")" && pwd)
ws=$(cd "$here/../../.." && pwd)
target=${CARGO_TARGET_DIR:-$ws/target}
out="$target/release/bundle"
app="$out/lastissue.app"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ws/Cargo.toml" | head -1)

cargo build --release --locked -p gui --manifest-path "$ws/Cargo.toml"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$target/release/lastissue-gui" "$app/Contents/MacOS/lastissue-gui"
sed "s/@VERSION@/$version/g" "$here/Info.plist" > "$app/Contents/Info.plist"
plutil -lint -s "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"

# Placeholder icon: one 1024px PNG scaled into an .iconset, then .icns.
icon="$out/icon-work"
rm -rf "$icon"
mkdir -p "$icon/AppIcon.iconset"
python3 -I "$here/make-icon.py" "$icon/1024.png"
for s in 16 32 128 256 512; do
    sips -z $s $s "$icon/1024.png" --out "$icon/AppIcon.iconset/icon_${s}x${s}.png" >/dev/null
    d=$((s * 2))
    sips -z $d $d "$icon/1024.png" --out "$icon/AppIcon.iconset/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$icon/AppIcon.iconset" -o "$app/Contents/Resources/AppIcon.icns"
rm -rf "$icon"

codesign --force --sign - "$app"
echo "$app"
