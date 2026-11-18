# Contributing

## What you need

- A recent stable Rust toolchain, for building and for the tests
- Python 3, for the diagram checker
- For the checks that render diagrams, the draw.io desktop command line (`drawio`) on PATH and Python with Pillow

## Tests

```sh
cargo test --workspace
python3 tools/geocheck.py --self-test
```

`cargo test --workspace` compiles every definition under `tests/cases` and compares the output with the expected files byte for byte. `python3 tools/geocheck.py --self-test` runs the self-test of the diagram checker and needs neither draw.io nor a diagram.

When a change is meant to change the output, write the expected files again, check them, and look at the images before you commit:

```sh
ARCHWIRE_BLESS=1 cargo test -p archwire --test cases
sh tools/check_cases.sh
```

Continuous integration runs `cargo test --workspace` and the checker's self-test on Linux and macOS. It does not render diagrams, because that needs the draw.io desktop app, so the rendered-diagram check is run locally.

## Checking the diagrams

`sh tools/check_cases.sh` checks each expected diagram as draw.io renders it and writes images to `out/`. It exits with 0 when every case passes, 1 when a case fails the checker or cannot be read or rendered, and 2 when `drawio` is not on PATH. The output tells the two kinds of failure apart: a case with a blocking fault in the diagram, or a case the checker could not read, which says nothing about the diagram. `tools/geocheck.py` exits with 1 when a diagram has a blocking fault (a line over text, a line over an icon, two lines on top of each other, two icons or two texts overlapping, or an icon or its text outside its frame), and with 2 when it could not read the rendering, which says nothing about the diagram.

`tools/known_findings.txt` lists findings that are known and do not fail `check_cases.sh`, one per line as `<case> <kind> <finding>`. A finding in the list that no longer appears fails the check, so the list cannot go stale. The list is empty today.

## Generated definitions

The layout is also tested on definitions made by a program. Without a VPC they vary the number of branches, shared targets, auxiliary flows, named parts, several CloudFront distributions and the language; `cargo test --workspace` lays out every one of them and checks that each is laid out without `LAYOUT_FAILED` (no parts or names overlapping, every part in its frame) and that no line or label is reported as not drawn clear. With a VPC they vary the zones, the NAT gateways, the readers, the routes out and the side lines, and the definitions mixing a function in the VPC with other parts vary as well; `cargo test --workspace` draws the web definitions with two zones, and the rest are drawn by tests that run only when named, `generated_definitions_all_draw_cleanly` and `generated_mixed_definitions_all_draw_cleanly` with `-- --ignored`:

```sh
cargo test -p archwire-core generated_definitions_all_draw_cleanly -- --ignored
cargo test -p archwire-core generated_mixed_definitions_all_draw_cleanly -- --ignored
```

To check 160 of them as draw.io renders them (40 without a VPC, 40 with one, and 80 mixing a function in the VPC with other parts), write them out and run the checker:

```sh
ARCHWIRE_GEN_OUT=out/gen cargo test -p archwire-core -- generated_diagrams_compile_for_the_checker arranged_diagrams_for_the_checker mixed_diagrams_for_the_checker
sh tools/check_generated.sh out/gen
```

`check_generated.sh` exits with 0 when every diagram passes, 1 when a diagram has a blocking fault or is missing or the folder does not hold 160 definitions, and 2 when `drawio` is not on PATH.

## Reporting a problem

Open an [issue](https://github.com/sagochiko/archwire/issues) with:

- the smallest definition that shows the problem
- the command you ran
- the JSON report it printed, and the exit code
- what you expected instead
- for a drawing problem, an image of the diagram as draw.io shows it

The report's `versions` says which archwire and rules data you used.
