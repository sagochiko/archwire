#!/bin/sh
# Runs the diagram checker on the diagrams made from generated definitions. Write them first:
#
#     ARCHWIRE_GEN_OUT=out/gen cargo test -p archwire-core generated
#     sh tools/check_generated.sh out/gen
#
# Every <number>.drawio in the folder is checked with tools/geocheck.py, with no known findings.
# Exit status: 0 if every diagram passes; 1 if any diagram has a blocking finding (their names
# are listed) or the folder holds no diagram; 2 if the draw.io desktop CLI (`drawio`) is not on
# PATH. A diagram the checker could not read is listed apart: that says nothing about the
# diagram itself, and it also gives 1.
set -u
if [ "$#" -ne 1 ]; then
    echo "usage: sh tools/check_generated.sh <folder>" >&2
    exit 2
fi
dir=$1
here=$(cd "$(dirname "$0")" && pwd)

if ! command -v drawio >/dev/null 2>&1; then
    echo "check_generated: the draw.io desktop CLI 'drawio' is not on PATH; install draw.io desktop and put its command-line tool on PATH" >&2
    exit 2
fi

checked=0
blocked=""
unread=""
for d in "$dir"/*.drawio; do
    [ -e "$d" ] || continue
    name=$(basename "$d" .drawio)
    checked=$((checked + 1))
    echo "== $name"
    python3 "$here/geocheck.py" "$d"
    status=$?
    if [ "$status" -eq 1 ]; then
        blocked="$blocked $name"
    elif [ "$status" -ne 0 ]; then
        echo "check_generated: the checker could not read $name (status $status); this says nothing about the diagram itself" >&2
        unread="$unread $name"
    fi
done

if [ "$checked" -eq 0 ]; then
    echo "check_generated: no .drawio file in $dir" >&2
    exit 1
fi
if [ -n "$unread" ]; then
    echo "check_generated: could not read:$unread"
fi
if [ -n "$blocked" ]; then
    echo "check_generated: blocking findings in:$blocked"
fi
if [ -n "$blocked" ] || [ -n "$unread" ]; then
    exit 1
fi
echo "check_generated: all $checked diagrams passed"
