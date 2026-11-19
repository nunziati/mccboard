#!/bin/sh
# Tests install.sh with no network and no build of mccboard.
#
#   sh scripts/test-install.sh [SHELL...]
#
# It runs every case once with each SHELL on the PATH (default: sh). CI runs it with dash and
# bash. The test makes its releases in a temporary folder. Their archives have the layout of
# the ones that .github/workflows/release.yml publishes, with a shell script in place of mccboard
# that prints `mccboard <version>`. install.sh downloads them through
# MCCBOARD_DOWNLOAD_URL=file://... or, for the lookup of the latest release, through a stand-in
# curl that answers for GitHub. A stand-in uname reports the platform each case needs.

set -u

HERE=$(cd "$(dirname "$0")" && pwd)
INSTALL_SH=$(cd "$HERE/.." && pwd)/install.sh
ORIGINAL_PATH=$PATH
REAL_CURL=$(command -v curl) || {
    echo "test-install.sh: the tests need curl to serve the file:// URLs." >&2
    exit 1
}

if [ $# -eq 0 ]; then
    set -- sh
fi

failures=0
checks=0

fail() {
    failures=$((failures + 1))
    printf 'FAIL [%s] %s: %s\n' "$SH" "$CASE" "$1"
    if [ -s "$OUT" ]; then
        sed 's/^/    out | /' "$OUT"
    fi
    if [ -s "$ERR" ]; then
        sed 's/^/    err | /' "$ERR"
    fi
}

pass() {
    checks=$((checks + 1))
}

# Expects the last run to have ended with status $1, or with any non-zero status for "fail".
expect_status() {
    if [ "$1" = fail ] && [ "$STATUS" -ne 0 ]; then
        pass
    elif [ "$1" = "$STATUS" ]; then
        pass
    else
        fail "status $STATUS, expected $1"
    fi
}

# Expects the file $1 (OUT or ERR of the last run) to contain the text $2.
expect_in() {
    if grep -F -q -e "$2" "$1"; then
        pass
    else
        fail "$(basename "$1") lacks: $2"
    fi
}

expect_not_in() {
    if grep -F -q -e "$2" "$1"; then
        fail "$(basename "$1") has: $2"
    else
        pass
    fi
}

# Expects the mccboard installed in $1 to report version $2.
expect_installed() {
    reported=$("$1/mccboard" --version 2>/dev/null) || reported="(none)"
    if [ "$reported" = "mccboard $2" ]; then
        pass
    else
        fail "the mccboard in $1 reports '$reported', expected 'mccboard $2'"
    fi
}

# Expects no staged binary left in $1 and an empty temporary folder.
expect_clean() {
    leftovers=$(find "$1" "$ROOT/tmp" -name '.mccboard.new.*' -o -name 'mccboard-install.*')
    if [ -z "$leftovers" ]; then
        pass
    else
        fail "left behind: $leftovers"
    fi
}

expect_same_file() {
    if cmp -s "$1" "$2"; then
        pass
    else
        fail "$2 changed"
    fi
}

# Every run gets a home folder with a space in its name, the install folder inside it and off the
# PATH, and the stand-in uname first on the PATH. Leading NAME=value arguments of run and
# run_at_terminal change this environment. install.sh gets the arguments after them.
setup_env() {
    unset MCCBOARD_VERSION MCCBOARD_DOWNLOAD_URL FAKE_LATEST
    export HOME="$HOME_DIR" TMPDIR="$ROOT/tmp" MCCBOARD_INSTALL_DIR="$BIN"
    export PATH="$ROOT/fake-bin:$ORIGINAL_PATH" SHELL=/bin/sh
    export SETUP_LOG="$ROOT/setup.log" FAKE_RELEASES="$ROOT/releases" REAL_CURL
}

# Runs install.sh as `sh install.sh ARGS`, without a terminal. The exit status goes in STATUS,
# stdout in OUT and stderr in ERR.
run() {
    rm -f "$ROOT/setup.log"
    (
        setup_env
        while [ $# -gt 0 ]; do
            case $1 in
                *=*)
                    export "${1?}"
                    shift
                    ;;
                *) break ;;
            esac
        done
        cd "$ROOT" && exec "$SH" "$INSTALL_SH" "$@"
    ) >"$OUT" 2>"$ERR" </dev/null
    STATUS=$?
}

