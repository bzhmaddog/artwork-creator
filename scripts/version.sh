#!/bin/sh
# Suggests the next version from the commits since the last release, bumps the version
# and makes releases. Run with `make version-check`, `make version-bump` and
# `make version-release`.
#
# Usage:
#   version.sh [check]
#       Show the current version, the last release (its "Release vX.Y.Z" commit), the
#       commits since and the suggested next version, with the reason.
#   version.sh bump VERSION|major|minor|patch [--sdk VERSION|major|minor|patch]
#       Set the app version (major, minor and patch count from the current one), and the
#       plugin SDK version with --sdk, and commit it as "Bump version to X.Y.Z". Not a
#       release: the next suggestion still counts from the last release.
#   version.sh release [VERSION|major|minor|patch] [--sdk VERSION|major|minor|patch]
#       Release a version: the one given, or else the one Cargo.toml was bumped to since
#       the last release, or else the suggested one. Sets it, moves the "Unreleased"
#       section of CHANGELOG.md (if there is one) under it and commits it as
#       "Release vX.Y.Z".
#   Both ask before committing. No tag is made and nothing is pushed.
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
# release yet, the current version is suggested as the first release. Releases made
# before the script stopped tagging are found by their vX.Y.Z tag when no release commit
# is.
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

RELEASE_SUBJECT='^Release v[0-9]+\.[0-9]+\.[0-9]+'

# Sets $base (the last release's commit), $released (its version) and $tag (its name,
# "vX.Y.Z (abc1234)"): the last "Release vX.Y.Z" commit, or else the last vX.Y.Z tag; all
# empty without one.
last_release() {
    base=$(git log -1 -E --grep="$RELEASE_SUBJECT" --format=%H 2>/dev/null || true)
    if [ -n "$base" ]; then
        released=$(git log -1 --format=%s "$base" | sed -E 's/^Release v([0-9]+\.[0-9]+\.[0-9]+).*/\1/')
        tag="v$released ($(git rev-parse --short "$base"))"
        return
    fi
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
        echo "Release it with: make version-release (version-release-patch|minor|major or V=X.Y.Z picks another)"
        [ "$next" != "$current" ] && echo "or set it without releasing: make version-bump V=$next"
    fi
}

# cmd_set bump|release [VERSION|major|minor|patch] [--sdk VERSION|major|minor|patch]
cmd_set() {
    kind=$1
    shift
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

    analyze_bumped
    if [ -n "$want" ]; then
        new=$(resolve "$current" "$want")
    elif [ "$kind" = bump ]; then
        die "give the version: make version-bump-patch|minor|major, or V=X.Y.Z (make version-check suggests one)"
    else
        new=$(suggested_version)
        [ -n "$new" ] || die "nothing to release ($reason)"
    fi

    sdk_old=$(toml_version "$SDK_TOML")
    sdk_new=
    [ -n "$sdk_want" ] && sdk_new=$(resolve "$sdk_old" "$sdk_want")

    if [ "$kind" = bump ]; then
        [ "$new" != "$current" ] || [ -n "$sdk_new" ] || die "the version is $current already"
        subject="Bump version to $new"
    else
        if git log -E --grep="^Release v$new( |$)" --format=%s | grep -q . ||
            git rev-parse -q --verify "refs/tags/v$new" >/dev/null; then
            die "v$new was already released"
        fi
        subject="Release v$new"
    fi
    [ -n "$sdk_new" ] && subject="$subject (plugin SDK $sdk_new)"

    echo "Branch: $(git branch --show-current)"
    if [ "$new" = "$current" ]; then
        echo "App:    $current (unchanged)"
    else
        echo "App:    $current -> $new"
    fi
    [ -n "$sdk_new" ] && echo "SDK:    $sdk_old -> $sdk_new"
    echo "Commit: $subject"
    [ "$kind" = release ] && [ -z "$want" ] && echo "Reason: $reason"
    [ -z "$sdk_new" ] && sdk_status
    printf 'Commit? [y/N] '
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
    if [ "$kind" = release ] && [ -f "$CHANGELOG" ] && grep -q '^## Unreleased' "$CHANGELOG"; then
        sed -i "s/^## Unreleased.*/## $new - $(date +%Y-%m-%d)/" "$CHANGELOG"
    fi

    git add -A "$APP_TOML" "$SDK_TOML" Cargo.lock
    [ -f "$CHANGELOG" ] && git add "$CHANGELOG"
    # A release is committed even with nothing changed (the version was bumped before):
    # the commit marks it.
    git commit -q --allow-empty -m "$subject"

    if [ "$kind" = bump ]; then
        echo "Version set to $new (not released). Nothing is pushed."
        return
    fi
    echo "Released v$new (no tag made). Publish it with:"
    echo "  git push"
    echo "The release packages are built by GitHub Actions when a v$new tag is pushed (for"
    echo "example by publishing a GitHub Release for v$new), or by hand: Actions > Release."
}

case "${1:-check}" in
    check) cmd_check ;;
    bump | release)
        cmd_set "$@"
        ;;
    -h | --help | help) sed -n '2,/^set -e/p' "$0" | sed '$d; s/^# \{0,1\}//' ;;
    *) die "unknown command $1 (use check, bump or release)" ;;
esac
