#!/usr/bin/env python3
"""Counts geometry faults in a .drawio diagram as draw.io actually renders it.

    python3 tools/geocheck.py diagram.drawio [--json] [--known known_findings.txt]

The diagram is exported to SVG with the draw.io desktop CLI (`drawio` on PATH). Line
routes come from the rendered paths and text boxes from the pixels of the rendered labels,
so the counts describe the picture, not the coordinates written in the file.

Only the first page is checked. One line per kind is printed, always in the same order,
as `<kind>: <count>`, each followed by its findings indented by four spaces.

The blocking kinds are line_over_text, line_over_icon, line_overlap, icon_overlap (two icon
boxes, each shrunk by 2px, overlap), text_overlap (two text boxes, each shrunk by 1px, overlap;
icon labels, line labels and frame names all count) and outside_frame (an icon's box or its
label's box sticks out of the frame that is its parent). The other kinds are reported but do
not fail.

With `--known`, findings listed in that file are printed with ` (known)` and do not fail. Each
line of the file is `<case> <kind> <finding>`, where the case is the name of the folder that holds
the diagram; text from `#` to the end of a line is a comment. A listed finding for this case that
is no longer found is printed as `stale known finding: <case> <kind> <finding>` and fails, so the
list does not go out of date.

Exit status: 1 if a blocking kind has a finding that is not in the known list, or a known finding
is stale; 2 if the checker could not read the diagram or the known list: the file or the rendering
is unreadable, `drawio` or Pillow is missing, or the rendering yields fewer text boxes, lines or
icon boxes than the file has labelled cells, edges or icons (the missing ids are printed); else 0.

Deliberately not counted: a line through the blank margin of an icon's 60x60 box (the icon's
box is shrunk by 2px), a line over the icon at either of its own ends, and a line over its
own label.

    python3 tools/geocheck.py --self-test

checks the read-coverage comparison, the overlap and frame checks and the known list without draw.io.
"""
import base64
import io
import json
import re
import shutil
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET
from collections import defaultdict
from pathlib import Path

SVG_NS = "{http://www.w3.org/2000/svg}"
XLINK = "{http://www.w3.org/1999/xlink}href"

KINDS = [
    "line_over_text",
    "line_over_icon",
    "line_overlap",
    "frame_over_text",
    "line_crossing",
    "port_position",
    "icon_overlap",
    "text_overlap",
    "outside_frame",
]
BLOCKING = {"line_over_text", "line_over_icon", "line_overlap", "icon_overlap", "text_overlap", "outside_frame"}


class CheckError(Exception):
    pass


class ReadError(CheckError):
    """The rendering was read, but not everything the file says should be there was found."""


def style_of(cell):
    out = {}
    for part in (cell.get("style") or "").split(";"):
        if "=" in part:
            k, v = part.split("=", 1)
            out[k] = v
    return out


def read_cells(drawio):
    """The cells of the first page, as attribute dicts.

    A cell wrapped in `<object>` (or `<UserObject>`) keeps its id and label on the wrapper
    and its style, geometry and parent on the inner `<mxCell>`; the two are merged here.
    """
    root = ET.parse(drawio).getroot()
    if root.tag == "mxfile":
        diagram = root.find("diagram")
        if diagram is None:
            raise CheckError("the file has no <diagram> page")
        model = diagram.find("mxGraphModel")
        if model is None:
            raise CheckError("the first page is compressed or empty; save the file uncompressed")
    elif root.tag == "mxGraphModel":
        model = root
    else:
        raise CheckError(f"not a draw.io file (root element <{root.tag}>)")
    cells = {}
    for el in model.iter():
        if el.tag in ("object", "UserObject"):
            inner = el.find("mxCell")
            attrs = dict(inner.attrib) if inner is not None else {}
            attrs["id"] = el.get("id")
            attrs["value"] = el.get("label", "")
            cells[attrs["id"]] = attrs
        elif el.tag == "mxCell" and el.get("id") is not None:
            cells[el.get("id")] = dict(el.attrib)
    return cells


def export_svg(drawio, out_dir):
    if shutil.which("drawio") is None:
        raise CheckError("the draw.io desktop CLI `drawio` is not on PATH")
    out = Path(out_dir) / "d.svg"
    r = subprocess.run(["drawio", "-x", "-f", "svg", "-p", "1", "-o", str(out), str(drawio)],
                       capture_output=True, text=True, timeout=120)
    if not out.exists():
        raise CheckError(f"drawio could not export {drawio}: {r.stderr.strip() or r.stdout.strip()}")
    return out


