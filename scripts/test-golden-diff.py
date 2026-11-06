#!/usr/bin/env python3
"""Tests for golden-diff.py on small synthetic PNGs. Run: python3 scripts/test-golden-diff.py"""

import importlib.util
import os
import random
import struct
import tempfile
import unittest
import zlib
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "golden_diff", Path(__file__).resolve().with_name("golden-diff.py")
)
golden_diff = importlib.util.module_from_spec(spec)
spec.loader.exec_module(golden_diff)

W, H = 9, 7


def paeth(a, b, c):
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    if pa <= pb and pa <= pc:
        return a
    return b if pb <= pc else c


def encode(width, height, rows, filters=None):
    """An independent PNG encoder that applies the given filter type to each
    row, so the decoder is checked against every filter."""
    bpp, raw, prev = 4, b"", bytes(width * 4)
    for y, row in enumerate(rows):
        kind = filters[y % len(filters)] if filters else 0
        out = bytearray()
        for i, value in enumerate(row):
            a = row[i - bpp] if i >= bpp else 0
            b = prev[i]
            c = prev[i - bpp] if i >= bpp else 0
            predicted = (0, a, b, (a + b) >> 1, paeth(a, b, c))[kind]
            out.append((value - predicted) & 255)
        raw += bytes([kind]) + bytes(out)
        prev = row

    def chunk(tag, body):
        return (
            struct.pack(">I", len(body)) + tag + body
            + struct.pack(">I", zlib.crc32(tag + body) & 0xFFFFFFFF)
        )

    return (
        golden_diff.SIGNATURE
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw))
        + chunk(b"IEND", b"")
    )


def image(seed=1, width=W, height=H):
    rng = random.Random(seed)
    return [bytes(rng.randrange(60, 190) for _ in range(width * 4)) for _ in range(height)]


def nudge(rows, deltas):
    """Add delta to the byte at (row, column)."""
    rows = [bytearray(r) for r in rows]
    for (y, x), delta in deltas.items():
        rows[y][x] += delta
    return [bytes(r) for r in rows]


class GoldenDiff(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.golden = self.root / "golden"
        self.renders = self.root / "renders"
        self.diffs = self.root / "diffs"
        self.golden.mkdir()
        self.renders.mkdir()

    def tearDown(self):
        self.tmp.cleanup()

    def put(self, folder, name, rows, width=W, height=H, filters=None):
        (folder / f"{name}.png").write_bytes(encode(width, height, rows, filters))

    def run_tool(self, *extra):
        args = [
            "--golden", str(self.golden), "--renders", str(self.renders),
            "--diff-dir", str(self.diffs), "--jobs", "1", *extra,
        ]
        return golden_diff.main(args)

    def test_every_filter_type_decodes(self):
        rows = image(3)
        for kind in range(5):
            path = self.root / f"f{kind}.png"
            path.write_bytes(encode(W, H, rows, [kind]))
            self.assertEqual(golden_diff.read_png(path), (W, H, rows), f"filter {kind}")

    def test_mixed_filters_across_rows_decode(self):
        rows = image(4)
        path = self.root / "mixed.png"
        path.write_bytes(encode(W, H, rows, [4, 1, 2, 3, 0, 2, 4]))
        self.assertEqual(golden_diff.read_png(path), (W, H, rows))

    def test_identical_images_pass(self):
        rows = image()
        self.put(self.golden, "a", rows)
        self.put(self.renders, "a", rows, filters=[4, 2])
        self.assertEqual(self.run_tool(), 0)

    def test_every_pixel_off_by_the_tolerance_passes(self):
        rows = image()
        shifted = [bytes(v + golden_diff.TOLERANCE for v in r) for r in rows]
        self.put(self.golden, "a", rows)
        self.put(self.renders, "a", shifted)
        self.assertEqual(self.run_tool(), 0)

    def test_one_pixel_past_the_tolerance_fails(self):
        rows = image()
        self.put(self.golden, "a", rows)
        self.put(self.renders, "a", nudge(rows, {(3, 5): golden_diff.TOLERANCE + 1}))
        self.assertEqual(self.run_tool(), 1)
        self.assertTrue((self.diffs / "a.png").exists())

    def test_result_counts_the_pixels_and_the_largest_difference(self):
        rows = image()
        self.put(self.golden, "a", rows)
        self.put(self.renders, "a", nudge(rows, {(0, 0): 13, (0, 1): 20, (6, 35): 5}))
        result = golden_diff.compare(("a", self.golden / "a.png", self.renders / "a.png", None))
        self.assertEqual((result["over"], result["changed"], result["largest"]), (1, 2, 20))
        self.assertTrue(result["fail"])

    def test_alpha_counts(self):
        rows = image()
        self.put(self.golden, "a", rows)
        self.put(self.renders, "a", nudge(rows, {(2, 4 * 2 + 3): 40}))
        self.assertEqual(self.run_tool(), 1)

    def test_size_change_fails(self):
        self.put(self.golden, "a", image())
        self.put(self.renders, "a", image(width=W + 1), width=W + 1)
        self.assertEqual(self.run_tool(), 1)

    def test_missing_render_and_missing_golden_fail(self):
        self.put(self.golden, "only-golden", image())
        self.assertEqual(self.run_tool(), 1)
        (self.golden / "only-golden.png").unlink()
        self.put(self.renders, "only-render", image())
        self.assertEqual(self.run_tool(), 1)

    def test_render_older_than_the_marker_fails(self):
        rows = image()
        self.put(self.golden, "a", rows)
        self.put(self.renders, "a", rows)
        marker = self.root / "marker"
        marker.touch()
        os.utime(self.renders / "a.png", ns=(1, 1))
        self.assertEqual(self.run_tool("--newer-than", str(marker)), 1)
        os.utime(self.renders / "a.png", ns=(marker.stat().st_mtime_ns + 10**9,) * 2)
        self.assertEqual(self.run_tool("--newer-than", str(marker)), 0)

    def test_empty_folders_fail(self):
        self.assertEqual(self.run_tool(), 1)

    def test_summary_gets_the_table(self):
        rows = image()
        self.put(self.golden, "a", rows)
        self.put(self.renders, "a", nudge(rows, {(1, 1): 99}))
        summary = self.root / "summary.md"
        self.assertEqual(self.run_tool("--summary", str(summary)), 1)
        text = summary.read_text()
        self.assertIn("| a | 1 | 1 | 99 | FAIL", text)
        self.assertIn("Golden strict check: FAIL", text)


if __name__ == "__main__":
    unittest.main()