# Runs install.sh as `curl ... | sh -s -- ARGS` would, at a terminal that script(1) provides.
# The terminal merges stdout and stderr into OUT.
run_at_terminal() {
    rm -f "$ROOT/setup.log"
    (
        setup_env
        while [ $# -gt 0 ]; do
            case $1 in
                *=*)
                    export "${1?}"
                    shift
                    ;;
                *) break ;;
            esac
        done
        printf '%s\n' "cat '$INSTALL_SH' | $SH -s -- $*" >"$ROOT/at-terminal.sh"
        cd "$ROOT" && exec script -qec "sh '$ROOT/at-terminal.sh'" /dev/null
    ) >"$OUT" 2>"$ERR" </dev/null
    STATUS=$?
}

# Makes the release $1 in $ROOT/releases/v$1, for both targets. $2 says whether its mccboard
# has the setup command (yes or no). When $3 is bad-checksum, the .sha256 files are wrong.
make_release() {
    folder=$ROOT/releases/v$1
    mkdir -p "$folder" "$ROOT/staging"
    if [ "$2" = yes ]; then
        # This text is code of the stand-in. Its variables expand only when the stand-in runs.
        # shellcheck disable=SC2016
        setup_case='setup) [ "${2:-}" = --help ] && exit 0
        if [ -t 0 ]; then echo terminal >"$SETUP_LOG"; else echo no-terminal >"$SETUP_LOG"; fi
        echo "stand-in setup ran" ;;'
    else
        setup_case=''
    fi
    cat >"$ROOT/staging/mccboard" <<EOF
#!/bin/sh
# A stand-in for mccboard $1, made by scripts/test-install.sh.
case "\${1:-}" in
    --version) echo "mccboard $1" ;;
    $setup_case
    *) echo "error: unrecognized subcommand '\${1:-}'" >&2; exit 2 ;;
esac
EOF
    chmod 755 "$ROOT/staging/mccboard"
    echo "license" >"$ROOT/staging/LICENSE"
    echo "readme" >"$ROOT/staging/README.md"
    for target in x86_64-unknown-linux-musl aarch64-unknown-linux-musl; do
        name=mccboard-$1-$target.tar.gz
        tar -czf "$folder/$name" -C "$ROOT/staging" mccboard LICENSE README.md
        if [ "${3:-}" = bad-checksum ]; then
            echo "not the archive" >"$ROOT/staging/other"
            hash=$(sha256sum "$ROOT/staging/other")
            echo "${hash%% *}  $name" >"$folder/$name.sha256"
        else
            (cd "$folder" && sha256sum "$name" >"$name.sha256")
        fi
    done
}

make_fakes() {
    mkdir -p "$ROOT/fake-bin" "$ROOT/curl-bin"
    cat >"$ROOT/fake-bin/uname" <<'EOF'
#!/bin/sh
case "${1:-}" in
    -m) echo "${FAKE_UNAME_M:-x86_64}" ;;
    *) echo "${FAKE_UNAME_S:-Linux}" ;;
esac
EOF
    # Answers the lookup of releases/latest with the URL in FAKE_LATEST and serves the GitHub
    # release downloads from FAKE_RELEASES. The real curl does the rest.
    cat >"$ROOT/curl-bin/curl" <<'EOF'
#!/bin/sh
github=https://github.com/nunziati/mccboard/releases
for arg in "$@"; do
    if [ "$arg" = "$github/latest" ]; then
        printf '%s' "$FAKE_LATEST"
        exit 0
    fi