NUMBER = r"[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?"


def polyline(d):
    """The points of a line drawn with straight commands only (M L H V Z, absolute or relative).

    Any other command (a curve or an arc) gives [], so the line counts as not read: a line in
    these diagrams is made of straight segments, and guessing a curve's shape would hide a change.
    """
    pts, cur, start = [], (0.0, 0.0), (0.0, 0.0)
    for cmd, args in re.findall(r"([A-Za-z])([^A-Za-z]*)", d):
        nums = [float(n) for n in re.findall(NUMBER, args)]
        op = cmd.upper()
        rel = cmd.islower()
        if op in ("M", "L"):
            if len(nums) < 2 or len(nums) % 2:
                return []
            for k in range(0, len(nums), 2):
                x, y = nums[k], nums[k + 1]
                cur = (cur[0] + x, cur[1] + y) if rel else (x, y)
                if op == "M" and k == 0:
                    start = cur
                pts.append(cur)
        elif op in ("H", "V"):
            if not nums:
                return []
            for n in nums:
                if op == "H":
                    cur = (cur[0] + n if rel else n, cur[1])
                else:
                    cur = (cur[0], cur[1] + n if rel else n)
                pts.append(cur)
        elif op == "Z":
            cur = start
            pts.append(cur)
        else:
            return []
    return pts


def shape_points(d):
    """Absolute points of a shape's outline, enough for its bounding box; curves are skipped."""
    return [(float(x), float(y)) for x, y in re.findall(r"[ML]\s*([-\d.]+)\s+([-\d.]+)", d)]


def glyph_box(img):
    """The box of the label's opaque pixels: where the text is actually drawn."""
    try:
        from PIL import Image
    except ImportError:
        raise CheckError("Pillow is not installed for python3; install it with `python3 -m pip install Pillow`")
    x, y = float(img.get("x")), float(img.get("y"))
    w, h = float(img.get("width")), float(img.get("height"))
    data = img.get(XLINK).split(",", 1)[1]
    im = Image.open(io.BytesIO(base64.b64decode(data))).convert("RGBA")
    bb = im.getchannel("A").point(lambda a: 255 if a > 40 else 0).getbbox()
    if not bb:
        return None
    sx, sy = w / im.width, h / im.height
    return (x + bb[0] * sx, y + bb[1] * sy, x + bb[2] * sx, y + bb[3] * sy)


def read_svg(svg, edge_ids=()):
    root = ET.parse(svg).getroot()
    edges, labels, rects = {}, {}, {}
    for g in root.iter(SVG_NS + "g"):
        cid = g.get("data-cell-id")
        if not cid or cid in ("0", "1"):
            continue
        # Only this cell's own drawing: direct child groups without a cell id.
        own = [c for c in g if c.tag == SVG_NS + "g" and not c.get("data-cell-id")]
        own_paths = [p for c in own for p in c.iter(SVG_NS + "path")]
        own_imgs = [i for c in own for i in c.iter(SVG_NS + "image")
                    if (i.get(XLINK) or "").startswith("data:image/png")]
        # A frame is drawn as a <path> or, like the AZ frame, a <rect>; the first shape is its outline.
        shapes = [el for c in own for el in c.iter() if el.tag in (SVG_NS + "path", SVG_NS + "rect")]
        if shapes and cid not in edge_ids and shapes[0].tag == SVG_NS + "rect":
            r = shapes[0]
            x, y = float(r.get("x", 0)), float(r.get("y", 0))
            rects[cid] = (x, y, x + float(r.get("width", 0)), y + float(r.get("height", 0)))
        elif own_paths:
            first = own_paths[0].get("d", "")
            if cid in edge_ids:
                edges[cid] = polyline(first)
            else:
                pts = shape_points(first)
                if pts:
                    xs, ys = [p[0] for p in pts], [p[1] for p in pts]
                    rects[cid] = (min(xs), min(ys), max(xs), max(ys))
        for img in own_imgs[-1:]:
            box = glyph_box(img)
            if box:
                labels[cid] = box
    return edges, labels, rects


def seg_hits_box(p, q, box, pad=0.0):
    (x1, y1), (x2, y2) = p, q
    bx1, by1, bx2, by2 = box[0] - pad, box[1] - pad, box[2] + pad, box[3] + pad
    if abs(y1 - y2) < 0.5:  # horizontal
        return by1 < y1 < by2 and max(x1, x2) > bx1 and min(x1, x2) < bx2
    if abs(x1 - x2) < 0.5:  # vertical
        return bx1 < x1 < bx2 and max(y1, y2) > by1 and min(y1, y2) < by2
    return False


