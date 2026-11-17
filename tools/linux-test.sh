#!/usr/bin/env bash
# Builds and tests `ut` on Linux from this checkout, and compares its grep
# with GNU grep where GNU grep is installed.
#
#   wsl -- bash -l tools/linux-test.sh        (from Windows, in the repository)
#   bash tools/linux-test.sh                  (on Linux)
#
# The build goes to a directory on the Linux side (UT_TARGET, default
# ~/.cache/uni-term-target): building in a Windows-mounted checkout would
# mix two systems' files in one target directory, and is slow.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export CARGO_TARGET_DIR="${UT_TARGET:-$HOME/.cache/uni-term-target}"
cd "$root"

echo "--- build and test ut ($(rustc --version), $(uname -srm)) ---"
cargo test -p ut --quiet 2>&1 | grep -E "^test result|FAILED|panicked|cases;|error" || true
cargo test -p ut --quiet -- --nocapture every_command 2>&1 | grep -E "cases;" || true

ut="$CARGO_TARGET_DIR/debug/ut"
echo "--- $("$ut" --version) ---"

if ! command -v grep >/dev/null || ! grep --version 2>/dev/null | grep -q "GNU grep"; then
    echo "GNU grep is not installed: no comparison"
    exit 0
fi

# The same searches with ut's grep and GNU grep, on the same files: every
# byte of output and the exit code must be the same.
work="$(mktemp -d)"
trap 'chmod -R u+rwx "$work" 2>/dev/null; rm -rf "$work"' EXIT
mkdir -p "$work/tree/sub"
printf 'banana\napple\ncherry\napple pie\nBanana\ndate\n' > "$work/words.txt"
printf 'one\ntwo\nthree\nfour\n' > "$work/left.txt"
printf 'alpha\nbeta\n' > "$work/tree/a.txt"
printf 'beta\ngamma\n' > "$work/tree/sub/b.txt"
printf '中文一行\nsecond 行\n' > "$work/cjk.txt"
printf 'alpha\0binary\n' > "$work/bin.dat"
# A directory that cannot be read (as root it can: the check is then left out).
mkdir -p "$work/shut/open" "$work/shut/locked"
printf 'beta\n' > "$work/shut/open/x.txt"
chmod 000 "$work/shut/locked"
cd "$work"

