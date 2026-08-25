"""Corpus tests for the streaming read paths.

Every file in the real-world YXDB corpus is read two independent ways -- the
eager whole-file path and the batched streaming path -- and the results must
agree. The two paths share only the field decoders, so agreement across the
corpus is a strong check on block handling, record framing, and projection.

Set ``YXDB_CORPUS_DIR`` to point at a directory containing ``e1/`` and ``e2/``
subdirectories. Tests skip when the corpus is unavailable.
"""

from __future__ import annotations

import os
from pathlib import Path

import polars as pl
import pytest

import sigilyx as yx

DEFAULT_CORPUS_DIR = Path("/workspaces/Projects/Data/YXDB")


def corpus_files(layout: str) -> list[Path]:
    """Return every ``.yxdb`` file for ``layout``, sorted; empty if unavailable."""
    root = Path(os.environ.get("YXDB_CORPUS_DIR", DEFAULT_CORPUS_DIR))
    directory = root / layout
    if not directory.is_dir():
        return []
    return sorted(p for p in directory.glob("*.yxdb") if p.is_file())


E1_FILES = corpus_files("e1")
E2_FILES = corpus_files("e2")

if not E1_FILES and not E2_FILES:
    pytest.skip("YXDB corpus not available", allow_module_level=True)


def _has_spatial_index(path: Path) -> bool:
    try:
        return bool(yx.read_spatial_info(str(path))["has_spatial_index"])
    except Exception:  # noqa: BLE001
        return False


# Files carrying a spatial index interleave grid blocks in the record block
# stream. They are read by the same paths as any other file, and parametrised
# separately only so a failure names them.
E1_SPATIAL_FILES = [p for p in E1_FILES if _has_spatial_index(p)]
E1_PLAIN_FILES = [p for p in E1_FILES if p not in set(E1_SPATIAL_FILES)]


def _concat_batches(path: Path, batch_size: int, **kwargs) -> pl.DataFrame | None:
    """Read `path` in batches and stack them, or None if it yields nothing."""
    frames = list(yx.read_yxdb_batches(str(path), batch_size=batch_size, **kwargs))
    if not frames:
        return None
    return pl.concat(frames, how="vertical")