def segments(pts):
    return list(zip(pts, pts[1:]))


def cross(s, t):
    (a, b), (c, d) = s, t
    sh, th = abs(a[1] - b[1]) < 0.5, abs(c[1] - d[1]) < 0.5
    if sh == th:
        return False
    h, v = (s, t) if sh else (t, s)
    y = h[0][1]
    x = v[0][0]
    return (min(h[0][0], h[1][0]) + 1 < x < max(h[0][0], h[1][0]) - 1 and
            min(v[0][1], v[1][1]) + 1 < y < max(v[0][1], v[1][1]) - 1)


def overlap(s, t):
    (a, b), (c, d) = s, t
    if abs(a[1] - b[1]) < 0.5 and abs(c[1] - d[1]) < 0.5 and abs(a[1] - c[1]) < 2:
        lo, hi = max(min(a[0], b[0]), min(c[0], d[0])), min(max(a[0], b[0]), max(c[0], d[0]))
        return hi - lo > 4
    if abs(a[0] - b[0]) < 0.5 and abs(c[0] - d[0]) < 0.5 and abs(a[0] - c[0]) < 2:
        lo, hi = max(min(a[1], b[1]), min(c[1], d[1])), min(max(a[1], b[1]), max(c[1], d[1]))
        return hi - lo > 4
    return False


def frame_lines(box):
    x1, y1, x2, y2 = box
    return [((x1, y1), (x2, y1)), ((x2, y1), (x2, y2)), ((x2, y2), (x1, y2)), ((x1, y2), (x1, y1))]


def side_of(x, y):
    x, y = float(x), float(y)
    if y == 0:
        return "top"
    if y == 1:
        return "bottom"
    if x == 0:
        return "left"
    if x == 1:
        return "right"
    return None


def port_rule(cells):
    """Where lines meet one side of an icon: one line at 0.5, two at 0.25/0.75, three at 0.25/0.5/0.75."""
    use = defaultdict(list)
    for c in cells.values():
        if c.get("edge") != "1":
            continue
        st = style_of(c)
        for side, key in (("exit", "source"), ("entry", "target")):
            x, y = st.get(side + "X"), st.get(side + "Y")
            if x is None or y is None:
                continue
            s = side_of(x, y)
            if s is None:
                continue
            pos = float(x) if s in ("top", "bottom") else float(y)
            use[(c.get(key), s)].append((pos, c.get("id")))
    want = {1: [0.5], 2: [0.25, 0.75], 3: [0.25, 0.5, 0.75]}
    bad = []
    for (cell, s), items in sorted(use.items()):
        got = sorted(p for p, _ in items)
        w = want.get(len(got))
        if w and got != w:
            bad.append(f"{cell} {s} side has {len(got)} lines at {got} (expected {w})")
    return bad


def shrink(box, by):
    return (box[0] + by, box[1] + by, box[2] - by, box[3] - by)


def boxes_overlap(a, b):
    return a[0] < b[2] and b[0] < a[2] and a[1] < b[3] and b[1] < a[3]


def overlapping_pairs(boxes, by):
    """Pairs of ids, in id order, whose boxes still overlap after each is shrunk by `by` px."""
    ids = sorted(boxes)
    small = {cid: shrink(boxes[cid], by) for cid in ids}
    return [(a, b) for i, a in enumerate(ids) for b in ids[i + 1:] if boxes_overlap(small[a], small[b])]


def icon_overlaps(icons):
    """Icons drawn over each other; touching within 2px is not counted."""
    return [f"{a} and {b}" for a, b in overlapping_pairs(icons, 2)]


def text_overlaps(labels):
    """Texts drawn over each other (icon labels, line labels and frame names); touching within 1px is not counted."""
    return [f"the text of {a} and the text of {b}" for a, b in overlapping_pairs(labels, 1)]


def inside(box, frame):
    return frame[0] <= box[0] and frame[1] <= box[1] and box[2] <= frame[2] and box[3] <= frame[3]


def outside_frames(cells, icons, labels, rects):
    """Icons whose box or label box sticks out of the frame that is their parent cell."""
    bad = []
    for cid in sorted(icons):
        cell = cells.get(cid) or {}
        frame = rects.get(cell.get("parent"))
        if frame is None:
            continue
        boxes = [icons[cid]] + ([labels[cid]] if cid in labels else [])
        if not all(inside(b, frame) for b in boxes):
            bad.append(f"{cid} outside {cell.get('parent')}")
    return bad


