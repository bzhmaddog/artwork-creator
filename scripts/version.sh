#!/bin/sh
# Suggests the next version from the commits since the last release, and makes releases.
# Run with `make version` and `make release`.
#
# Usage:
#   version.sh [suggest]
#       Show the current version, the commits since the last release tag (vX.Y.Z) and the
#       suggested next version, with the reason.
#   version.sh release [VERSION|major|minor|patch] [--sdk VERSION|major|minor|patch]
#       Set the app version (the suggested one when none is given), and the plugin SDK
#       version with --sdk, move the "Unreleased" section of CHANGELOG.md (if there is
#       one) under the new version, then commit and tag vX.Y.Z (and sdk-vX.Y.Z). Asks
#       before committing. Nothing is pushed.
#
# Only commits that change what is shipped count: the paths in APP_PATHS (the app code,
# translations, templates, plugin SDK and dependencies). Commits that only touch tooling,
# CI, docs or tests are listed apart and never call for a release.
#
# How the next version is suggested, from the messages of those commits:
#   - breaking change: a message containing "BREAKING", a "type!:" prefix, or a change
#     of ABI_VERSION in the plugin SDK -> major (minor while below 1.0.0);
#   - new feature: a message with a word such as "add", "new", "feat", "implement",
#     "support" -> minor;
#   - anything else -> patch.
# With no tag yet, the current version is suggested as the first release.
set -e

cd "$(git rev-parse --show-toplevel)"

APP_TOML=Cargo.toml
SDK_TOML=plugin-sdk/Cargo.toml
ABI_FILE=plugin-sdk/src/abi.rs
CHANGELOG=CHANGELOG.md
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

last_tag() {
    git describe --tags --abbrev=0 --match 'v[0-9]*' 2>/dev/null || true
}

# Sets $level (major, minor or patch), $reason and $range from the commits since the
# last tag. $level is empty when there is nothing to release.
analyze() {
    tag=$(last_tag)
    current=$(toml_version "$APP_TOML")
    if [ -z "$tag" ]; then
        range=HEAD
        level=first
        reason="no release tag yet: $current would be the first release"
        return
    fi
    range="$tag..HEAD"
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

suggested_version() {
    case "$level" in
        first) echo "$current" ;;
        '') echo "" ;;
        *) bump "$current" "$level" ;;
    esac
}

sdk_status() {
    sdk=$(toml_version "$SDK_TOML")
    [ "$range" = HEAD ] && return
    if git diff "$range" -- "$ABI_FILE" | grep -Eq '^[+-]pub const ABI_VERSION'; then
        echo "Plugin SDK $sdk: the plugin interface changed, release it too, e.g. --sdk $(bump "$sdk" "$([ "${sdk%%.*}" = 0 ] && echo minor || echo major)")"
    elif [ -n "$(git diff --name-only "$range" -- plugin-sdk)" ]; then
        echo "Plugin SDK $sdk: changed since ${range%%..*}, consider --sdk patch or --sdk minor"
    fi
}

cmd_suggest() {
    analyze
    echo "Current version: $current"
    echo "Last release:    ${tag:-none}"
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
        echo "Release it with: make release (or make release V=X.Y.Z, V=major|minor|patch)"
    fi
}

cmd_release() {
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

    analyze
    if [ -n "$want" ]; then
        new=$(resolve "$current" "$want")
    else
        new=$(suggested_version)
        [ -n "$new" ] || die "nothing to release ($reason)"
    fi
    git rev-parse -q --verify "refs/tags/v$new" >/dev/null && die "tag v$new already exists"

    sdk_old=$(toml_version "$SDK_TOML")
    sdk_new=
    if [ -n "$sdk_want" ]; then
        sdk_new=$(resolve "$sdk_old" "$sdk_want")
        git rev-parse -q --verify "refs/tags/sdk-v$sdk_new" >/dev/null && die "tag sdk-v$sdk_new already exists"
    fi

    echo "Branch: $(git branch --show-current)"
    if [ "$new" = "$current" ]; then
        echo "App:    $current (unchanged), tag v$new"
    else
        echo "App:    $current -> $new, tag v$new"
    fi
    [ -n "$sdk_new" ] && echo "SDK:    $sdk_old -> $sdk_new, tag sdk-v$sdk_new"
    [ -z "$want" ] && echo "Reason: $reason"
    [ -z "$sdk_new" ] && sdk_status
    printf 'Commit and tag? [y/N] '
    read -r answer
    case "$answer" in
        y | Y | yes) ;;
        *) die "canceled" ;;
    esac

    [ "$new" != "$current" ] && set_toml_version "$APP_TOML" "$new"
    [ -n "$sdk_new" ] && [ "$sdk_new" != "$sdk_old" ] && set_toml_version "$SDK_TOML" "$sdk_new"
    if [ -n "$(git status --porcelain -- "$APP_TOML" "$SDK_TOML")" ]; then
        # Brings the workspace versions in Cargo.lock up to date.
        cargo update --workspace --quiet
    fi
    if [ -f "$CHANGELOG" ] && grep -q '^## Unreleased' "$CHANGELOG"; then
        sed -i "s/^## Unreleased.*/## $new - $(date +%Y-%m-%d)/" "$CHANGELOG"
    fi

    if [ -n "$(git status --porcelain)" ]; then
        git add -A "$APP_TOML" "$SDK_TOML" Cargo.lock
        [ -f "$CHANGELOG" ] && git add "$CHANGELOG"
        git commit -q -m "Release v$new"
    fi
    git tag -a "v$new" -m "Artwork Creator $new"
    [ -n "$sdk_new" ] && git tag -a "sdk-v$sdk_new" -m "Artwork plugin SDK $sdk_new"

    echo "Released v$new. Publish it with:"
    if [ -n "$sdk_new" ]; then
        echo "  git push && git push origin v$new sdk-v$sdk_new"
    else
        echo "  git push && git push origin v$new"
    fi
}

case "${1:-suggest}" in
    suggest) cmd_suggest ;;
    release)
        shift
        cmd_release "$@"
        ;;
    -h | --help | help) sed -n '2,/^set -e/p' "$0" | sed '$d; s/^# \{0,1\}//' ;;
    *) die "unknown command $1 (use suggest or release)" ;;
esac
