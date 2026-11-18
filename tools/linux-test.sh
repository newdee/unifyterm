#!/usr/bin/env bash
# Builds and tests `ut` on Linux from this checkout, and compares its grep
# with GNU grep where GNU grep is installed.
#
#   wsl -- bash -l tools/linux-test.sh        (from Windows, in the repository)
#   bash tools/linux-test.sh                  (on Linux)
#
# The build goes to a directory on the Linux side (UT_TARGET, default
# ~/.cache/unifyterm-target): building in a Windows-mounted checkout would
# mix two systems' files in one target directory, and is slow.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export CARGO_TARGET_DIR="${UT_TARGET:-$HOME/.cache/unifyterm-target}"
cd "$root"

echo "--- build and test ut ($(rustc --version), $(uname -srm)) ---"
cargo test -p ut --quiet 2>&1 | grep -E "^test result|FAILED|panicked|cases;|error" || true
cargo test -p ut --quiet -- --nocapture every_command 2>&1 | grep -E "cases;" || true

ut="$CARGO_TARGET_DIR/debug/ut"
echo "--- $("$ut" --version) ---"

# The shells' hooks (crates/unifyterm/src/hooks): each shell, run interactively in
# a pty by `script` with a clean environment (no user start files; PATH without
# the Windows directories WSL adds, whose spaces would break the command line),
# types two commands; the marks it prints are checked.
hooks="$root/crates/unifyterm/src/hooks"
marks_ok=0
marks_missing=0
expect_marks() { # shell name, output file
    local shell=$1 out=$2 missing=""
    for mark in "633;P;Shell=$shell" "133;A" "133;B" "633;E;echo hook-one" "133;D;0" "633;E;false" "133;D;1" "633;P;Cwd=$work_hooks"; do
        grep -qF "$mark" "$out" || missing="$missing [$mark]"
    done
    if [ -z "$missing" ]; then
        marks_ok=$((marks_ok + 1)); echo "$shell: all 8 marks"
    else
        marks_missing=$((marks_missing + 1)); echo "MISSING ($shell):$missing"
    fi
}
work_hooks="$(mktemp -d)"
lines='echo hook-one\nfalse\nexit\n'
( cd "$work_hooks" && printf "$lines" | script -qfec "env -i HOME=$work_hooks TERM=xterm PATH=/usr/local/bin:/usr/bin:/bin bash --rcfile $hooks/bashrc -i" /dev/null > "$work_hooks/bash.out" 2>&1 ) || true
expect_marks bash "$work_hooks/bash.out"
if command -v zsh >/dev/null; then
    mkdir -p "$work_hooks/zdot"
    cp "$hooks/zshenv" "$work_hooks/zdot/.zshenv"; cp "$hooks/zprofile" "$work_hooks/zdot/.zprofile"; cp "$hooks/zshrc" "$work_hooks/zdot/.zshrc"
    ( cd "$work_hooks" && printf "$lines" | script -qfec "env -i HOME=$work_hooks TERM=xterm PATH=/usr/local/bin:/usr/bin:/bin UNIFYTERM_ZDOTDIR=$work_hooks ZDOTDIR=$work_hooks/zdot zsh -i" /dev/null > "$work_hooks/zsh.out" 2>&1 ) || true
    expect_marks zsh "$work_hooks/zsh.out"
else
    echo "zsh is not installed: not checked"
fi
if command -v fish >/dev/null; then
    ( cd "$work_hooks" && printf "$lines" | script -qfec "env -i HOME=$work_hooks TERM=xterm PATH=/usr/local/bin:/usr/bin:/bin fish --init-command 'source $hooks/hook.fish' -i" /dev/null > "$work_hooks/fish.out" 2>&1 ) || true
    expect_marks fish "$work_hooks/fish.out"
else
    echo "fish is not installed: not checked"
fi
echo "shell hooks: $marks_ok complete, $marks_missing missing marks"
rm -rf "$work_hooks"

