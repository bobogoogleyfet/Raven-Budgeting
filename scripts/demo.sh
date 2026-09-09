#!/usr/bin/env bash
#
# Build a throwaway demo database and open the app on it.
#
# Everything here goes through `pacc`, the same CLI a user has, so the demo is
# the real core doing real arithmetic — the only fiction is that the receipts
# never happened. Nothing touches the database the app normally opens: the demo
# lives under a path of its own and is rebuilt from nothing on every run.
#
#   scripts/demo.sh                   build, then run on the Linux desktop
#   scripts/demo.sh --device android  build, then push to a connected phone
#   scripts/demo.sh --build-only      just make the database
#   scripts/demo.sh --today 2026-08-20  pretend it is some other day
#
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Outside the repository on purpose. This is regenerated on every run and is
# nobody's working copy, and `.gitignore` has no rule that would stop a stray
# `pacc.db` being committed.
#
# Named for the user, because /tmp is shared: a bare /tmp/pacc-demo is a
# fixed path any account on the machine can create first, and then this script
# either cannot write to it or writes a demo into someone else's directory.
DEMO_DIR="${PACC_DEMO_DIR:-${TMPDIR:-/tmp}/pacc-demo-$(id -u)}"
DEMO_DB="$DEMO_DIR/pacc.db"

ANDROID_PKG=com.tannerhathaway.app

DEVICE=linux
BUILD_ONLY=0
TODAY="$(date +%F)"
while [ $# -gt 0 ]; do
  case "$1" in
    --device) DEVICE="$2"; shift 2 ;;
    --build-only) BUILD_ONLY=1; shift ;;
    --today) TODAY="$2"; shift 2 ;;
    -h|--help) sed -n '3,13p' "${BASH_SOURCE[0]}" | sed 's/^# \?//'; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