def read_known(path):
    """The known findings, as `(case, kind, finding)`; each line is `<case> <kind> <finding>`.

    Text from `#` to the end of a line is a comment; blank lines are skipped.
    """
    known = set()
    for n, raw in enumerate(Path(path).read_text(encoding="utf-8").splitlines(), 1):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        parts = line.split(None, 2)
        if len(parts) != 3 or parts[1] not in KINDS:
            raise CheckError(f"{path}:{n}: expected `<case> <kind> <finding>` with a known kind, got {raw!r}")
        known.add(tuple(parts))
    return known


def expected_parts(cells):
    """What the file says the rendering must contain: labelled cells, edges and icons."""
    groups = {cid for cid, c in cells.items() if c.get("vertex") == "1" and
              ("group" in (c.get("style") or "") or any(o.get("parent") == cid for o in cells.values()))}
    labelled = {cid for cid, c in cells.items()
                if (c.get("vertex") == "1" or c.get("edge") == "1") and (c.get("value") or "").strip()}
    edges = {cid for cid, c in cells.items() if c.get("edge") == "1"}
    icons = {cid for cid, c in cells.items() if c.get("vertex") == "1" and cid not in groups}
    return groups, labelled, edges, icons


def has_area(box):
    return len(box) == 4 and box[2] - box[0] > 0 and box[3] - box[1] > 0


def missing_reads(cells, labels, edges, icons):
    """Problems, one per kind, where fewer parts were read from the rendering than the file has.

    A text box or an icon box counts as read only if it has a width and a height; a line only
    if at least two of its points were read.
    """
    _, want_labels, want_edges, want_icons = expected_parts(cells)
    read_labels = {cid for cid, box in labels.items() if has_area(box)}
    read_edges = {cid for cid, pts in edges.items() if len(pts) >= 2}
    read_icons = {cid for cid, box in icons.items() if has_area(box)}
    problems = []
    for kind, want, got in (("text boxes", want_labels, read_labels), ("lines", want_edges, read_edges),
                            ("icon boxes", want_icons, read_icons)):
        missing = sorted(want - got)
        if missing:
            problems.append(f"read {len(want) - len(missing)} of {len(want)} {kind}; not read: {', '.join(missing)}")
    return problems


def check(drawio):
    cells = read_cells(drawio)
    with tempfile.TemporaryDirectory() as tmp:
        svg = export_svg(drawio, tmp)
        edge_ids = {cid for cid, c in cells.items() if c.get("edge") == "1"}
        edges, labels, rects = read_svg(svg, edge_ids)
    if not any(c.get("vertex") == "1" or c.get("edge") == "1" for c in cells.values()):
        raise CheckError("the first page has no shapes or lines")
    groups = expected_parts(cells)[0]
    # A frame with no children (an empty dashed AZ, say) is not an icon.
    icons = {cid: r for cid, r in rects.items() if cid not in groups and cells.get(cid) is not None
             and cells[cid].get("vertex") == "1"
             and r[2] - r[0] <= 120 and r[3] - r[1] <= 120}
    problems = missing_reads(cells, labels, edges, icons)
    if problems:
        raise ReadError("could not read the rendered diagram: " + "; ".join(problems))

    def kind_of_text(cid):
        if cid in groups:
            return "frame name"
        if cells.get(cid) is not None and cells[cid].get("edge") == "1":
            return "line label"
        return "icon label"

    found = defaultdict(list)
    for eid, pts in edges.items():
        e = cells.get(eid)
        ends = {e.get("source"), e.get("target")} if e is not None else set()
        for s in segments(pts):
            for cid, box in labels.items():
                if cid == eid:
                    continue
                if seg_hits_box(*s, box, pad=1):
                    found["line_over_text"].append(f"{eid} over the {kind_of_text(cid)} of {cid}")
            for cid, box in icons.items():
                if cid in ends:
                    continue
                if seg_hits_box(*s, box, pad=-2):
                    found["line_over_icon"].append(f"{eid} over {cid}")
    for gid in groups:
        if gid not in rects:
            continue
        for fl in frame_lines(rects[gid]):
            for cid, box in labels.items():
                if cid == gid or (cells.get(cid) is not None and cells[cid].get("parent") == gid and cid in groups):
                    continue
                if seg_hits_box(*fl, box, pad=-1):
                    found["frame_over_text"].append(f"{gid} border over the {kind_of_text(cid)} of {cid}")
    ids = sorted(edges)
    for i, a in enumerate(ids):
        for b in ids[i + 1:]:
            for s in segments(edges[a]):
                for t in segments(edges[b]):
                    if cross(s, t):
                        found["line_crossing"].append(f"{a} and {b}")
                    if overlap(s, t):
                        found["line_overlap"].append(f"{a} and {b}")
    found["port_position"] = port_rule(cells)
    found["icon_overlap"] = icon_overlaps(icons)
    found["text_overlap"] = text_overlaps(labels)
    found["outside_frame"] = outside_frames(cells, icons, labels, rects)
    return {k: sorted(set(found.get(k, []))) for k in KINDS}