# The start files: a home with profile and rc files that each put a directory
# of their own first in PATH. Started the way unifyterm starts a tab's shell
# (plain, and as a login shell, which is what it does on macOS), each shell
# says which files it read and what is first in PATH: unifyterm's commands.
# (printf '%b': the lines hold a %, which is not a format here.)
starts_ok=0
starts_wrong=0
home="$(mktemp -d)"
mkdir -p "$home/zdot" "$home/.config/fish"
cp "$hooks/zshenv" "$home/zdot/.zshenv"; cp "$hooks/zprofile" "$home/zdot/.zprofile"; cp "$hooks/zshrc" "$home/zdot/.zshrc"
mkdir -p "$home/profile-bin"
printf '#!/bin/sh\necho "[]"\n' > "$home/profile-bin/fake-agent"; chmod +x "$home/profile-bin/fake-agent"
printf 'export FROM_PROFILE=profile\nexport PATH=%s/profile-bin:$PATH\n' "$home" | tee "$home/.zprofile" > "$home/.bash_profile"
printf 'export FROM_RC=rc\nexport PATH=/opt/rc-bin:$PATH\n' | tee "$home/.zshrc" > "$home/.bashrc"
printf 'status is-login; and set -gx FROM_PROFILE profile\nstatus is-login; and set -gx PATH %s/profile-bin $PATH\nset -gx FROM_RC rc\nset -gx PATH /opt/rc-bin $PATH\n' "$home" > "$home/.config/fish/config.fish"
clean="env -i HOME=$home TERM=xterm PATH=/data/ut-bin:/usr/local/bin:/usr/bin:/bin UNIFYTERM_COMMANDS=/data/ut-bin"
say='echo "first=${PATH%%:*} profile=${FROM_PROFILE:-none} rc=${FROM_RC:-none} login=${UNIFYTERM_LOGIN:-unset}"\nexit\n'
say_fish='echo "first=$PATH[1] profile="(set -q FROM_PROFILE; and echo $FROM_PROFILE; or echo none)" rc=$FROM_RC login=unset"\nexit\n'
expect_start() { # what it is, the line it must print, the output file
    if grep -qF "$2" "$3"; then
        starts_ok=$((starts_ok + 1))
    else
        starts_wrong=$((starts_wrong + 1)); echo "WRONG ($1): wanted [$2], got [$(grep -a 'first=/' "$3" | tail -n 1 | tr -d '\r')]"
    fi
}
( cd "$home" && printf '%b' "$say" | script -qfec "$clean bash --rcfile $hooks/bashrc -i" /dev/null > "$home/bash-plain.out" 2>&1 ) || true
expect_start "bash, plain" "first=/data/ut-bin profile=none rc=rc login=unset" "$home/bash-plain.out"
( cd "$home" && printf '%b' "$say" | script -qfec "$clean UNIFYTERM_LOGIN=1 bash --rcfile $hooks/bashrc -i" /dev/null > "$home/bash-login.out" 2>&1 ) || true
expect_start "bash, login" "first=/data/ut-bin profile=profile rc=none login=unset" "$home/bash-login.out"
if command -v zsh >/dev/null; then
    ( cd "$home" && printf '%b' "$say" | script -qfec "$clean UNIFYTERM_ZDOTDIR=$home ZDOTDIR=$home/zdot zsh -i" /dev/null > "$home/zsh-plain.out" 2>&1 ) || true
    expect_start "zsh, plain" "first=/data/ut-bin profile=none rc=rc login=unset" "$home/zsh-plain.out"
    ( cd "$home" && printf '%b' "$say" | script -qfec "$clean UNIFYTERM_ZDOTDIR=$home ZDOTDIR=$home/zdot zsh -l -i" /dev/null > "$home/zsh-login.out" 2>&1 ) || true
    expect_start "zsh, login" "first=/data/ut-bin profile=profile rc=rc login=unset" "$home/zsh-login.out"
fi
if command -v fish >/dev/null; then
    ( cd "$home" && printf '%b' "$say_fish" | script -qfec "$clean fish --init-command 'source $hooks/hook.fish' -i" /dev/null > "$home/fish-plain.out" 2>&1 ) || true
    expect_start "fish, plain" "first=/data/ut-bin profile=none rc=rc login=unset" "$home/fish-plain.out"
    ( cd "$home" && printf '%b' "$say_fish" | script -qfec "$clean fish -l --init-command 'source $hooks/hook.fish' -i" /dev/null > "$home/fish-login.out" 2>&1 ) || true
    expect_start "fish, login" "first=/data/ut-bin profile=profile rc=rc login=unset" "$home/fish-login.out"
fi
# What unifyterm asks a login shell when a program (keepane, for its sidebar)
# is not in its own PATH: the shell finds it where the profile put it.
for shell in bash zsh fish; do
    command -v "$shell" >/dev/null || continue
    got=$(env -i HOME="$home" PATH=/usr/local/bin:/usr/bin:/bin "$shell" -l -c 'command -v fake-agent' 2>/dev/null | grep '^/' | tail -n 1 || true)
    if [ "$got" = "$home/profile-bin/fake-agent" ]; then
        starts_ok=$((starts_ok + 1))
    else
        starts_wrong=$((starts_wrong + 1)); echo "WRONG ($shell -l -c 'command -v fake-agent'): got [$got]"
    fi
done
echo "start files: $starts_ok as expected, $starts_wrong wrong"
rm -rf "$home"

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
[ "$differ" -eq 0 ] && [ "$marks_missing" -eq 0 ]