same=0
differ=0
# The same, with what is said on stderr compared too.
both() {
    local ours theirs ours_code theirs_code
    ours="$("$ut" grep "$@" 2>&1 | od -c)" || true
    ours_code=$("$ut" grep "$@" >/dev/null 2>&1; echo $?)
    theirs="$(grep "$@" 2>&1 | od -c)" || true
    theirs_code=$(grep "$@" >/dev/null 2>&1; echo $?)
    if [ "$ours" = "$theirs" ] && [ "$ours_code" = "$theirs_code" ]; then
        same=$((same + 1))
    else
        differ=$((differ + 1))
        echo "DIFFERS (with stderr): grep $* (exit ours $ours_code, GNU $theirs_code)"
        echo "  ours: $("$ut" grep "$@" 2>&1 | head -3 | tr '\n' '|')"
        echo "  GNU:  $(grep "$@" 2>&1 | head -3 | tr '\n' '|')"
    fi
}
# The same, reading what is given as the first argument from standard input.
piped() {
    local input="$1"; shift
    local ours theirs ours_code theirs_code
    ours="$(printf "$input" | "$ut" grep "$@" 2>/dev/null | od -c)" || true
    ours_code=$(printf "$input" | "$ut" grep "$@" >/dev/null 2>&1; echo $?)
    theirs="$(printf "$input" | grep "$@" 2>/dev/null | od -c)" || true
    theirs_code=$(printf "$input" | grep "$@" >/dev/null 2>&1; echo $?)
    if [ "$ours" = "$theirs" ] && [ "$ours_code" = "$theirs_code" ]; then
        same=$((same + 1))
    else
        differ=$((differ + 1))
        echo "DIFFERS (stdin): grep $* (exit ours $ours_code, GNU $theirs_code)"
    fi
}
check() {
    local ours theirs ours_code theirs_code
    ours="$("$ut" grep "$@" 2>/dev/null | od -c)" && ours_code=0 || ours_code=$?
    ours_code=$("$ut" grep "$@" >/dev/null 2>&1; echo $?)
    theirs="$(grep "$@" 2>/dev/null | od -c)" || true
    theirs_code=$(grep "$@" >/dev/null 2>&1; echo $?)
    if [ "$ours" = "$theirs" ] && [ "$ours_code" = "$theirs_code" ]; then
        same=$((same + 1))
    else
        differ=$((differ + 1))
        echo "DIFFERS: grep $* (exit ours $ours_code, GNU $theirs_code)"
        echo "  ours: $("$ut" grep "$@" 2>/dev/null | head -4 | tr '\n' '|')"
        echo "  GNU:  $(grep "$@" 2>/dev/null | head -4 | tr '\n' '|')"
    fi
}
check apple words.txt
check -n three left.txt words.txt
check -c an words.txt
check -i banana words.txt
check -v a words.txt
check -o an words.txt
check -E '^(apple|date)$' words.txt
check 'cherry\|date' words.txt
check 'a\+' words.txt
check 'ap\{2\}le' words.txt
check -w apple words.txt
check -x apple words.txt
check -n -A1 two left.txt
check -n -B1 -A1 three left.txt
check -C1 two left.txt
check -m1 a words.txt
check -b cherry words.txt
check -H apple words.txt
check -h two left.txt words.txt
check -l apple words.txt left.txt
check -L apple words.txt left.txt
check -q apple words.txt
check -s x no-such-file
check zzz words.txt
check -F 'a.' words.txt
check -e date -e cherry words.txt
check --color=always -n an words.txt
check --color=always -o an words.txt
check --color=always -c an words.txt left.txt
check --color=always -l an words.txt left.txt
check --color=always -L an words.txt left.txt
check --color=always -H -c an words.txt
check --color=always -rn beta tree
check --color=always -n -A1 two left.txt
check --color=always -v -n a words.txt
check -Z -l an words.txt left.txt
check -Z -c an words.txt left.txt
check -on a words.txt
check -ob an words.txt
check -ic BANANA words.txt
check -x -n 'apple pie' words.txt
check -vc a words.txt
check -m2 -n a words.txt
check -n -e a -e e left.txt
check -n 行 cjk.txt
check -c '' words.txt
check -rl --include='*.txt' beta tree
check -rc beta tree
check apple tree
check --include='w*' apple words.txt left.txt
check --exclude='*.txt' apple words.txt left.txt
check --exclude='left*' -c e words.txt left.txt
check -r --exclude-dir=sub beta tree
check -rh beta tree
check -rL gamma tree
check -ho an words.txt left.txt
check -cv a words.txt
check -iw BANANA words.txt
check 'a*' left.txt
check '^*' words.txt
check '\(ap\)\+le' words.txt
check '[[:upper:]]' words.txt
check 'b.n' words.txt
check 'e$' words.txt
check '^a' words.txt
check '\<pie\>' words.txt
check -E 'p{2}|rr' words.txt
check -E '(an)+a' words.txt
check -E 'a|' left.txt
check -Fx apple words.txt
check -F -e apple -e date words.txt
check -n -A5 four left.txt
check -n -B5 one left.txt
check -m1 -A1 -n o left.txt
check -m1 -A2 -n o left.txt
check -m2 -A1 -n e words.txt
check -m1 -B1 -A1 -n three left.txt
check -m0 a words.txt
check -m1 -c a words.txt left.txt
check -m1 -o an words.txt
check -n -v -m2 e words.txt
check -c -m1 a words.txt
check -o -i 'b[a-z]*' words.txt
check -n -e '' left.txt
check -x '' words.txt
check -w 'pie' words.txt
check -b -o an words.txt
check -lv zzz words.txt left.txt
check -L -v a words.txt left.txt
check -sq apple no-such-file words.txt
check -H -c x no-such-file words.txt
both -n two no-such-file left.txt
both apple tree
both -r alpha no-such-dir
both alpha bin.dat
both -c alpha bin.dat
both -a alpha bin.dat
both -I alpha bin.dat words.txt
if [ "$(id -u)" -ne 0 ]; then both -r beta shut; fi
piped 'a\nb\nab\n' -c a
piped 'a\nb\nab\n' -n b
piped 'x\n' -H x
piped 'one\ntwo\n' -n two - left.txt
piped '' x
echo "grep against GNU grep $(grep --version | head -1 | awk '{print $NF}'): $same the same, $differ differ"
[ "$differ" -eq 0 ]