def self_test():
    """The read-coverage comparison on made-up cells: short reads give status 2 and name the ids."""
    cells = {
        "frame": {"id": "frame", "vertex": "1", "value": "VPC", "style": "shape=mxgraph.aws4.group"},
        "a": {"id": "a", "vertex": "1", "value": "A", "parent": "frame"},
        "b": {"id": "b", "vertex": "1", "value": "", "parent": "frame"},
        "e1": {"id": "e1", "edge": "1", "value": "HTTP", "source": "a", "target": "b"},
        "e2": {"id": "e2", "edge": "1", "value": "", "source": "b", "target": "a"},
    }
    box = (0.0, 0.0, 10.0, 10.0)
    full_labels = {"frame": box, "a": box, "e1": box}
    full_edges = {"e1": [(0.0, 0.0), (5.0, 0.0)], "e2": [(0.0, 5.0), (5.0, 5.0)]}
    full_icons = {"a": box, "b": box}
    ok = missing_reads(cells, full_labels, full_edges, full_icons)
    assert ok == [], ok
    # Every line id is there, but one line has no points or a single point: not read.
    for pts in ([], [(0.0, 0.0)]):
        got = missing_reads(cells, full_labels, {"e1": pts, "e2": full_edges["e2"]}, full_icons)
        assert got == ["read 1 of 2 lines; not read: e1"], (pts, got)
    # A box with no width or no height is not read.
    flat = missing_reads(cells, {**full_labels, "a": (0.0, 0.0, 10.0, 0.0)}, full_edges, {**full_icons, "b": (3.0, 0.0, 3.0, 9.0)})
    assert flat == ["read 2 of 3 text boxes; not read: a", "read 1 of 2 icon boxes; not read: b"], flat
    # Path commands: straight ones, absolute or relative, are read; a curve is not.
    assert polyline("m 10 20 l 30 0 v 5 h -4") == [(10, 20), (40, 20), (40, 25), (36, 25)], polyline("m 10 20 l 30 0 v 5 h -4")
    assert polyline("M 10 20 L 40 20 V 25 H 36") == [(10, 20), (40, 20), (40, 25), (36, 25)]
    assert polyline("M 10 20 C 20 20 30 30 40 40") == []
    short = missing_reads(cells, {"frame": box}, {"e2": full_edges["e2"]}, {"b": box})
    assert short == ["read 1 of 3 text boxes; not read: a, e1",
                     "read 1 of 2 lines; not read: e1",
                     "read 1 of 2 icon boxes; not read: a"], short
    nothing = missing_reads(cells, {}, {}, {})
    assert len(nothing) == 3 and "not read: a, e1, frame" in nothing[0], nothing
    # Through main: a short read exits 2 and names the ids; a full read with no findings exits 0.
    import contextlib
    global check
    real = check

    def fake(problems):
        def run(_path):
            if problems:
                raise ReadError("could not read the rendered diagram: " + "; ".join(problems))
            return {k: [] for k in KINDS}
        return run
    def run_main(problems):
        err, out = io.StringIO(), io.StringIO()
        global check
        check = fake(problems)
        with contextlib.redirect_stderr(err), contextlib.redirect_stdout(out):
            return main(["geocheck.py", "x.drawio"]), err.getvalue()
    try:
        status, err = run_main(short)
        assert status == 2 and "not read: a, e1" in err, (status, err)
        for pts in ([], [(0.0, 0.0)]):
            status, err = run_main(missing_reads(cells, full_labels, {"e1": pts, "e2": full_edges["e2"]}, full_icons))
            assert status == 2 and "lines; not read: e1" in err, (pts, status, err)
        err, out = io.StringIO(), io.StringIO()
        check = fake(ok)
        with contextlib.redirect_stderr(err), contextlib.redirect_stdout(out):
            status = main(["geocheck.py", "x.drawio"])
        assert status == 0, status
    finally:
        check = real
    # Icon boxes shrunk by 2px, text boxes shrunk by 1px, and parts outside their parent frame.
    got = icon_overlaps({"a": (0, 0, 60, 60), "b": (50, 0, 110, 60), "c": (200, 0, 260, 60)})
    assert got == ["a and b"], got
    got = icon_overlaps({"a": (0, 0, 60, 60), "b": (59, 0, 119, 60)})
    assert got == [], got
    got = text_overlaps({"x": (0, 70, 80, 90), "y": (70, 75, 150, 95), "z": (0, 200, 10, 210)})
    assert got == ["the text of x and the text of y"], got
    frame_cells = {
        "r": {"id": "r", "vertex": "1", "value": "Frame", "style": "group"},
        "in": {"id": "in", "vertex": "1", "value": "In", "parent": "r"},
        "out": {"id": "out", "vertex": "1", "value": "", "parent": "r"},
    }
    got = outside_frames(frame_cells, {"in": (10, 10, 70, 70), "out": (250, 10, 310, 70)},
                         {"in": (0, 74, 80, 94)}, {"r": (0, 0, 300, 200)})
    assert got == ["out outside r"], got
    # Through main with a list of known findings: the case name is the diagram's folder name.
    finding = "the text of a and the text of b"

    def fake_found(_path):
        out = {k: [] for k in KINDS}
        out["text_overlap"] = [finding]
        return out

    def run_known(fake_check, known_text):
        global check
        with tempfile.TemporaryDirectory() as tmp:
            known = Path(tmp) / "known.txt"
            known.write_text(known_text)
            err, out = io.StringIO(), io.StringIO()
            check = fake_check
            try:
                with contextlib.redirect_stderr(err), contextlib.redirect_stdout(out):
                    status = main(["geocheck.py", "cases/web-x/expected.drawio", "--known", str(known)])
            finally:
                check = real
            return status, out.getvalue() + err.getvalue()
    line = f"web-x text_overlap {finding}  # a comment\n"
    status, text = run_known(fake_found, "# known findings\n" + line)
    assert status == 0 and f"{finding} (known)" in text, (status, text)
    status, text = run_known(fake_found, "")
    assert status == 1, (status, text)
    status, text = run_known(fake(ok), line)
    assert status == 1 and f"stale known finding: web-x text_overlap {finding}" in text, (status, text)
    # A finding listed for another case does not count for this one.
    status, text = run_known(fake_found, f"web-y text_overlap {finding}\n")
    assert status == 1 and "stale" not in text, (status, text)
    print("self-test: counts match -> exit 0; line e1 with 0 or 1 points -> exit 2 naming e1; flat boxes -> " + " | ".join(flat))
    print("self-test: short reads -> exit 2 with: " + " | ".join(short))
    print("self-test: m/l/h/v paths read, a curve is not read")
    print("self-test: icons overlapping past 2px, texts past 1px, and parts outside their frame are found; "
          "known findings pass, unlisted ones fail, stale ones fail")
    print("self-test: passed")
    return 0