def _sample(files: list[Path], n: int) -> list[Path]:
    """Take up to `n` files spread across the corpus rather than a prefix."""
    if len(files) <= n:
        return files
    return files[:: len(files) // n][:n]


# --------------------------------------------------------------------------- #
# Batched reads agree with eager reads
# --------------------------------------------------------------------------- #


@pytest.mark.parametrize("path", E1_PLAIN_FILES, ids=lambda p: p.name)
def test_e1_batched_matches_eager(path: Path) -> None:
    """The chunked whole-file path and the streaming path must agree.

    These are separate decode paths: the eager read decompresses whole block
    batches out of an mmap, while the streaming read walks the LZF stream one
    record at a time. Any divergence in block handling shows up here.
    """
    eager = yx.read_yxdb(str(path), spatial="raw")
    streamed = _concat_batches(path, batch_size=8192)
    if eager.height == 0:
        assert streamed is None or streamed.height == 0, path.name
        return
    assert streamed is not None, path.name
    assert streamed.height == eager.height, path.name
    assert streamed.columns == eager.columns, path.name
    assert streamed.equals(eager), path.name


@pytest.mark.parametrize("path", E2_FILES, ids=lambda p: p.name)
def test_e2_batched_matches_eager(path: Path) -> None:
    """E2 files must be readable through the batch reader, matching the eager read."""
    try:
        eager = yx.read_yxdb(str(path), spatial="raw")
    except Exception as exc:  # noqa: BLE001 - corpus holds intentionally odd files
        pytest.skip(f"file not readable eagerly: {exc}")

    streamed = _concat_batches(path, batch_size=1024)
    if eager.height == 0:
        assert streamed is None or streamed.height == 0, path.name
        return
    assert streamed is not None, path.name
    assert streamed.height == eager.height, path.name
    assert streamed.columns == eager.columns, path.name
    assert streamed.equals(eager), path.name


@pytest.mark.parametrize("layout", ["e1", "e2"])
def test_batch_size_does_not_change_results(layout: str) -> None:
    """Row content must not depend on where batch boundaries fall.

    Uses tiny batch sizes, so it is limited to low-row-count files to keep
    the number of DataFrame constructions reasonable.
    """
    checked = 0
    for path in _sample(corpus_files(layout), 40):
        if path.stat().st_size > 256 * 1024:
            continue
        try:
            reference = yx.read_yxdb(str(path), spatial="raw")
        except Exception:  # noqa: BLE001
            continue
        if not 4 <= reference.height <= 2000:
            continue
        for batch_size in (1, 3, 7, 1024):
            got = _concat_batches(path, batch_size=batch_size)
            assert got is not None, f"{path.name} @ {batch_size}"
            assert got.equals(reference), f"{path.name} @ batch_size={batch_size}"
        checked += 1
        if checked >= 8:
            break
    assert checked > 0, f"no {layout} file exercised"


# --------------------------------------------------------------------------- #
# Record counts
# --------------------------------------------------------------------------- #


@pytest.mark.parametrize("layout", ["e1", "e2"])
def test_record_count_matches_decoded_height(layout: str) -> None:
    """``record_count`` must equal the number of rows actually decoded."""
    checked = 0
    for path in _sample(corpus_files(layout), 25):
        try:
            height = yx.read_yxdb(str(path), spatial="raw").height
        except Exception:  # noqa: BLE001
            continue
        assert yx.record_count(str(path)) == height, path.name
        checked += 1
    assert checked > 0, f"no {layout} file exercised"


@pytest.mark.parametrize("layout", ["e1", "e2"])
def test_read_schema_matches_frame_columns(layout: str) -> None:
    """``read_schema`` must describe the columns the reader actually produces."""
    checked = 0
    for path in _sample(corpus_files(layout), 25):
        try:
            df = yx.read_yxdb(str(path), spatial="raw")
        except Exception:  # noqa: BLE001
            continue
        names = [f["name"] for f in yx.read_schema(str(path))]
        assert names == df.columns, path.name
        checked += 1
    assert checked > 0, f"no {layout} file exercised"


# --------------------------------------------------------------------------- #
# Projection and lazy scan
# --------------------------------------------------------------------------- #


@pytest.mark.parametrize("layout", ["e1", "e2"])
def test_projection_matches_eager_selection(layout: str) -> None:
    """Reading a projection must equal reading everything and selecting."""
    checked = 0
    for path in _sample(corpus_files(layout), 15):
        try:
            full = yx.read_yxdb(str(path), spatial="raw")
        except Exception:  # noqa: BLE001
            continue
        if full.width < 2:
            continue
        # Reverse and drop half, so both order and subset matter.
        wanted = list(reversed(full.columns))[: max(1, full.width // 2)]
        got = yx.read_yxdb_columns(str(path), wanted, spatial="raw")
        assert got.columns == wanted, path.name
        assert got.equals(full.select(wanted)), path.name

        streamed = _concat_batches(path, batch_size=64, columns=wanted)
        if full.height > 0:
            assert streamed is not None, path.name
            assert streamed.columns == wanted, path.name
            assert streamed.equals(full.select(wanted)), path.name
        checked += 1
    assert checked > 0, f"no {layout} file exercised"


@pytest.mark.parametrize("layout", ["e1", "e2"])
def test_scan_collect_matches_eager(layout: str) -> None:
    """A full lazy collect must equal the eager read."""
    checked = 0
    for path in _sample(corpus_files(layout), 15):
        if path.stat().st_size > 4 << 20:
            continue
        try:
            eager = yx.read_yxdb(str(path), spatial="raw")
        except Exception:  # noqa: BLE001
            continue
        got = yx.scan_yxdb(str(path)).collect()
        assert got.columns == eager.columns, path.name
        assert got.height == eager.height, path.name
        assert got.equals(eager), path.name
        checked += 1
    assert checked > 0, f"no {layout} file exercised"


@pytest.mark.parametrize("layout", ["e1", "e2"])
def test_scan_head_matches_prefix(layout: str) -> None:
    """Row-limit pushdown must return exactly the first k rows."""
    checked = 0
    for path in _sample(corpus_files(layout), 15):
        try:
            eager = yx.read_yxdb(str(path), spatial="raw")
        except Exception:  # noqa: BLE001
            continue
        if eager.height < 5:
            continue
        k = min(37, eager.height)
        got = yx.scan_yxdb(str(path)).head(k).collect()
        assert got.height == k, path.name
        assert got.equals(eager.head(k)), path.name
        checked += 1
    assert checked > 0, f"no {layout} file exercised"


@pytest.mark.parametrize("layout", ["e1", "e2"])
def test_n_rows_limit_stops_early(layout: str) -> None:
    """``n_rows`` must cap the total rows yielded across batches."""
    checked = 0
    for path in _sample(corpus_files(layout), 15):
        try:
            eager = yx.read_yxdb(str(path), spatial="raw")
        except Exception:  # noqa: BLE001
            continue
        if eager.height < 10:
            continue
        limit = eager.height // 2
        got = _concat_batches(path, batch_size=3, n_rows=limit)
        assert got is not None, path.name
        assert got.height == limit, path.name
        assert got.equals(eager.head(limit)), path.name
        checked += 1
    assert checked > 0, f"no {layout} file exercised"


# --------------------------------------------------------------------------- #
# Spatial-index files
# --------------------------------------------------------------------------- #


def _spatial_index_files() -> list[Path]:
    """E1 corpus files whose header advertises a spatial index."""
    out = []
    for path in E1_FILES:
        try:
            if yx.read_spatial_info(str(path))["has_spatial_index"]:
                out.append(path)
        except Exception:  # noqa: BLE001
            continue
    return out


SPATIAL_FILES = _spatial_index_files()


@pytest.mark.skipif(not SPATIAL_FILES, reason="no spatial-index files in corpus")
@pytest.mark.parametrize("path", SPATIAL_FILES, ids=lambda p: p.name)
def test_spatial_index_file_decodes_to_wkb(path: Path) -> None:
    """Spatial-index files decode every record, and SpatialObj becomes WKB."""
    df = yx.read_yxdb(str(path), spatial="wkb")
    assert df.height == yx.record_count(str(path)), path.name

    for name in yx.read_spatial_info(str(path))["spatial_columns"]:
        values = df[name].drop_nulls()
        if values.is_empty():
            continue
        # ISO WKB starts with a byte-order marker of 0x00 (big) or 0x01 (little).
        assert values[0][0] in (0, 1), f"{path.name}: {name} is not WKB"


@pytest.mark.parametrize("path", E1_SPATIAL_FILES, ids=lambda p: p.name)
def test_spatial_index_batched_matches_eager(path: Path) -> None:
    """Spatial-index files must stream to the same rows as the eager read.

    Grid blocks are skipped only where the format can place one: on a record
    boundary that also ends a record-block-index group. Records spanning
    several blocks, and blocks reached part-way through a record, must not be
    mistaken for grid data.
    """
    eager = yx.read_yxdb(str(path), spatial="raw")
    streamed = _concat_batches(path, batch_size=8192)
    if eager.height == 0:
        assert streamed is None or streamed.height == 0, path.name
        return
    assert streamed is not None, path.name
    assert streamed.height == eager.height, path.name
    assert streamed.columns == eager.columns, path.name
    assert streamed.equals(eager), path.name


@pytest.mark.skipif(not E1_SPATIAL_FILES, reason="no spatial-index files in corpus")
def test_spatial_index_files_stream_with_records_spanning_blocks() -> None:
    """Cover files whose records are larger than one 256 KiB block.

    A record spanning several blocks leaves every block after the first
    starting mid-record, and leaves the first one unable to show a complete
    record. Both look like grid data to a per-block test.
    """
    covered = 0
    for path in E1_SPATIAL_FILES:
        height = yx.record_count(str(path))
        if height == 0 or path.stat().st_size / height < 512 * 1024:
            continue
        streamed = _concat_batches(path, batch_size=8192)
        assert streamed is not None, path.name
        assert streamed.height == height, path.name
        covered += 1
    assert covered > 0, "no corpus file has records spanning multiple blocks"
