#!/bin/sh
#
# Give a PACC_DEMO build the demo's bundle id and name, on macOS and iOS.
#
# Run by Xcode as the last build phase of Runner in both ios/ and macos/,
# after the Info.plist has been written and before the app is signed.
#
# The demo is a separate app, not a mode of this one. Same bundle id means the
# demo installs over the real app and — on iOS — opens the real database,
# because a mobile build keeps it in the per-app directory, and that directory
# belongs to whichever app has the id. So a demo build takes an id of its own,
# com.tannerhathaway.app.demo, and calls itself "Raven Demo".
#
# Keyed on the same --dart-define=PACC_DEMO=true the Dart side reads, so no
# build can be a demo to Dart and the real app to the system. Xcode never sees
# a --dart-define as such: the flutter tool writes them into Generated.xcconfig
# as one DART_DEFINES build setting, a comma-separated list with each
# KEY=VALUE base64-encoded on its own. An xcconfig has no way to look inside
# that list, which is why this is a script rather than a line in
# AppInfo.xcconfig, and why it edits the built Info.plist rather than the
# PRODUCT_BUNDLE_IDENTIFIER setting: build settings are fixed before any build
# phase runs. Flutter's own embed_and_thin phase edits the built Info.plist
# the same way.
#
# That is enough on macOS, where the app is signed to run locally and the
# signature takes its identifier from the Info.plist. It is not enough on iOS,
# where the provisioning profile is chosen from PRODUCT_BUNDLE_IDENTIFIER and
# `flutter run` launches whatever that setting names — so a demo built under
# the real id would install, then open the real app. There this refuses rather
# than rewrites, and the id has to come in as a build setting:
#
#   FLUTTER_XCODE_PRODUCT_BUNDLE_IDENTIFIER=com.tannerhathaway.app.demo \
#     flutter run -d <ios device> --dart-define=PACC_DEMO=true
#
# (the flutter tool passes any FLUTTER_XCODE_* variable to xcodebuild as a
# build setting, including when it asks Xcode which id to launch).
set -eu

REAL_NAME="Raven Budgeting"
DEMO_NAME="Raven Demo"
PACC_DEMO_ENCODED="UEFDQ19ERU1PPXRydWU="   # base64 of PACC_DEMO=true

plist="${TARGET_BUILD_DIR}/${INFOPLIST_PATH}"
configured="${PRODUCT_BUNDLE_IDENTIFIER}"

demo=0
old_ifs=$IFS
IFS=,
for define in ${DART_DEFINES:-}; do
  if [ "$define" = "$PACC_DEMO_ENCODED" ]; then demo=1; fi
done
IFS=$old_ifs

if [ $demo -eq 1 ]; then
  id="${configured%.demo}.demo"
  name="$DEMO_NAME"
else
  case "$configured" in
    *.demo)
      echo "error: $configured is the demo's bundle id, but this build was not given --dart-define=PACC_DEMO=true." >&2
      echo "error: it would be the real app to Dart and the demo to the system. Unset FLUTTER_XCODE_PRODUCT_BUNDLE_IDENTIFIER, or pass the flag." >&2
      exit 1
      ;;
  esac
  id="$configured"
  name="$REAL_NAME"
fi

if [ "$id" != "$configured" ] && [ "${PLATFORM_NAME:-}" != "macosx" ]; then
  echo "error: this is a PACC_DEMO build, but Xcode is building it as $configured, the real app's id." >&2
  echo "error: it would be signed, installed and launched as the real app, and open the real app's data. Build it with" >&2
  echo "error:   FLUTTER_XCODE_PRODUCT_BUNDLE_IDENTIFIER=$id flutter ... --dart-define=PACC_DEMO=true" >&2
  exit 1
fi

if [ ! -f "$plist" ]; then
  # A real build with no Info.plist yet will get one written from source,
  # which already says the right thing. A demo build must not go on to be
  # signed as whatever that turns out to be.
  if [ $demo -eq 1 ]; then
    echo "error: $plist does not exist, so the demo cannot be given its own bundle id. Build again." >&2
    exit 1
  fi
  exit 0
fi

# Written in both directions, every build. Xcode does not always rewrite an
# Info.plist whose sources did not change, so a real build straight after a
# demo one would otherwise be signed with the demo's id still in it.
plutil -replace CFBundleIdentifier -string "$id" "$plist"
plutil -replace CFBundleName -string "$name" "$plist"
plutil -replace CFBundleDisplayName -string "$name" "$plist"
