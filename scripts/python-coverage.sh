#!/bin/sh
# Runs the Python tests (scripts/ and the plugin SDK's) under coverage, into the data
# file target/.coverage. Used by scripts/coverage.sh and the Sonar workflow, so both
# measure the same files: SonarQube Cloud counts test files as code to cover too, so
# they are measured, not left out.
#
#   PYTHON=.venv/bin/python3 sh scripts/python-coverage.sh
#
# PYTHON is a Python with coverage installed (python3 by default).
set -e

PYTHON=${PYTHON:-python3}
DATA=target/.coverage

mkdir -p target
rm -f "$DATA"
"$PYTHON" -m coverage run --data-file="$DATA" --source=scripts \
    -m unittest discover -s scripts
"$PYTHON" -m coverage run --data-file="$DATA" --append \
    --source=plugin-sdk/python \
    -m unittest discover -s plugin-sdk/python
