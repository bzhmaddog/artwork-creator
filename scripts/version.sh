#!/bin/sh
# Suggests the next version from the commits since the last release and bumps the
# version. Run with `make version-check` and `make version-bump-*`. A release is a vX.Y.Z
# tag, made by hand (see doc/releasing.md).
#
# Usage:
#   version.sh [check]
#       Show the current version, the last release (its vX.Y.Z tag), the commits since
#       and the suggested next version, with the reason.
#   version.sh bump VERSION|major|minor|patch [--sdk VERSION|major|minor|patch]
#       Set the app version (major, minor and patch count from the current one), and the
#       plugin SDK version with --sdk, and commit it as "Bump version to X.Y.Z". Asks
#       before committing; no tag is made and nothing is pushed.
#
# Only commits that change what is shipped count: the paths in APP_PATHS (the app code,
# translations, templates, plugin SDK and dependencies). Commits that only touch tooling,
# CI, docs or tests are listed apart and never call for a release.
#
# How the next version is suggested, from the last release's and the messages of those
# commits:
#   - breaking change: a message containing "BREAKING", a "type!:" prefix, or a change
#     of ABI_VERSION in the plugin SDK -> major (minor while below 1.0.0);
#   - new feature: a message with a word such as "add", "new", "feat", "implement",
#     "support" -> minor;
#   - anything else -> patch.
# A version Cargo.toml was already bumped to is suggested when it is that far. With no
# release yet, the current version is suggested as the first release.
set -e

cd "$(git rev-parse --show-toplevel)"

APP_TOML=Cargo.toml
SDK_TOML=plugin-sdk/Cargo.toml
ABI_FILE=plugin-sdk/src/abi.rs
APP_PATHS='src locales templates plugin-sdk Cargo.toml Cargo.lock'
FEATURE_WORDS='add|added|adds|adding|new|feat|feature|features|implement|implemented|support|supports'

die() {
    echo "version: $*" >&2
    exit 1
}

# The [package] version of a Cargo.toml: its first `version = "..."` line.
toml_version() {
    sed -n 's/^version = "\(.*\)"/\1/p' "$1" | head -n 1
}

set_toml_version() {
    awk -v v="$2" '!done && /^version = "/ { print "version = \"" v "\""; done = 1; next } { print }' \
        "$1" >"$1.tmp" && mv "$1.tmp" "$1"
}

is_version() {
    echo "$1" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'
}

# bump VERSION major|minor|patch
bump() {
    echo "$1" | awk -F. -v part="$2" '{
        if (part == "major") print $1 + 1 ".0.0"
        else if (part == "minor") print $1 "." $2 + 1 ".0"
        else print $1 "." $2 "." $3 + 1
    }'
}

# resolve CURRENT VERSION|major|minor|patch -> VERSION
resolve() {
    case "$2" in
        major | minor | patch) bump "$1" "$2" ;;
        *)
            is_version "$2" || die "'$2' is not a version (X.Y.Z) nor major, minor or patch"
            echo "$2"
            ;;
    esac
}

# Sets $base and $tag (the last vX.Y.Z tag reachable from HEAD) and $released (its
# version); all empty without one.
last_release() {
    tag=$(git describe --tags --abbrev=0 --match 'v[0-9]*' 2>/dev/null || true)
    base=$tag
    released=${tag#v}
}

# version_gt A B: A is a later version than B.
version_gt() {
    [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | tail -n 1)" = "$1" ]
}

# Sets $level (major, minor or patch), $reason and $range from the commits since the
# last release. $level is empty when there is nothing to release.
analyze() {
    last_release
    current=$(toml_version "$APP_TOML")
    if [ -z "$base" ]; then
        range=HEAD
        level=first
        reason="no release yet: $current would be the first release"
        return
    fi
    range="$base..HEAD"
    if [ -z "$(git rev-list "$range")" ]; then
        level=
        reason="no commits since $tag"
        return
    fi

    # shellcheck disable=SC2086 # APP_PATHS is a list of paths
    if [ -z "$(git rev-list "$range" -- $APP_PATHS)" ]; then
        level=
        reason="no app changes since $tag, only tooling, CI, docs or tests"
        return
    fi
    # shellcheck disable=SC2086
    messages=$(git log --format='%s%n%b' "$range" -- $APP_PATHS)
    if git diff "$range" -- "$ABI_FILE" | grep -Eq '^[+-]pub const ABI_VERSION'; then
        level=major
        reason="the plugin interface (ABI_VERSION) changed"
    elif echo "$messages" | grep -Eq 'BREAKING|^[a-z]+(\([^)]*\))?!:'; then
        level=major
        reason="a commit is marked as breaking"
    elif echo "$messages" | grep -Eiwq "$FEATURE_WORDS"; then
        level=minor
        reason="new features: $(git log --format='%s' "$range" -- $APP_PATHS | grep -Eiw "$FEATURE_WORDS" | head -n 3 | sed 's/^/"/; s/$/"/' | paste -sd, - | sed 's/,/, /g')"
    else
        level=patch
        reason="fixes and changes only"
    fi
    # Below 1.0.0, breaking changes only raise the minor version.
    if [ "$level" = major ] && [ "${current%%.*}" = 0 ]; then
        level=minor
        reason="$reason (minor while below 1.0.0)"
    fi
}

# analyze, the reason saying so when the version suggested is the one Cargo.toml was
# bumped to.
analyze_bumped() {
    analyze
    if [ -n "$released" ] && version_gt "$current" "$released" &&
        [ "$(suggested_version)" = "$current" ]; then
        reason="Cargo.toml was bumped to $current since $tag"
    fi
}

# The next version: the last release's raised by $level, or the version Cargo.toml was
# already bumped to when that is later; empty when there is nothing to release.
suggested_version() {
    case "$level" in
        first) echo "$current" ;;
        '') version_gt "$current" "${released:-$current}" && echo "$current" || echo "" ;;
        *)
            next=$(bump "$released" "$level")
            if version_gt "$current" "$next"; then echo "$current"; else echo "$next"; fi
            ;;
    esac
}