def main(argv):
    if "--self-test" in argv:
        return self_test()
    args, known_path, rest = [], None, iter(argv[1:])
    for a in rest:
        if a == "--known":
            known_path = next(rest, None)
            if known_path is None:
                print(__doc__.strip(), file=sys.stderr)
                return 2
        elif not a.startswith("--"):
            args.append(a)
    if len(args) != 1:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    diagram = Path(args[0])
    case = diagram.parent.name
    try:
        known = {(k, f) for c, k, f in read_known(known_path) if c == case} if known_path else set()
        result = check(diagram)
    except (CheckError, ET.ParseError, OSError, subprocess.TimeoutExpired) as e:
        print(f"geocheck: {e}", file=sys.stderr)
        return 2
    if "--json" in argv:
        print(json.dumps(result, ensure_ascii=False, indent=1))
    else:
        for k in KINDS:
            print(f"{k}: {len(result[k])}")
            for x in result[k]:
                print("   ", x + (" (known)" if (k, x) in known else ""))
    stale = sorted(kf for kf in known if kf[1] not in result.get(kf[0], []))
    for k, f in stale:
        print(f"stale known finding: {case} {k} {f}")
    blocking = any(x for k in BLOCKING for x in result[k] if (k, x) not in known)
    return 1 if blocking or stale else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