MONTH="${TODAY%-*}"        # YYYY-MM
DAY="${TODAY##*-}"; DAY=$((10#$DAY))

# The release binary. Debug `pacc` is fine for a handful of commands and far
# too slow for the few hundred this makes.
PACC="$REPO/target/release/pacc"
if [ ! -x "$PACC" ]; then
  echo "==> building pacc"
  cargo build --manifest-path "$REPO/Cargo.toml" -p accounting-cli --release
fi
pacc() { "$PACC" --db "$DEMO_DB" "$@"; }

# ---------------------------------------------------------------------------
# Deterministic variation
#
# The amounts wander so the screens do not read as generated, but they wander
# the same way every run: a demo that shows different figures each time cannot
# be rehearsed, and a screenshot of it cannot be checked against anything.
# ---------------------------------------------------------------------------
SEED=20260909
rnd() { SEED=$(( (SEED * 1103515245 + 12345) % 2147483648 )); echo $(( SEED % $1 )); }
# An amount in dollars.cents, uniform in [$lo, $hi].
amount() { local lo=$1 hi=$2 c; c=$(( lo * 100 + $(rnd $(( (hi - lo) * 100 + 1 )) ) )); printf '%d.%02d' $(( c / 100 )) $(( c % 100 )); }
# One of the remaining arguments, chosen deterministically.
pick() { local n=$#; eval echo "\${$(( $(rnd "$n") + 1 ))}"; }

# The months with history, oldest first, ending at the current one.
months() {
  local y=${MONTH%-*} m=$((10#${MONTH#*-})) i out=()
  for i in 7 6 5 4 3 2 1 0; do
    local mm=$(( m - i )) yy=$y
    while [ $mm -le 0 ]; do mm=$(( mm + 12 )); yy=$(( yy - 1 )); done
    out+=("$(printf '%04d-%02d' "$yy" "$mm")")
  done
  echo "${out[@]}"
}
MONTHS=($(months))
FIRST="${MONTHS[0]}"

echo "==> building a demo database at $DEMO_DB"
# Say which of the two it is. "Permission denied" from `rm` on a path the
# script chose itself reads as a bug in the script rather than as a directory
# somebody else owns.
if [ -e "$DEMO_DIR" ] && ! rm -rf "$DEMO_DIR" 2>/dev/null; then
  echo "cannot clear $DEMO_DIR — it exists and is not yours ($(stat -c '%U' "$DEMO_DIR"))." >&2
  echo "remove it, or set PACC_DEMO_DIR to somewhere you own." >&2
  exit 1
fi
mkdir -p "$DEMO_DIR"
pacc init >/dev/null

# A published Utah jurisdiction rather than a flat percentage. Adopting one
# brings the reduced grocery rate and the restaurant rate with it, which is
# half of what this app is for — and it is what clears the app's first-run
# form, which fires on a home location still sitting at 0%.
pacc location --name "Salt Lake City" --home >/dev/null

# ---------------------------------------------------------------------------
# Budgets
#
# Started before the history so that every month of it has a line in force —
# `budget contribute` refuses a month with no line to default its amount from.
# Anything that is not `monthly` is a sinking fund to the budget screen, which
# is what puts the "Set aside" button on the row.
# ---------------------------------------------------------------------------
echo "==> setting budgets"
set_monthly() { pacc budget set --category "$1" --amount "$2" --start "$FIRST" >/dev/null; }
set_yearly()  { pacc budget set --category "$1" --amount "$2" --period yearly --start "$FIRST" >/dev/null; }

set_monthly "Bills/Rent"              1650
set_monthly "Bills/Electric"           135
set_monthly "Bills/Internet"            70
set_monthly "Bills/Phone"               95
set_monthly "Bills/Health Insurance"   485
set_monthly "Bills/Subscriptions"       48
set_monthly "Frequent/Groceries"       650
set_monthly "Frequent/Restaurants"     180
set_monthly "Frequent/Gas"             160
set_monthly "Frequent/Household"        90
set_monthly "Frequent/Toiletries"       45
set_monthly "Quality of Life/Entertainment" 60
set_monthly "Quality of Life/Hobbies"   55
set_monthly "Quality of Life/Fitness"   45

set_yearly  "Bills/Car Insurance"     1320
set_yearly  "Occasional/Car Repair"    900
set_yearly  "Occasional/Medical"       750
set_yearly  "Occasional/Travel"       2400
set_yearly  "Occasional/Gifts"         600
set_yearly  "Occasional/Clothing"      540
set_yearly  "Savings/Emergency Fund"  3600

# ---------------------------------------------------------------------------
# History
# ---------------------------------------------------------------------------
echo "==> recording receipts"
add() { pacc add "$@" >/dev/null; }
d() { printf '%s-%02d' "$1" "$2"; }   # a date in month $1, day $2

TXNS=0
for M in "${MONTHS[@]}"; do
  CURRENT=0; [ "$M" = "$MONTH" ] && CURRENT=1
  # The current month is only as far through as today, and February is only as
  # far as February goes. Everything below asks `upto` before it spends, so the
  # newest month reads as a month in progress rather than one that
  # mysteriously stopped, and no month invents a day it does not have.
  LAST=$(date -d "${M}-01 +1 month -1 day" +%-d)
  [ $CURRENT -eq 1 ] && LAST=$DAY
  upto() { [ "$1" -le "$LAST" ]; }

  # Bills. Fixed days, no sales tax — the Bills group is billed, not bought.
  # Rent is deliberately the one that has not gone out yet in the current
  # month: an unpaid bill is what "behind on funding" is for, and a budget
  # screen where nothing is behind never shows that column doing its job.
  upto 2 && add --date "$(d "$M" 2)" --merchant "SelectHealth"          --category "Bills/Health Insurance" --total 485.00
  upto 3 && add --date "$(d "$M" 3)" --merchant "Rocky Mountain Power" --category "Bills/Electric" --total "$(amount 96 134)"
  upto 5 && add --date "$(d "$M" 5)" --merchant "Google Fiber"         --category "Bills/Internet" --total 70.00
  upto 7 && add --date "$(d "$M" 7)" --merchant "Verizon"              --category "Bills/Phone" --total 95.00
  upto 8 && add --date "$(d "$M" 8)" --merchant "Subscriptions"        --category "Bills/Subscriptions" --total "$(amount 39 48)"
  if [ $CURRENT -eq 0 ]; then
    add --date "$(d "$M" 1)" --merchant "Ivory Ridge Apartments" --category "Bills/Rent" --total 1650.00
    TXNS=$((TXNS+1))
  fi
  TXNS=$((TXNS+5))

  # Groceries, roughly weekly.
  for DAYN in 1 5 9 14 20 26; do
    upto $DAYN || continue
    add --date "$(d "$M" $DAYN)" --merchant "$(pick Harmons "Smith's" "Macey's" "WinCo Foods" Costco)" \
        --category "Frequent/Groceries" --total "$(amount 74 168)"
    TXNS=$((TXNS+1))
  done

  # Restaurants. In the current month this is the line that goes over, which
  # is the one row on the budget screen that has to look wrong at a glance.
  for DAYN in 2 4 6 8 13 18 23 27; do
    upto $DAYN || continue
    if [ $CURRENT -eq 1 ]; then LO=48; HI=76; else LO=11; HI=34; fi
    add --date "$(d "$M" $DAYN)" --merchant "$(pick "Cafe Rio" "Costa Vida" "Chick-fil-A" "In-N-Out Burger" Swig)" \
        --category "Frequent/Restaurants" --total "$(amount $LO $HI)"
    TXNS=$((TXNS+1))
  done

  # Fuel.
  for DAYN in 3 12 21 28; do
    upto $DAYN || continue
    add --date "$(d "$M" $DAYN)" --merchant "$(pick Maverik Chevron Sinclair "Holiday Oil")" \
        --category "Frequent/Gas" --total "$(amount 41 68)"
    TXNS=$((TXNS+1))
  done

  # The rest of the everyday.
  upto 6  && { add --date "$(d "$M" 6)"  --merchant "$(pick Target Walmart)" --category "Frequent/Household" --total "$(amount 18 74)"; TXNS=$((TXNS+1)); }
  upto 9  && { add --date "$(d "$M" 9)"  --merchant "$(pick Walgreens CVS)"  --category "Frequent/Toiletries" --total "$(amount 12 38)"; TXNS=$((TXNS+1)); }
  upto 4  && { add --date "$(d "$M" 4)"  --merchant "VASA Fitness"           --category "Quality of Life/Fitness" --total 45.00; TXNS=$((TXNS+1)); }
  upto 15 && { add --date "$(d "$M" 15)" --merchant "$(pick Amazon eBay)"    --category "Quality of Life/Hobbies" --total "$(amount 14 62)"; TXNS=$((TXNS+1)); }
  upto 24 && { add --date "$(d "$M" 24)" --merchant "Megaplex Theatres"      --category "Quality of Life/Entertainment" --total "$(amount 16 58)"; TXNS=$((TXNS+1)); }
done

# The occasional things, which is what the sinking funds are for. Spread over
# the history rather than sprinkled monthly: a fund that is drawn on every
# month is just a budget, and the screen would have nothing to say about it.
occasional() { add --date "$1" --merchant "$2" --category "$3" --total "$4"; TXNS=$((TXNS+1)); }
occasional "$(d "${MONTHS[1]}" 14)" "Larry H. Miller"    "Occasional/Car Repair" 412.88
occasional "$(d "${MONTHS[2]}" 22)" "Nordstrom Rack"     "Occasional/Clothing"   186.40
occasional "$(d "${MONTHS[3]}" 8)"  "Intermountain Health" "Occasional/Medical"  240.00
occasional "$(d "${MONTHS[4]}" 3)"  "Delta Air Lines"    "Occasional/Travel"     684.20
occasional "$(d "${MONTHS[4]}" 5)"  "Hyatt Place"        "Occasional/Travel"     512.75
occasional "$(d "${MONTHS[5]}" 19)" "Amazon"             "Occasional/Gifts"      124.99
occasional "$(d "${MONTHS[6]}" 11)" "Les Schwab"         "Occasional/Car Repair" 268.50

# One receipt in full detail: a pre-tax subtotal the core turns into a total,
# split across two categories with the tax apportioned between them, and the
# groceries half itemized down to the eggs. Reconciling that is the arithmetic
# the app exists to get right, and it is the screen worth opening on camera.
add --date "$(d "${MONTHS[6]}" 16)" --merchant "Costco" \
    --subtotal 117.40 \
    --split "Frequent/Groceries=75.42" --split "Frequent/Household=remainder" \
    --item "1:Rotisserie chicken=4.99" --item "1:Eggs, 24ct=6.49" \
    --item "1:Olive oil, 2L=19.99" --item "1:Ground beef, 4lb=22.47" \
    --item "1:Coffee, 2lb=12.49" --item "1:Blueberries, 2lb=8.99" >/dev/null
TXNS=$((TXNS+1))

# ---------------------------------------------------------------------------
# Sinking funds
#
# Every month of the history except the current one, which is what leaves the
# budget screen with a live "Set aside $110.00" button to press on camera.
# ---------------------------------------------------------------------------
echo "==> funding the sinking funds"
FUNDS=("Bills/Car Insurance" "Occasional/Car Repair" "Occasional/Medical"
       "Occasional/Travel" "Occasional/Gifts" "Occasional/Clothing"
       "Savings/Emergency Fund")
# Left waiting in the current month, so the budget screen opens with something
# to press. Three is a demonstration; all seven is a chore, and a screen where
# every row wants something is one nobody reads a single row of.
PENDING=("Bills/Car Insurance" "Occasional/Travel" "Savings/Emergency Fund")
pending() { local c; for c in "${PENDING[@]}"; do [ "$c" = "$1" ] && return 0; done; return 1; }

for M in "${MONTHS[@]}"; do
  for CAT in "${FUNDS[@]}"; do
    [ "$M" = "$MONTH" ] && pending "$CAT" && continue
    pacc budget contribute --category "$CAT" --month "$M" >/dev/null
  done
done

# ---------------------------------------------------------------------------
# The inbox
#
# A demo where every screen is already finished has nothing to demonstrate.
# These are totals recorded without a category — the "I will deal with it
# later" path — so the Inbox opens with real work in it.
# ---------------------------------------------------------------------------
echo "==> leaving a few things to categorize"
inbox() { pacc quick-add "$2" --merchant "$1" --date "$3" >/dev/null; TXNS=$((TXNS+1)); }
[ $DAY -ge 2 ] && inbox "Harmons"      47.31 "$(d "$MONTH" 2)"
[ $DAY -ge 5 ] && inbox "Maverik"      52.08 "$(d "$MONTH" 5)"
[ $DAY -ge 6 ] && inbox "Home Depot"   88.64 "$(d "$MONTH" 6)"
[ $DAY -ge 8 ] && inbox "Swig"          9.45 "$(d "$MONTH" 8)"

echo
echo "    $TXNS transactions across ${#MONTHS[@]} months, $FIRST to $MONTH"
echo
echo "    Budget      Restaurants is over; Rent is behind, because it is not paid yet"
echo "    Budget      three funds are waiting on this month's set-aside"
echo "    Inbox       4 things to categorize"
echo "    Expenses    $TXNS receipts, one of them itemized across two splits"
echo

[ $BUILD_ONLY -eq 1 ] && { echo "==> built. PACC_DB=$DEMO_DB"; exit 0; }

case "$DEVICE" in
  linux)
    echo "==> running on the Linux desktop"
    # The desktop resolves its path from the same $PACC_DB the CLI reads, so
    # pointing the app at the demo is one variable and no code.
    cd "$REPO/app" && PACC_DB="$DEMO_DB" exec flutter run -d linux --dart-define=PACC_DEMO=true
    ;;
  android)
    # Mobile ignores $PACC_DB — it asks Android for its own support directory
    # (main.dart:35) — so the only way in is to put the file where the app
    # will look. `run-as` is what makes that legal on a debug build.
    command -v adb >/dev/null || { echo "adb is not on PATH" >&2; exit 1; }
    SERIAL="$(adb devices | awk '$2=="device"{print $1; exit}')"
    [ -n "$SERIAL" ] || { echo "no authorized android device; check the phone for the USB dialog" >&2; exit 1; }
    echo "==> installing the app on $SERIAL"
    ( cd "$REPO/app" && flutter build apk --debug --dart-define=PACC_DEMO=true )
    adb -s "$SERIAL" install -r "$REPO/app/build/app/outputs/flutter-apk/app-debug.apk"

    # Ship one checkpointed file. `backup` is VACUUM INTO, which folds the WAL
    # in — copying pacc.db alone would land a database missing its most recent
    # commits, and leaving the old -wal beside a new file is how you get
    # "database disk image is malformed".
    echo "==> pushing the demo database"
    rm -f "$DEMO_DIR/push.db"
    pacc backup --to "$DEMO_DIR/push.db" >/dev/null
    adb -s "$SERIAL" shell am force-stop "$ANDROID_PKG"
    adb -s "$SERIAL" push "$DEMO_DIR/push.db" /data/local/tmp/pacc.db >/dev/null
    adb -s "$SERIAL" shell chmod 666 /data/local/tmp/pacc.db
    # One quoted string, because `adb shell` joins its arguments and the
    # device's shell splits them again — an inner `sh -c '...'` passed as
    # separate words arrives as separate words, and the copy silently does not
    # happen while every command still reports success.
    #
    # Deleting the old -wal and -shm is load-bearing: a write-ahead log left
    # beside a database it does not belong to is how you get "database disk
    # image is malformed" on launch.
    adb -s "$SERIAL" shell "run-as $ANDROID_PKG sh -c 'rm -f files/pacc.db files/pacc.db-wal files/pacc.db-shm && cp /data/local/tmp/pacc.db files/pacc.db && chmod 600 files/pacc.db'" 
    adb -s "$SERIAL" shell rm -f /data/local/tmp/pacc.db

    # adb exits 0 whatever happened on the far end, so ask the device.
    LANDED="$(adb -s "$SERIAL" shell "run-as $ANDROID_PKG sh -c 'wc -c < files/pacc.db'" | tr -d '\r\n ')"
    EXPECTED="$(wc -c < "$DEMO_DIR/push.db" | tr -d ' ')"
    [ "$LANDED" = "$EXPECTED" ] || {
      echo "the database did not land: device has ${LANDED:-nothing}, expected $EXPECTED bytes" >&2
      exit 1
    }

    adb -s "$SERIAL" shell monkey -p "$ANDROID_PKG" -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1
    echo "==> running on $SERIAL"
    ;;
  *) echo "unknown device: $DEVICE (linux, android)" >&2; exit 2 ;;
esac