sdk_status() {
    sdk=$(toml_version "$SDK_TOML")
    [ "$range" = HEAD ] && return
    if git diff "$range" -- "$ABI_FILE" | grep -Eq '^[+-]pub const ABI_VERSION'; then
        echo "Plugin SDK $sdk: the plugin interface changed, release it too, e.g. --sdk $(bump "$sdk" "$([ "${sdk%%.*}" = 0 ] && echo minor || echo major)")"
    elif [ -n "$(git diff --name-only "$range" -- plugin-sdk)" ]; then
        echo "Plugin SDK $sdk: changed since $tag, consider --sdk patch or --sdk minor"
    fi
}

cmd_check() {
    analyze_bumped
    echo "Current version: $current"
    echo "Last release:    ${tag:-none}"
    if [ -n "$released" ] && version_gt "$current" "$released"; then
        echo "                 (Cargo.toml was bumped since)"
    fi
    if [ -n "$tag" ] && [ -n "$(git rev-list "$range")" ]; then
        count=$(git rev-list --count "$range")
        echo
        echo "$count commit(s) since $tag:"
        # shellcheck disable=SC2086
        app=$(git rev-list "$range" -- $APP_PATHS)
        for c in $(git rev-list "$range"); do
            if echo "$app" | grep -q "$c"; then
                git log -1 --format='  %h %s' "$c"
            else
                git log -1 --format='  %h %s  (not in the app)' "$c"
            fi
        done
    fi
    echo
    next=$(suggested_version)
    if [ -n "$next" ]; then
        echo "Suggested version: $next ($reason)"
    else
        echo "Nothing to release ($reason)"
    fi
    sdk_status
    if [ -n "$next" ]; then
        echo
        if [ "$next" = "$current" ]; then
            echo "Cargo.toml is at it: tag v$next to release it (see doc/releasing.md)"
        else
            echo "Set it with: make version-bump V=$next"
        fi
    fi
}

# cmd_bump VERSION|major|minor|patch [--sdk VERSION|major|minor|patch]
cmd_bump() {
    want=
    sdk_want=
    while [ $# -gt 0 ]; do
        case "$1" in
            --sdk)
                [ $# -ge 2 ] || die "--sdk needs a version, or major, minor or patch"
                sdk_want=$2
                shift 2
                ;;
            -*) die "unknown option $1" ;;
            *)
                [ -z "$want" ] || die "only one version can be given"
                want=$1
                shift
                ;;
        esac
    done

    [ -z "$(git status --porcelain)" ] || die "the working tree has changes; commit or stash them first"
    [ -n "$want" ] || die "give the version: make version-bump-patch|minor|major, or make version-bump V=X.Y.Z (make version-check suggests one)"

    analyze_bumped
    new=$(resolve "$current" "$want")
    git rev-parse -q --verify "refs/tags/v$new" >/dev/null && die "v$new was already released"

    sdk_old=$(toml_version "$SDK_TOML")
    sdk_new=
    [ -n "$sdk_want" ] && sdk_new=$(resolve "$sdk_old" "$sdk_want")
    [ "$new" != "$current" ] || [ -n "$sdk_new" ] || die "the version is $current already"
    subject="Bump version to $new"
    [ -n "$sdk_new" ] && subject="$subject (plugin SDK $sdk_new)"

    echo "Branch: $(git branch --show-current)"
    if [ "$new" = "$current" ]; then
        echo "App:    $current (unchanged)"
    else
        echo "App:    $current -> $new"
    fi
    [ -n "$sdk_new" ] && echo "SDK:    $sdk_old -> $sdk_new"
    echo "Commit: $subject"
    [ -z "$sdk_new" ] && sdk_status
    printf 'Commit? [y/N] '
    read -r answer
    case "$answer" in
        y | Y | yes) ;;
        *) die "canceled" ;;
    esac

    [ "$new" != "$current" ] && set_toml_version "$APP_TOML" "$new"
    [ -n "$sdk_new" ] && [ "$sdk_new" != "$sdk_old" ] && set_toml_version "$SDK_TOML" "$sdk_new"
    # Brings the workspace versions in Cargo.lock up to date.
    cargo update --workspace --quiet
    git add -A "$APP_TOML" "$SDK_TOML" Cargo.lock
    git commit -q -m "$subject"

    echo "Version set to $new. Nothing is pushed. To release it, merge this commit into main"
    echo "through a pull request, then tag the merged main and push the tag:"
    echo "  git switch main && git pull"
    echo "  git tag v$new"
    echo "  git push origin v$new"
    echo "GitHub Actions then builds the packages into a draft release to publish by hand;"
    echo "don't create the release on GitHub (see doc/releasing.md)."
}

case "${1:-check}" in
    check) cmd_check ;;
    bump)
        shift
        cmd_bump "$@"
        ;;
    -h | --help | help) sed -n '2,/^set -e/p' "$0" | sed '$d; s/^# \{0,1\}//' ;;
    *) die "unknown command $1 (use check or bump)" ;;
esac
