#!/bin/sh
# Installs, updates or removes the mccboard CLI on Linux, from a release on GitHub.
#
#   curl -fsSL https://raw.githubusercontent.com/nunziati/mccboard/main/install.sh | sh
#
# `--help` lists the options and the environment variables. The script never asks for root and
# writes only in the install folder and in a temporary folder that it removes on exit. Only the
# last line runs anything, by calling main. A download cut short therefore runs nothing.

set -eu

REPO_URL=https://github.com/nunziati/mccboard
SCRIPT_URL=https://raw.githubusercontent.com/nunziati/mccboard/main/install.sh
PREFIX=install.sh:

# The temporary folder and the staged binary of an install. cleanup removes both on exit.
TMP_DIR=
STAGED=

# Prints the first argument after the prefix and each further argument indented below it.
say() {
    printf '%s %s\n' "$PREFIX" "$1"
    shift
    for say_line in "$@"; do
        printf '    %s\n' "$say_line"
    done
}

# Prints an error on stderr, in the layout of say, and stops with status 1.
die() {
    die_first=$1
    shift
    say "error: $die_first" "$@" >&2
    exit 1
}

usage() {
    cat <<EOF
Installs the mccboard CLI on Linux (x86_64 or aarch64) from a GitHub release, then starts
\`mccboard setup\` when it runs at a terminal. Running it again updates mccboard.

Usage:
  curl -fsSL $SCRIPT_URL | sh
  curl -fsSL $SCRIPT_URL | sh -s -- [OPTION]
  sh install.sh [OPTION]

Options:
  --no-setup   Install without starting \`mccboard setup\` and print the next steps.
  --uninstall  Remove the mccboard binary. ~/.mccboard and the launchers stay.
  -h, --help   Print this help.

Environment:
  MCCBOARD_VERSION       The version to install, such as 0.2.0. Default: the latest release.
  MCCBOARD_INSTALL_DIR   The folder of the binary. Default: ~/.local/bin.
  MCCBOARD_DOWNLOAD_URL  The folder that holds the archives, for mirrors and tests. It needs
                         MCCBOARD_VERSION. Default: $REPO_URL/releases/download/v<version>.

The script never asks for root. It writes only in the install folder and in a temporary folder
that it removes on exit.
EOF
}

cleanup() {
    if [ -n "$STAGED" ]; then
        rm -f "$STAGED"
    fi
    if [ -n "$TMP_DIR" ]; then
        rm -rf "$TMP_DIR"
    fi
}

# Finds the install folder and stores it in INSTALL_DIR, as an absolute path without a trailing
# slash.
find_install_dir() {
    INSTALL_DIR=${MCCBOARD_INSTALL_DIR:-}
    if [ -z "$INSTALL_DIR" ]; then
        if [ -z "${HOME:-}" ]; then
            die "HOME is empty. Name the folder for mccboard in MCCBOARD_INSTALL_DIR."
        fi
        INSTALL_DIR=$HOME/.local/bin
    fi
    case $INSTALL_DIR in
        /*) ;;
        *) INSTALL_DIR=$(pwd)/$INSTALL_DIR ;;
    esac
    while [ "$INSTALL_DIR" != / ] && [ "${INSTALL_DIR%/}" != "$INSTALL_DIR" ]; do
        INSTALL_DIR=${INSTALL_DIR%/}
    done
}

# Stores in TARGET the Rust target of the release archive that runs on this machine.
find_target() {
    os=$(uname -s)
    arch=$(uname -m)
    TARGET=
    if [ "$os" = Linux ]; then
        case $arch in
            x86_64 | amd64) TARGET=x86_64-unknown-linux-musl ;;
            aarch64 | arm64) TARGET=aarch64-unknown-linux-musl ;;
        esac
    fi
    if [ -z "$TARGET" ]; then
        die "mccboard publishes binaries only for Linux on x86_64 and aarch64. This machine is $os on $arch." \
            "On Windows, the installer of the mccboard app installs the CLI as well." \
            "Elsewhere, build the CLI from source by running cargo install --path crates/cli in a clone of $REPO_URL."
    fi
}

# Chooses the download and checksum tools among those on the PATH, in DOWNLOADER and HASHER.
find_tools() {
    if command -v curl >/dev/null 2>&1; then
        DOWNLOADER=curl
    elif command -v wget >/dev/null 2>&1; then
        DOWNLOADER=wget
    else
        die "the script needs curl or wget to download mccboard. Install one of them and run it again."
    fi
    if command -v sha256sum >/dev/null 2>&1; then
        HASHER=sha256sum
    elif command -v shasum >/dev/null 2>&1; then
        HASHER=shasum
    else
        die "the script needs sha256sum or shasum to check the download against its checksum. Install one of them and run it again."
    fi
    if ! command -v tar >/dev/null 2>&1; then
        die "the script needs tar to unpack the download. Install it and run the script again."
    fi
}

# Downloads the URL $1 into the file $2.
download() {
    if [ "$DOWNLOADER" = curl ]; then
        curl -fsSL -o "$2" "$1"
    else
        wget -q -O "$2" "$1"
    fi
}

# Stops unless VERSION looks like a version. Other characters, such as a slash, could change the
# paths and URLs built from it.
check_version() {
    case $VERSION in
        '' | *[!0-9A-Za-z.+-]*) die "'$VERSION' is not a version of mccboard, such as 0.2.0." ;;
    esac
}

# Stores the latest release in VERSION. GitHub redirects releases/latest to the tag of the latest
# release, or to the list of releases while there is none. A pre-release is never the latest.
find_latest_version() {
    latest_url=$REPO_URL/releases/latest
    if [ "$DOWNLOADER" = curl ]; then
        location=$(curl -fsSIL -o /dev/null -w '%{url_effective}' "$latest_url" 2>/dev/null) || location=
    else
        # wget prints the headers of every response it follows. The last Location header names
        # the page where it ended.
        location=$(wget -S --spider "$latest_url" 2>&1 | tr -d '\r' |
            sed -n 's/^ *[Ll]ocation: *\([^ ]*\).*/\1/p' | tail -n 1) || location=
    fi
    case $location in
        */releases/tag/v?*)
            VERSION=${location##*/releases/tag/v}
            ;;
        */releases | */releases/)
            die "$REPO_URL has no release yet." \
                "To install a pre-release, name its version, as in curl -fsSL $SCRIPT_URL | MCCBOARD_VERSION=0.2.0-rc.1 sh" \
                "To build the CLI from source, run cargo install --path crates/cli in a clone of $REPO_URL."
            ;;
        *)
            die "could not read the latest version of mccboard from $latest_url." \
                "Check the connection, or name the version with MCCBOARD_VERSION."
            ;;
    esac
    check_version
    say "The latest release of mccboard is $VERSION."
}

# Prints the version that the mccboard at $1 reports, or nothing when it does not run.
version_of() {
    version_out=$("$1" --version </dev/null 2>/dev/null) || return 0
    printf '%s\n' "$version_out" | sed -n '1s/^mccboard //p'
}

# True when the mccboard at $1 has the setup command, which older versions lack.
has_setup() {
    "$1" setup --help </dev/null >/dev/null 2>&1
}

# True when stdout is a terminal and the controlling terminal opens. In CI or in a provisioning
# script, at least one of the two fails.
at_terminal() {
    [ -t 1 ] && (: </dev/tty) 2>/dev/null
}

run_uninstall() {
    target=$INSTALL_DIR/mccboard
    if [ -d "$target" ]; then
        die "$target is a folder, not the mccboard binary. Nothing was removed."
    elif [ -e "$target" ] || [ -L "$target" ]; then
        old=$(version_of "$target")
        rm -f "$target" || die "could not remove $target."
        if [ -n "$old" ]; then
            say "Removed mccboard $old from $INSTALL_DIR."
        else
            say "Removed $target."
        fi
    else
        say "There is no mccboard in $INSTALL_DIR. Nothing was removed."
        other=$(command -v mccboard 2>/dev/null) || other=
        if [ -n "$other" ]; then
            say "The mccboard on your PATH is $other. To remove it, run the script with its folder in MCCBOARD_INSTALL_DIR."
        fi
    fi

    if [ -z "${HOME:-}" ]; then
        return 0
    fi
    if [ -d "$HOME/.mccboard" ]; then
        say "Your settings and state stay in $HOME/.mccboard. To remove them as well, run this command:" \
            "rm -rf \"$HOME/.mccboard\""
    fi
    for rc_file in "$HOME/.bashrc" "$HOME/.zshrc"; do
        if [ -f "$rc_file" ] && grep -q '^# >>> mccboard launchers >>>' "$rc_file"; then
            say "The launchers stay in $rc_file. To remove them, delete these two lines and the lines between them, then open a new terminal:" \
                "# >>> mccboard launchers >>>" \
                "# <<< mccboard launchers <<<"
        fi
    done
}

run_install() {
    find_target
    find_tools

    if [ -n "${MCCBOARD_VERSION:-}" ]; then
        VERSION=${MCCBOARD_VERSION#v}
        check_version
    elif [ -n "${MCCBOARD_DOWNLOAD_URL:-}" ]; then
        die "MCCBOARD_DOWNLOAD_URL needs MCCBOARD_VERSION too, because the script reads the latest version only from GitHub."
    else
        find_latest_version
    fi

    base_url=${MCCBOARD_DOWNLOAD_URL:-$REPO_URL/releases/download/v$VERSION}
    base_url=${base_url%/}
    archive=mccboard-$VERSION-$TARGET.tar.gz

    TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/mccboard-install.XXXXXX") ||
        die "could not create a temporary folder in ${TMPDIR:-/tmp}."

    say "Downloading mccboard $VERSION for $TARGET from $base_url."
    if ! download "$base_url/$archive" "$TMP_DIR/$archive" ||
        ! download "$base_url/$archive.sha256" "$TMP_DIR/$archive.sha256"; then
        if [ -n "${MCCBOARD_DOWNLOAD_URL:-}" ]; then
            die "could not download $archive and its checksum from $base_url." \
                "Check that the folder holds $archive and $archive.sha256."
        fi
        die "could not download $archive and its checksum from $base_url." \
            "Check the connection, and look on $REPO_URL/releases for version $VERSION and its archive for $TARGET."
    fi

    # The .sha256 file holds the output of sha256sum, the checksum followed by two spaces and the
    # file name.
    expected=$(sed -n '1s/^\([0-9A-Fa-f]*\).*/\1/p' "$TMP_DIR/$archive.sha256" | tr 'A-F' 'a-f')
    if [ "${#expected}" -ne 64 ]; then
        die "$archive.sha256 does not hold a SHA-256 checksum. Nothing was installed."
    fi
    if [ "$HASHER" = sha256sum ]; then
        actual=$(sha256sum "$TMP_DIR/$archive") || die "sha256sum could not read $archive."
    else
        actual=$(shasum -a 256 "$TMP_DIR/$archive") || die "shasum could not read $archive."
    fi
    actual=${actual%% *}
    if [ "$actual" != "$expected" ]; then
        die "$archive does not match its published checksum. Nothing was installed or changed." \
            "published: $expected" \
            "download:  $actual"
    fi
    say "The archive matches its published SHA-256 checksum."

    mkdir "$TMP_DIR/unpacked"
    tar -xzf "$TMP_DIR/$archive" -C "$TMP_DIR/unpacked" || die "could not unpack $archive."
    if [ ! -f "$TMP_DIR/unpacked/mccboard" ]; then
        die "$archive does not hold mccboard."
    fi

    mkdir -p "$INSTALL_DIR" || die "could not create $INSTALL_DIR."
    target=$INSTALL_DIR/mccboard
    if [ -d "$target" ]; then
        die "$target is a folder. Move it away and run the script again."
    fi
    old=
    replacing=no
    if [ -e "$target" ] || [ -L "$target" ]; then
        replacing=yes
        old=$(version_of "$target")
    fi

    # The new binary is copied next to the target and then renamed over it. A rename replaces
    # the old binary in one step, even while it runs. Before the rename, the new binary runs
    # once from the install folder to catch a binary that this machine cannot run.
    STAGED=$(mktemp "$INSTALL_DIR/.mccboard.new.XXXXXX") || die "could not write in $INSTALL_DIR."
    cp "$TMP_DIR/unpacked/mccboard" "$STAGED" || die "could not write in $INSTALL_DIR."
    chmod 755 "$STAGED"
    new=$(version_of "$STAGED")
    if [ -z "$new" ]; then
        die "the downloaded mccboard does not run on this machine (\`mccboard --version\` failed). Nothing was installed or changed."
    fi
    mv -f "$STAGED" "$target" || die "could not move the new mccboard to $target."
    STAGED=
    rm -rf "$TMP_DIR"
    TMP_DIR=

    if [ "$replacing" = no ]; then
        say "Installed mccboard $new in $INSTALL_DIR."
    elif [ -n "$old" ]; then
        say "Installed mccboard $new in $INSTALL_DIR, in place of mccboard $old."
    else
        say "Installed mccboard $new in $INSTALL_DIR, in place of an mccboard that did not report its version."
    fi

    # The command shown to the user is mccboard when the PATH finds this binary first, or its full
    # path otherwise.
    command_name=$target
    case $command_name in
        *' '*) command_name="'$command_name'" ;;
    esac
    case ":${PATH:-}:" in
        *":$INSTALL_DIR:"* | *":$INSTALL_DIR/:"*)
            found=$(command -v mccboard 2>/dev/null) || found=
            if [ -n "$found" ] && [ "$found" != "$target" ] && [ "$found" != "$INSTALL_DIR//mccboard" ]; then
                say "Another mccboard comes first on your PATH, at $found. To run the one just installed, remove that one or move $INSTALL_DIR before its folder in your PATH."
            else
                command_name=mccboard
            fi
            ;;
        *)
            say "$INSTALL_DIR is not on your PATH. To add it, put the line for your shell at the end of its file and open a new terminal:" \
                "bash (~/.bashrc):                  export PATH=\"$INSTALL_DIR:\$PATH\"" \
                "zsh (~/.zshrc):                    export PATH=\"$INSTALL_DIR:\$PATH\"" \
                "fish (~/.config/fish/config.fish): fish_add_path \"$INSTALL_DIR\""
            ;;
    esac

    if ! has_setup "$target"; then
        say "This version of mccboard has no setup command. Start with these commands:" \
            "$command_name scan               lists the instances in your home folder" \
            "$command_name register ~/.claude registers the primary instance" \
            "$command_name add <label>        adds an account, with its instance in ~/.claude-<label>" \
            "$command_name --help             lists every command"
    elif [ "$SETUP" = yes ] && at_terminal; then
        say "Starting mccboard setup. You can stop at any step and run $command_name setup again later."
        setup_status=0
        "$target" setup </dev/tty || setup_status=$?
        if [ "$setup_status" -ne 0 ]; then
            say "mccboard is installed, but mccboard setup ended with status $setup_status." \
                "Run $command_name setup to start it again." >&2
            exit "$setup_status"
        fi
    else
        say "Next, run $command_name setup to register your instances and add your accounts."
    fi
}

main() {
    SETUP=yes
    action=run_install
    for arg in "$@"; do
        case $arg in
            --no-setup) SETUP=no ;;
            --uninstall) action=run_uninstall ;;
            -h | --help)
                usage
                exit 0
                ;;
            *) die "unknown option '$arg'. The options are --no-setup, --uninstall and --help." ;;
        esac
    done

    trap cleanup EXIT
    trap 'exit 129' HUP
    trap 'exit 130' INT
    trap 'exit 143' TERM

    find_install_dir
    "$action"
}

main "$@"
