#!/usr/bin/env python3
"""Build the SQLite format consumed by places-core from Overture GeoParquet.

The Overture client downloads only the requested bounding box.  This script is
kept in packaging/ so a release job can refresh the Taiwan snapshot without
changing the runtime or introducing a Python dependency into the EXE.
"""

from __future__ import annotations

import argparse
import datetime as dt
import sqlite3
import struct
from pathlib import Path

import pyarrow.parquet as pq


SCHEMA = """
PRAGMA journal_mode=OFF;
PRAGMA synchronous=OFF;
PRAGMA temp_store=MEMORY;
CREATE TABLE store_meta (key TEXT PRIMARY KEY NOT NULL, value TEXT);
CREATE TABLE places (
  provider TEXT NOT NULL, id TEXT NOT NULL, name TEXT NOT NULL,
  category TEXT, address TEXT, latitude REAL NOT NULL, longitude REAL NOT NULL,
  rating REAL, rating_scale INTEGER, review_count INTEGER NOT NULL DEFAULT 0,
  open_now INTEGER, provider_score REAL, popularity REAL,
  popularity_source TEXT, source_updated_at TEXT, phone TEXT, website TEXT,
  external_url TEXT NOT NULL, UNIQUE(provider,id)
);
CREATE INDEX places_name_idx ON places(name);
CREATE VIRTUAL TABLE places_rtree USING rtree(id,min_lat,max_lat,min_lon,max_lon);
CREATE VIRTUAL TABLE places_fts USING fts5(id UNINDEXED,name,category,address);
"""

INSERT = """
INSERT INTO places(
 provider,id,name,category,address,latitude,longitude,rating,rating_scale,
 review_count,open_now,provider_score,popularity,popularity_source,
 source_updated_at,phone,website,external_url
) VALUES ('overture',?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
"""


def point(wkb: bytes) -> tuple[float, float] | None:
    """Decode a GeoArrow WKB Point without requiring a GIS dependency."""
    if not wkb or len(wkb) < 21:
        return None
    endian = "<" if wkb[0] == 1 else ">" if wkb[0] == 0 else None
    if endian is None:
        return None
    geometry_type = struct.unpack_from(endian + "I", wkb, 1)[0] & 0xFF
    if geometry_type != 1:
        return None
    x, y = struct.unpack_from(endian + "dd", wkb, 5)
    return y, x


def first(values):
    return values[0] if values else None


def fields(row: dict, min_confidence: float):
    coords = point(row.get("geometry"))
    name = (row.get("names") or {}).get("primary")
    if not coords or not name or not name.strip():
        return None
    # A release pack should contain searchable places rather than unnamed
    # geometry records.  Overture's confidence is optional; retain records
    # without a score but reject low-confidence records when requested.
    if row.get("basic_category") is None:
        return None
    confidence = row.get("confidence")
    if confidence is not None and confidence < min_confidence:
        return None
    latitude, longitude = coords
    address = first(row.get("addresses") or []) or {}
    address_text = ", ".join(
        value.strip()
        for value in (
            address.get("freeform"),
            address.get("locality"),
            address.get("postcode"),
        )
        if value and value.strip()
    ) or None
    category = (
        (row.get("categories") or {}).get("primary")
        or row.get("basic_category")
        or "place"
    )
    updated = None
    for source in row.get("sources") or []:
        if source.get("update_time"):
            updated = source["update_time"]
            break
    external = (
        "https://www.google.com/maps/search/?api=1&query="
        f"{latitude:.7f},{longitude:.7f}"
    )
    return (
        str(row["id"]),
        name.strip(),
        category,
        address_text,
        latitude,
        longitude,
        None,
        None,
        0,
        None,
        row.get("confidence"),
        None,
        None,
        updated,
        first(row.get("phones") or []),
        first(row.get("websites") or []),
        external,
    )


def build(source: Path, output: Path, version: str, min_confidence: float) -> int:
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.with_suffix(output.suffix + ".part")
    temporary.unlink(missing_ok=True)
    connection = sqlite3.connect(temporary)
    connection.executescript(SCHEMA)
    insert_count = 0
    table = pq.ParquetFile(source)
    try:
        for batch in table.iter_batches(batch_size=10_000):
            rows = [
                value
                for row in batch.to_pylist()
                if (value := fields(row, min_confidence))
            ]
            if not rows:
                continue
            with connection:
                cursor = connection.executemany(INSERT, rows)
                # The rows are unique in the Overture snapshot; use the same
                # rowids for the RTree and FTS5 indexes as places-core.
                for row in rows:
                    rowid = connection.execute(
                        "SELECT rowid FROM places WHERE provider='overture' AND id=?",
                        (row[0],),
                    ).fetchone()[0]
                    connection.execute(
                        "INSERT OR REPLACE INTO places_rtree VALUES(?,?,?,?,?)",
                        (rowid, row[4], row[4], row[5], row[5]),
                    )
                    connection.execute(
                        "INSERT OR REPLACE INTO places_fts(rowid,id,name,category,address) VALUES(?,?,?,?,?)",
                        (rowid, row[0], row[1], row[2], row[3]),
                    )
            insert_count += len(rows)
            if insert_count % 100_000 < len(rows):
                print(f"inserted {insert_count:,} places", flush=True)
    finally:
        refreshed = dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat()
        connection.execute(
            "INSERT INTO store_meta(key,value) VALUES('data_pack_version',?),('refreshed_at',?)",
            (version, refreshed),
        )
        connection.commit()
        connection.close()
    temporary.replace(output)
    print(f"wrote {insert_count:,} places to {output}")
    return insert_count


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path, help="Overture .geoparquet file")
    parser.add_argument("output", type=Path, help="places-core SQLite path")
    parser.add_argument("--version", default="overture-2026-06-17.0")
    parser.add_argument(
        "--min-confidence",
        type=float,
        default=0.8,
        help="retain only categorized places at or above this confidence (default: 0.8)",
    )
    args = parser.parse_args()
    build(args.source, args.output, args.version, args.min_confidence)


if __name__ == "__main__":
    main()
