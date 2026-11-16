#!/bin/sh
# Runs the diagram checker on the diagrams made from generated definitions. Write them first:
#
#     ARCHWIRE_GEN_OUT=out/gen cargo test -p archwire-core -- generated_diagrams_compile_for_the_checker arranged_diagrams_for_the_checker
#     sh tools/check_generated.sh out/gen
#
# Every <name>.yaml in the folder must have its <name>.drawio, and there must be exactly
# 80 of them (40 made by compile, 40 by the layout for a VPC), so a definition that gave no diagram is not passed over. Each diagram is checked
# with tools/geocheck.py, with no known findings.
# Exit status: 0 if every diagram passes; 1 if any diagram has a blocking finding (their names
# are listed), a definition has no diagram, or the count is not 80; 2 if the draw.io desktop CLI
# (`drawio`) is not on PATH. A diagram the checker could not read is listed apart: that says
# nothing about the diagram itself, and it also gives 1.
set -u
if [ "$#" -ne 1 ]; then
    echo "usage: sh tools/check_generated.sh <folder>" >&2
    exit 2
fi
dir=$1
here=$(cd "$(dirname "$0")" && pwd)
expected=80

if ! command -v drawio >/dev/null 2>&1; then
    echo "check_generated: the draw.io desktop CLI 'drawio' is not on PATH; install draw.io desktop and put its command-line tool on PATH" >&2
    exit 2
fi

checked=0
blocked=""
unread=""
missing=""
for y in "$dir"/*.yaml; do
    [ -e "$y" ] || continue
    name=$(basename "$y" .yaml)
    d="$dir/$name.drawio"
    checked=$((checked + 1))
    if [ ! -e "$d" ]; then
        echo "check_generated: $name.yaml has no $name.drawio" >&2
        missing="$missing $name"
        continue
    fi
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

count_wrong=""
if [ "$checked" -ne "$expected" ]; then
    echo "check_generated: $dir holds $checked definitions; expected $expected" >&2
    count_wrong=1
fi
for d in "$dir"/*.drawio; do
    [ -e "$d" ] || continue
    if [ ! -e "${d%.drawio}.yaml" ]; then
        echo "check_generated: $(basename "$d") has no definition beside it" >&2
        count_wrong=1
    fi
done
if [ -n "$missing" ]; then
    echo "check_generated: no diagram for:$missing"
fi
if [ -n "$unread" ]; then
    echo "check_generated: could not read:$unread"
fi
if [ -n "$blocked" ]; then
    echo "check_generated: blocking findings in:$blocked"
fi
if [ -n "$blocked" ] || [ -n "$unread" ] || [ -n "$missing" ] || [ -n "$count_wrong" ]; then
    exit 1
fi
echo "check_generated: all $checked diagrams passed"
