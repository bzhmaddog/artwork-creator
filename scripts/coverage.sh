#!/bin/sh
# Coverage of new code, as the SonarQube Cloud quality gate measures it: the share of
# the lines added or changed since the branch left main that the tests run.
#
# Usage: coverage.sh [MIN]
#   Runs the Rust tests under cargo-llvm-cov (install it with `make dev-setup`) and the
#   script and Python SDK tests under Python's coverage from .venv (scripts/python-coverage.sh,
#   shared with the Sonar workflow), writes target/lcov.info and
#   target/python-lcov.info, then fails when new code is covered below MIN percent (default 80,
#   the gate's threshold). Files Sonar leaves out of coverage (sonar.exclusions and
#   sonar.coverage.exclusions in sonar-project.properties) are left out here too.
set -e

MIN=${1:-80}
LCOV=target/lcov.info
PY_LCOV=target/python-lcov.info

if ! cargo llvm-cov --version >/dev/null 2>&1; then
    echo "coverage: cargo-llvm-cov is not installed; run 'make dev-setup'." >&2
    exit 1
fi
if ! .venv/bin/python3 -m coverage --version >/dev/null 2>&1; then
    echo "coverage: Python's coverage is not installed; run 'make dev-setup'." >&2
    exit 1
fi

# Build progress shown (the first, instrumented build is long); tests as dots.
cargo llvm-cov --workspace --locked --lcov --output-path $LCOV -- -q
# The same Python measure as the Sonar workflow (test files included, as Sonar counts them).
PYTHON=.venv/bin/python3 sh scripts/python-coverage.sh
.venv/bin/python3 -m coverage lcov --data-file=target/.coverage -q -o $PY_LCOV

# Where the branch left main (origin/main when there is one).
BASE_BRANCH=main
git rev-parse -q --verify origin/main >/dev/null && BASE_BRANCH=origin/main
BASE=$(git merge-base HEAD $BASE_BRANCH)

# Sonar's exclusion globs, ** made into * (which also crosses / in a shell pattern).
EXCLUDED=$(sed -n 's/^sonar\.\(coverage\.\)\{0,1\}exclusions=//p' sonar-project.properties |
    tr ',' '\n' | sed 's|\*\*/|*|g')

# The new lines: "file line" for each line added since BASE (committed, staged or not).
NEW=$(git diff -U0 --no-color "$BASE" -- '*.rs' '*.py' | awk '
    /^\+\+\+ / { file = substr($2, 3); next }
    /^@@/ {
        split($3, a, ","); start = substr(a[1], 2); n = (a[2] == "" ? 1 : a[2])
        for (i = 0; i < n; i++) print file, start + i
    }')

# Each new line the coverage report knows (lines with code), covered or not.
REPORT=$(printf '%s\n' "$NEW" | awk -v root="$(pwd)/" -v excluded="$EXCLUDED" '
    function skip(f,   i, n, pats, re) {
        n = split(excluded, pats, "\n")
        for (i = 1; i <= n; i++) {
            if (pats[i] == "") continue
            re = pats[i]; gsub(/\./, "\\.", re); gsub(/\*/, ".*", re)
            if (f ~ ("^" re "$")) return 1
        }
        return 0
    }
    FILENAME == "-" { if (!skip($1)) want[$1 " " $2] = 1; next }
    /^SF:/ { file = substr($0, 4); sub(root, "", file); next }
    /^DA:/ {
        split(substr($0, 4), d, ",")
        if ((file " " d[1]) in want) { total++; if (d[2] > 0) hit++; else miss[file]++ }
    }
    END {
        for (f in miss) printf "  %4d uncovered  %s\n", miss[f], f
        printf "TOTAL %d %d\n", total, hit
    }' - $LCOV $PY_LCOV)

TOTAL=$(printf '%s\n' "$REPORT" | awk '/^TOTAL/ { print $2 }')
HIT=$(printf '%s\n' "$REPORT" | awk '/^TOTAL/ { print $3 }')
if [ "$TOTAL" = 0 ]; then
    echo "coverage: no new lines to cover"
    exit 0
fi
PCT=$(awk -v h="$HIT" -v t="$TOTAL" 'BEGIN { printf "%.1f", 100 * h / t }')
echo "coverage: $PCT% of new code covered ($HIT of $TOTAL lines since $BASE_BRANCH; gate $MIN%)"
if awk -v p="$PCT" -v m="$MIN" 'BEGIN { exit !(p < m) }'; then
    printf '%s\n' "$REPORT" | grep -v '^TOTAL' | sort -rn >&2
    echo "coverage: below $MIN%; add tests for the files above." >&2
    exit 1
fi
