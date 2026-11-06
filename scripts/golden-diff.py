#!/usr/bin/env python3
"""Strict golden comparison: one pixel past the tolerance flags a scene.

The golden tests tolerate 0.2% of pixels over a per-channel difference of
12, which let "12 more models" become "17 more models" pass. This tool keeps
the per-channel tolerance (GPU drivers differ in the last bits of blending
and glyph coverage, so about 98k pixels differ by a few levels) but has no
count allowance: a scene with any pixel whose channel differs by more than
TOLERANCE needs its image looked at and its golden regenerated.

    scripts/golden-diff.py --golden <approved> --renders <rendered>
        [--diff-dir <dir>] [--summary <file>] [--newer-than <file>]
        [--skip '<glob>=<reason>' ...]

Compares every PNG by name in <approved> and <renders> and prints a Markdown
table (also appended to --summary). A golden with no render, a render with
no golden, a different size, or (with --newer-than) a render that predates
the marker file all fail; exit 1 on any failing scene. `--skip` leaves
scenes this host cannot render out of the comparison and says why in the
table (the capture capsule goldens are macOS-only). Standard library only.
Run `scripts/test-golden-diff.py` for its tests.
"""

import argparse
import fnmatch
import itertools
import multiprocessing
import os
import struct
import sys
import zlib
from pathlib import Path

TOLERANCE = 12
SIGNATURE = b"\x89PNG\r\n\x1a\n"
# Samples per pixel by PNG colour type; 8-bit only (Hex's renders are RGBA8).
CHANNELS = {0: 1, 2: 3, 4: 2, 6: 4}


class PngError(Exception):
    pass


def _add_bytes(a, b):
    """Per-byte (a + b) mod 256 of two equal-length byte strings."""
    n = len(a)
    if n == 0:
        return b""
    high = int.from_bytes(b"\x80" * n, "little")
    x = int.from_bytes(a, "little")
    y = int.from_bytes(b, "little")
    total = ((x & ~high) + (y & ~high)) ^ ((x ^ y) & high)
    return (total & ((1 << (8 * n)) - 1)).to_bytes(n, "little")


def _unfilter_row(kind, row, prev, bpp):
    """Reverse one scanline filter. `row` is the filtered bytes."""
    n = len(row)
    if kind == 0:
        return bytes(row)
    if kind == 2:
        return _add_bytes(row, prev)
    out = bytearray(n)
    if kind == 1:
        for c in range(bpp):
            sums = itertools.accumulate(row[c::bpp])
            out[c::bpp] = bytes(v & 255 for v in sums)
        return bytes(out)
    if kind == 3:
        for i in range(n):
            left = out[i - bpp] if i >= bpp else 0
            out[i] = (row[i] + ((left + prev[i]) >> 1)) & 255
        return bytes(out)
    if kind == 4:
        for i in range(n):
            if i >= bpp:
                a, c = out[i - bpp], prev[i - bpp]
            else:
                a = c = 0
            b = prev[i]
            p = a + b - c
            pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
            if pa <= pb and pa <= pc:
                pred = a
            elif pb <= pc:
                pred = b
            else:
                pred = c
            out[i] = (row[i] + pred) & 255
        return bytes(out)
    raise PngError(f"unknown scanline filter {kind}")


def read_png(path):
    """Return (width, height, rows) with every row as RGBA bytes."""
    data = Path(path).read_bytes()
    if data[:8] != SIGNATURE:
        raise PngError("not a PNG")
    pos, idat, header, palette = 8, [], None, None
    while pos + 8 <= len(data):
        length, kind = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + length]
        pos += 12 + length
        if kind == b"IHDR":
            header = struct.unpack(">IIBBBBB", body)
        elif kind == b"PLTE":
            palette = body
        elif kind == b"IDAT":
            idat.append(body)
        elif kind == b"IEND":
            break
    if header is None:
        raise PngError("no IHDR")
    width, height, depth, color, _, _, interlace = header
    if depth != 8 or interlace != 0 or (color not in CHANNELS and color != 3):
        raise PngError(
            f"unsupported PNG (depth {depth}, colour type {color}, interlace {interlace})"
        )
    samples = 1 if color == 3 else CHANNELS[color]
    stride = width * samples
    raw = zlib.decompress(b"".join(idat))
    if len(raw) != height * (stride + 1):
        raise PngError("image data has the wrong length")
    rows, prev = [], bytes(stride)
    for y in range(height):
        start = y * (stride + 1)
        prev = _unfilter_row(raw[start], raw[start + 1:start + 1 + stride], prev, samples)
        rows.append(_to_rgba(prev, color, palette))
    return width, height, rows


