#!/bin/sh
# Runs the diagram checker on every acceptance case that has an expected diagram, and renders
# each one to out/<case>.png so a person can look at it. Exits non-zero if any case fails the
# checker or cannot be rendered. Needs the draw.io desktop CLI (`drawio`) and python3 with Pillow.
#
#     sh tools/check_cases.sh
set -u
cd "$(dirname "$0")/.." || exit 2

if ! command -v drawio >/dev/null 2>&1; then
    echo "check_cases: the draw.io desktop CLI 'drawio' is not on PATH; install draw.io desktop and put its command-line tool on PATH" >&2
    exit 2
fi

mkdir -p out
checked=0
failed=""
for expected in tests/cases/*/expected.drawio; do
    [ -e "$expected" ] || continue
    name=$(basename "$(dirname "$expected")")
    checked=$((checked + 1))
    echo "== $name"
    python3 tools/geocheck.py "$expected" --known tools/known_findings.txt
    status=$?
    if [ "$status" -eq 1 ]; then
        echo "check_cases: $name has a blocking fault in the diagram or a stale known finding"
        failed="$failed $name"
    elif [ "$status" -ne 0 ]; then
        echo "check_cases: the checker could not read $name (status $status); this says nothing about the diagram itself" >&2
        failed="$failed $name"
    fi
    rm -f "out/$name.png"
    if ! drawio -x -f png -o "out/$name.png" "$expected" >/dev/null 2>&1 || [ ! -s "out/$name.png" ]; then
        echo "check_cases: could not render $expected to out/$name.png" >&2
        failed="$failed $name"
    fi
done

if [ "$checked" -eq 0 ]; then
    echo "check_cases: no case under tests/cases has an expected.drawio" >&2
    exit 1
fi
if [ -n "$failed" ]; then
    echo "check_cases: failed:$failed"
    exit 1
fi
echo "check_cases: all $checked cases passed; images are in out/"