done
first=yes
for arg in "$@"; do
    if [ "$first" = yes ]; then
        set --
        first=no
    fi
    case $arg in
        "$github/download/"*) arg=file://$FAKE_RELEASES/${arg#"$github/download/"} ;;
    esac
    set -- "$@" "$arg"
done
exec "$REAL_CURL" "$@"
EOF
    chmod 755 "$ROOT/fake-bin/uname" "$ROOT/curl-bin/curl"
}

run_cases() {
    ROOT=$(mktemp -d "${TMPDIR:-/tmp}/mccboard-test-install.XXXXXX")
    OUT=$ROOT/out
    ERR=$ROOT/err
    HOME_DIR="$ROOT/home folder"
    BIN=$HOME_DIR/.local/bin
    mkdir -p "$ROOT/tmp" "$HOME_DIR"
    make_fakes
    make_release 1.0.0 no
    make_release 1.1.0 yes
    make_release 1.2.0 yes bad-checksum
    releases=file://$ROOT/releases

    CASE="--help"
    run --help
    expect_status 0
    expect_in "$OUT" "--uninstall"
    expect_in "$OUT" "MCCBOARD_DOWNLOAD_URL"

    CASE="unknown option"
    run --bogus
    expect_status fail
    expect_in "$ERR" "install.sh: error: unknown option '--bogus'"

    CASE="download URL without a version"
    run MCCBOARD_DOWNLOAD_URL="$releases/v1.0.0"
    expect_status fail
    expect_in "$ERR" "MCCBOARD_DOWNLOAD_URL needs MCCBOARD_VERSION"

    CASE="unsupported architecture"
    run FAKE_UNAME_M=riscv64 MCCBOARD_VERSION=1.0.0 MCCBOARD_DOWNLOAD_URL="$releases/v1.0.0"
    expect_status fail
    expect_in "$ERR" "This machine is Linux on riscv64."
    expect_in "$ERR" "cargo install --path crates/cli"
    expect_in "$ERR" "On Windows"
    if [ -e "$BIN/mccboard" ]; then fail "mccboard was installed"; else pass; fi

    CASE="unsupported system"
    run FAKE_UNAME_S=Darwin FAKE_UNAME_M=arm64 MCCBOARD_VERSION=1.0.0 MCCBOARD_DOWNLOAD_URL="$releases/v1.0.0"
    expect_status fail
    expect_in "$ERR" "This machine is Darwin on arm64."

    CASE="first install"
    run MCCBOARD_VERSION=1.0.0 MCCBOARD_DOWNLOAD_URL="$releases/v1.0.0"
    expect_status 0
    expect_in "$OUT" "for x86_64-unknown-linux-musl"
    expect_in "$OUT" "Installed mccboard 1.0.0 in $BIN."
    expect_installed "$BIN" 1.0.0
    expect_clean "$BIN"
    expect_in "$OUT" "$BIN is not on your PATH"
    expect_in "$OUT" "export PATH=\"$BIN:\$PATH\""
    expect_in "$OUT" "fish_add_path \"$BIN\""
    # 1.0.0 has no setup command. The next steps are then the first commands of the CLI.
    expect_in "$OUT" "has no setup command"
    expect_in "$OUT" "scan"
    if [ -e "$ROOT/setup.log" ]; then fail "the setup ran"; else pass; fi

    CASE="update"
    run MCCBOARD_VERSION=v1.1.0 MCCBOARD_DOWNLOAD_URL="$releases/v1.1.0/"
    expect_status 0
    expect_in "$OUT" "Installed mccboard 1.1.0 in $BIN, in place of mccboard 1.0.0."
    expect_installed "$BIN" 1.1.0
    expect_clean "$BIN"
    # Without a terminal the setup does not start, and the next step is printed instead.
    expect_in "$OUT" "Next, run '$BIN/mccboard' setup"
    if [ -e "$ROOT/setup.log" ]; then fail "the setup ran without a terminal"; else pass; fi

    CASE="wrong checksum"
    cp "$BIN/mccboard" "$ROOT/installed-before"
    run MCCBOARD_VERSION=1.2.0 MCCBOARD_DOWNLOAD_URL="$releases/v1.2.0"
    expect_status fail
    expect_in "$ERR" "does not match its published checksum"
    expect_not_in "$OUT" "Installed"
    expect_same_file "$ROOT/installed-before" "$BIN/mccboard"
    expect_installed "$BIN" 1.1.0
    expect_clean "$BIN"

    CASE="missing version"
    run MCCBOARD_VERSION=9.9.9 MCCBOARD_DOWNLOAD_URL="$releases/v9.9.9"
    expect_status fail
    expect_in "$ERR" "could not download"
    expect_same_file "$ROOT/installed-before" "$BIN/mccboard"
    expect_clean "$BIN"

    CASE="--no-setup"
    run MCCBOARD_VERSION=1.1.0 MCCBOARD_DOWNLOAD_URL="$releases/v1.1.0" --no-setup
    expect_status 0
    expect_in "$OUT" "Installed mccboard 1.1.0 in $BIN, in place of mccboard 1.1.0."
    expect_in "$OUT" "Next, run '$BIN/mccboard' setup"
    if [ -e "$ROOT/setup.log" ]; then fail "the setup ran"; else pass; fi

    CASE="install folder on the PATH"
    run PATH="$BIN:$ROOT/fake-bin:$ORIGINAL_PATH" MCCBOARD_VERSION=1.1.0 MCCBOARD_DOWNLOAD_URL="$releases/v1.1.0"
    expect_status 0
    expect_not_in "$OUT" "not on your PATH"
    expect_in "$OUT" "Next, run mccboard setup"

    CASE="another mccboard first on the PATH"
    mkdir -p "$ROOT/other-bin"
    cp "$ROOT/staging/mccboard" "$ROOT/other-bin/mccboard"
    run PATH="$ROOT/other-bin:$BIN:$ROOT/fake-bin:$ORIGINAL_PATH" MCCBOARD_VERSION=1.1.0 MCCBOARD_DOWNLOAD_URL="$releases/v1.1.0"
    expect_status 0
    expect_in "$OUT" "Another mccboard comes first on your PATH, at $ROOT/other-bin/mccboard."

    CASE="aarch64"
    run FAKE_UNAME_M=arm64 MCCBOARD_VERSION=1.1.0 MCCBOARD_DOWNLOAD_URL="$releases/v1.1.0" --no-setup
    expect_status 0
    expect_in "$OUT" "for aarch64-unknown-linux-musl"
    expect_installed "$BIN" 1.1.0

    CASE="latest release"
    run PATH="$ROOT/curl-bin:$ROOT/fake-bin:$ORIGINAL_PATH" MCCBOARD_INSTALL_DIR="$ROOT/latest-bin" \
        FAKE_LATEST=https://github.com/nunziati/mccboard/releases/tag/v1.0.0
    expect_status 0
    expect_in "$OUT" "The latest release of mccboard is 1.0.0."
    expect_installed "$ROOT/latest-bin" 1.0.0

    CASE="no release yet"
    run PATH="$ROOT/curl-bin:$ROOT/fake-bin:$ORIGINAL_PATH" MCCBOARD_INSTALL_DIR="$ROOT/none-bin" \
        FAKE_LATEST=https://github.com/nunziati/mccboard/releases
    expect_status fail
    expect_in "$ERR" "has no release yet"
    expect_in "$ERR" "MCCBOARD_VERSION="
    if [ -e "$ROOT/none-bin" ]; then fail "the install folder was created"; else pass; fi

    CASE="default install folder"
    run MCCBOARD_INSTALL_DIR= MCCBOARD_VERSION=1.1.0 MCCBOARD_DOWNLOAD_URL="$releases/v1.1.0"
    expect_status 0
    expect_in "$OUT" "Installed mccboard 1.1.0 in $HOME_DIR/.local/bin, in place of mccboard 1.1.0."

    if script --version 2>/dev/null | grep -q util-linux; then
        CASE="terminal"
        run_at_terminal MCCBOARD_VERSION=1.1.0 MCCBOARD_DOWNLOAD_URL="$releases/v1.1.0"
        expect_status 0
        expect_in "$OUT" "Starting mccboard setup."
        expect_in "$OUT" "stand-in setup ran"
        # The setup reads from the terminal even though the script itself reads from the pipe.
        if [ "$(cat "$ROOT/setup.log" 2>/dev/null)" = terminal ]; then
            pass
        else
            fail "the setup did not run with the terminal as its input"
        fi

        CASE="terminal with --no-setup"
        run_at_terminal MCCBOARD_VERSION=1.1.0 MCCBOARD_DOWNLOAD_URL="$releases/v1.1.0" --no-setup
        expect_status 0
        expect_in "$OUT" "Next, run '$BIN/mccboard' setup"
        if [ -e "$ROOT/setup.log" ]; then fail "the setup ran"; else pass; fi

        CASE="terminal, version without setup"
        run_at_terminal MCCBOARD_VERSION=1.0.0 MCCBOARD_DOWNLOAD_URL="$releases/v1.0.0"
        expect_status 0
        expect_in "$OUT" "has no setup command"
        if [ -e "$ROOT/setup.log" ]; then fail "the setup ran"; else pass; fi
        expect_installed "$BIN" 1.0.0
    else
        echo "test-install.sh: skipped the cases at a terminal, which need script(1) of util-linux."
    fi

    CASE="--uninstall"
    mkdir -p "$HOME_DIR/.mccboard"
    echo "language = \"en\"" >"$HOME_DIR/.mccboard/config.toml"
    printf '%s\n' "export EDITOR=vi" "# >>> mccboard launchers >>>" \
        "alias claude-b='CLAUDE_CONFIG_DIR=/x/.claude-b claude'" "# <<< mccboard launchers <<<" \
        >"$HOME_DIR/.bashrc"
    echo "#!/bin/sh" >"$BIN/claude"
    cp "$HOME_DIR/.mccboard/config.toml" "$ROOT/config-before"
    cp "$HOME_DIR/.bashrc" "$ROOT/bashrc-before"
    cp "$BIN/claude" "$ROOT/claude-before"
    run --uninstall
    expect_status 0
    expect_in "$OUT" "Removed mccboard"
    expect_in "$OUT" "Your settings and state stay in $HOME_DIR/.mccboard."
    expect_in "$OUT" "rm -rf \"$HOME_DIR/.mccboard\""
    expect_in "$OUT" "The launchers stay in $HOME_DIR/.bashrc."
    if [ -e "$BIN/mccboard" ]; then fail "mccboard is still there"; else pass; fi
    expect_same_file "$ROOT/config-before" "$HOME_DIR/.mccboard/config.toml"
    expect_same_file "$ROOT/bashrc-before" "$HOME_DIR/.bashrc"
    expect_same_file "$ROOT/claude-before" "$BIN/claude"

    CASE="--uninstall with nothing installed"
    run --uninstall
    expect_status 0
    expect_in "$OUT" "There is no mccboard in $BIN."

    rm -rf "$ROOT"
}

for SH in "$@"; do
    if ! command -v "$SH" >/dev/null 2>&1; then
        CASE=setup
        OUT=/dev/null
        ERR=/dev/null
        fail "the shell $SH is not on the PATH"
        continue
    fi
    run_cases
done

if [ "$failures" -ne 0 ]; then
    echo "test-install.sh: $failures failed, $checks passed."
    exit 1
fi
echo "test-install.sh: all $checks checks passed with: $*"