def _to_rgba(row, color, palette):
    if color == 6:
        return row
    n = len(row)
    if color == 2:
        out = bytearray(b"\xff" * (n // 3 * 4))
        for c in range(3):
            out[c::4] = row[c::3]
        return bytes(out)
    if color == 0:
        out = bytearray(b"\xff" * (n * 4))
        for c in range(3):
            out[c::4] = row
        return bytes(out)
    if color == 4:
        out = bytearray(n // 2 * 4)
        for c in range(3):
            out[c::4] = row[0::2]
        out[3::4] = row[1::2]
        return bytes(out)
    if palette is None:
        raise PngError("palette PNG without PLTE")
    out = bytearray(n * 4)
    for i, index in enumerate(row):
        out[i * 4:i * 4 + 3] = palette[index * 3:index * 3 + 3]
        out[i * 4 + 3] = 255
    return bytes(out)


def write_png(path, width, height, rows):
    """Write RGBA8 rows (unfiltered) as a PNG."""
    raw = b"".join(b"\x00" + row for row in rows)

    def chunk(kind, body):
        crc = zlib.crc32(kind + body) & 0xFFFFFFFF
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", crc)

    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    Path(path).write_bytes(
        SIGNATURE + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b"")
    )


_DIM = bytes(v // 3 for v in range(256))


def _diff_rows(golden_rows, render_rows, width):
    """Count pixels past the tolerance, pixels that differ at all, and the
    largest channel difference; also return the rows of a difference image
    (the golden dimmed, offending pixels red)."""
    over = changed = largest = 0
    picture = []
    for a, b in zip(golden_rows, render_rows):
        if a == b:
            dim = bytearray(a.translate(_DIM))
            dim[3::4] = b"\xff" * width
            picture.append(bytes(dim))
            continue
        d = [abs(x - y) for x, y in zip(a, b)]
        largest = max(largest, max(d))
        per_pixel = list(map(max, d[0::4], d[1::4], d[2::4], d[3::4]))
        changed += sum(1 for v in per_pixel if v)
        bad = [i for i, v in enumerate(per_pixel) if v > TOLERANCE]
        over += len(bad)
        dim = bytearray(a.translate(_DIM))
        dim[3::4] = b"\xff" * width
        for i in bad:
            dim[i * 4:i * 4 + 4] = b"\xff\x00\x00\xff"
        picture.append(bytes(dim))
    return over, changed, largest, picture


def compare(task):
    """Compare one named scene. Returns a result dict; never raises."""
    name, golden, render, diff_dir = task
    result = {"name": name, "over": 0, "changed": 0, "largest": 0, "fail": True}
    try:
        gw, gh, grows = read_png(golden)
        rw, rh, rrows = read_png(render)
    except (PngError, zlib.error, OSError, struct.error) as error:
        result["note"] = f"unreadable: {error}"
        return result
    if (gw, gh) != (rw, rh):
        result["note"] = f"size changed {gw}x{gh} -> {rw}x{rh}"
        return result
    over, changed, largest, picture = _diff_rows(grows, rrows, gw)
    result.update(over=over, changed=changed, largest=largest, fail=over > 0)
    result["note"] = "look at the image and regenerate" if over else ""
    if over and diff_dir:
        Path(diff_dir).mkdir(parents=True, exist_ok=True)
        write_png(Path(diff_dir) / f"{name}.png", gw, gh, picture)
    return result


def collect(golden_dir, renders_dir, newer_than, diff_dir, skips=()):
    golden = {p.stem: p for p in Path(golden_dir).glob("*.png")}
    renders = {p.stem: p for p in Path(renders_dir).glob("*.png")}
    marker = os.stat(newer_than).st_mtime_ns if newer_than else None
    tasks, results = [], []
    for name in sorted(golden.keys() | renders.keys()):
        row = {"name": name, "over": "", "changed": "", "largest": "", "fail": True}
        skipped = next((why for glob, why in skips if fnmatch.fnmatchcase(name, glob)), None)
        if skipped is not None:
            row.update(fail=False, skipped=True, note=f"not checked: {skipped}")
            results.append(row)
        elif name not in renders:
            row["note"] = "no render (the test skipped or never ran)"
            results.append(row)
        elif name not in golden:
            row["note"] = "render has no golden"
            results.append(row)
        elif marker is not None and renders[name].stat().st_mtime_ns <= marker:
            row["note"] = "render is older than the run (not rewritten)"
            results.append(row)
        else:
            tasks.append((name, golden[name], renders[name], diff_dir))
    return tasks, results


def table(results):
    lines = [
        "| Scene | Over tolerance | Differing | Max channel diff | Verdict |",
        "| --- | ---: | ---: | ---: | --- |",
    ]
    for r in results:
        verdict = "FAIL" if r["fail"] else ("skipped" if r.get("skipped") else "ok")
        if r.get("note"):
            verdict += f": {r['note']}"
        lines.append(
            f"| {r['name']} | {r['over']} | {r['changed']} | {r['largest']} | {verdict} |"
        )
    return "\n".join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--golden", required=True, help="folder of approved PNGs")
    parser.add_argument("--renders", required=True, help="folder of rendered PNGs")
    parser.add_argument("--diff-dir", help="write a difference image per failing scene")
    parser.add_argument("--summary", help="append the table to this file")
    parser.add_argument("--newer-than", help="renders must be newer than this file")
    parser.add_argument(
        "--skip", action="append", default=[], metavar="GLOB=REASON",
        help="do not compare scenes matching GLOB; the table says REASON",
    )
    parser.add_argument("--jobs", type=int, default=os.cpu_count() or 1)
    args = parser.parse_args(argv)

    skips = []
    for item in args.skip:
        glob, _, why = item.partition("=")
        skips.append((glob, why or "skipped"))
    tasks, results = collect(args.golden, args.renders, args.newer_than, args.diff_dir, skips)
    if args.jobs > 1 and len(tasks) > 1:
        with multiprocessing.Pool(args.jobs) as pool:
            results += pool.map(compare, tasks, chunksize=1)
    else:
        results += [compare(task) for task in tasks]
    results.sort(key=lambda r: r["name"])
    failing = [r for r in results if r["fail"]]
    checked = [r for r in results if not r.get("skipped")]
    if not checked:
        failing = [{"name": "(none)"}]

    heading = (
        f"### Golden strict check: {'FAIL' if failing else 'pass'}\n\n"
        f"{len(results)} scenes, {len(results) - len(checked)} not checked, "
        f"{len(failing)} failing. A scene fails when any pixel's "
        f"channel differs by more than {TOLERANCE} (no count allowance).\n\n"
    )
    report = heading + (table(results) if results else "No PNGs found.") + "\n"
    print(report)
    if args.summary:
        with open(args.summary, "a", encoding="utf-8") as summary:
            summary.write(report)
    return 1 if failing else 0


if __name__ == "__main__":
    sys.exit(main())
