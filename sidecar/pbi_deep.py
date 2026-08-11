#!/usr/bin/env python3
"""Small JSON bridge between PBI Lens and PBIXRay.

All output on stdout is one JSON document so the Rust host can treat this as a
replaceable, crash-isolated decoder. PBIX files are never modified.
"""

from __future__ import annotations

import argparse
from collections import OrderedDict
import json
import math
import sys
from pathlib import Path
from typing import Any

from pbixray import PBIXRay


def json_value(value: Any) -> Any:
    if value is None:
        return None
    if isinstance(value, float) and (math.isnan(value) or math.isinf(value)):
        return None
    if isinstance(value, (str, int, float, bool)):
        return value
    if hasattr(value, "item"):
        try:
            return json_value(value.item())
        except (TypeError, ValueError):
            pass
    if hasattr(value, "isoformat"):
        try:
            return value.isoformat()
        except (TypeError, ValueError):
            pass
    return str(value)


def records(frame: Any, limit: int | None = None) -> list[dict[str, Any]]:
    if frame is None or not hasattr(frame, "to_dict"):
        return []
    if limit is not None:
        frame = frame.head(limit)
    return [
        {str(key): json_value(value) for key, value in row.items()}
        for row in frame.to_dict("records")
    ]


def safe_frame(
    report: PBIXRay,
    name: str,
    limit: int | None = None,
    warnings: list[str] | None = None,
) -> list[dict[str, Any]]:
    try:
        return records(getattr(report, name), limit)
    except Exception as error:
        if warnings is not None:
            warnings.append(f"{name}: {error}")
        return []


def metadata(path: Path, report: PBIXRay | None = None) -> dict[str, Any]:
    report = report or PBIXRay(str(path), on_disk=True)
    warnings: list[str] = []
    schema = safe_frame(report, "schema", warnings=warnings)
    schema_ok = bool(schema)
    if not schema_ok and not any(item.startswith("schema:") for item in warnings):
        warnings.append("schema: the semantic model did not expose any tables or columns")
    statistics = safe_frame(report, "statistics", 10_000, warnings)
    measures = safe_frame(report, "dax_measures", 10_000, warnings)
    dax_columns = safe_frame(report, "dax_columns", 10_000, warnings)
    dax_tables = safe_frame(report, "dax_tables", 2_000, warnings)
    table_meta = safe_frame(report, "tmschema_tables", 2_000, warnings)
    column_meta = safe_frame(report, "tmschema_columns", 20_000, warnings)

    stats_by_key = {
        (row.get("TableName"), row.get("ColumnName")): row for row in statistics
    }
    dax_by_key = {
        (row.get("TableName"), row.get("ColumnName")): row.get("Expression") or ""
        for row in dax_columns
    }
    column_by_key = {
        (row.get("TableName"), row.get("Name")): row for row in column_meta
    }
    table_info = {row.get("Name"): row for row in table_meta}
    table_expressions = {
        row.get("TableName"): row.get("Expression") or "" for row in dax_tables
    }

    tables: dict[str, dict[str, Any]] = {}
    for item in schema:
        table_name = str(item.get("TableName") or "Unnamed table")
        column_name = str(item.get("ColumnName") or "Unnamed column")
        table = tables.setdefault(
            table_name,
            {
                "name": table_name,
                "columns": [],
                "is_hidden": bool((table_info.get(table_name) or {}).get("IsHidden") or 0),
                "description": (table_info.get(table_name) or {}).get("Description") or "",
                "expression": table_expressions.get(table_name, ""),
                "row_count": None,
            },
        )
        extra = column_by_key.get((table_name, column_name), {})
        stat = stats_by_key.get((table_name, column_name), {})
        table["columns"].append(
            {
                "name": column_name,
                "data_type": str(item.get("PandasDataType") or extra.get("DataType") or ""),
                "kind": "Calculated column" if dax_by_key.get((table_name, column_name)) else "Column",
                "expression": dax_by_key.get((table_name, column_name), ""),
                "is_hidden": bool(extra.get("IsHidden") or 0),
                "description": extra.get("Description") or "",
                "format_string": extra.get("FormatString") or "",
                "display_folder": extra.get("DisplayFolder") or "",
                "cardinality": stat.get("Cardinality"),
                "data_size": stat.get("DataSize"),
            }
        )

    for measure in measures:
        table_name = str(measure.get("TableName") or "Measures")
        table = tables.setdefault(
            table_name,
            {
                "name": table_name,
                "columns": [],
                "is_hidden": bool((table_info.get(table_name) or {}).get("IsHidden") or 0),
                "description": (table_info.get(table_name) or {}).get("Description") or "",
                "expression": table_expressions.get(table_name, ""),
                "row_count": None,
            },
        )
        table["columns"].append(
            {
                "name": str(measure.get("Name") or "Unnamed measure"),
                "data_type": "DAX",
                "kind": "Measure",
                "expression": measure.get("Expression") or "",
                "is_hidden": False,
                "description": measure.get("Description") or "",
                "format_string": "",
                "display_folder": measure.get("DisplayFolder") or "",
                "cardinality": None,
                "data_size": None,
            }
        )

    queries = [
        {
            "name": str(row.get("TableName") or "Unnamed query"),
            "formula": row.get("Expression") or "",
        }
        for row in safe_frame(report, "power_query", 5_000, warnings)
    ]

    relationships = [
        {
            "from_table": row.get("FromTableName") or "",
            "from_column": row.get("FromColumnName") or "",
            "to_table": row.get("ToTableName") or "",
            "to_column": row.get("ToColumnName") or "",
            "is_active": bool(row.get("IsActive") or 0),
            "cardinality": row.get("Cardinality") or "",
            "cross_filtering": row.get("CrossFilteringBehavior") or "",
            "referential_integrity": bool(row.get("RelyOnReferentialIntegrity") or 0),
        }
        for row in safe_frame(report, "relationships", 20_000, warnings)
    ]

    return {
        "decoder": "PBIXRay",
        "schema_ok": schema_ok,
        "warnings": warnings,
        "tables": list(tables.values()),
        "relationships": relationships,
        "queries": queries,
        "statistics": statistics,
        "partitions": safe_frame(report, "tmschema_partitions", 2_000, warnings),
        "roles": safe_frame(report, "rls", 2_000, warnings),
        "object_security": safe_frame(report, "ols", 2_000, warnings),
        "perspectives": safe_frame(report, "perspectives", 2_000, warnings),
        "parameters": safe_frame(report, "m_parameters", 2_000, warnings),
        "kpis": safe_frame(report, "tmschema_kpis", 2_000, warnings),
        "calculation_groups": safe_frame(
            report, "tmschema_calculation_groups", 2_000, warnings
        ),
        "calculation_items": safe_frame(
            report, "tmschema_calculation_items", 10_000, warnings
        ),
    }


def table_rows(
    path: Path,
    table_name: str,
    offset: int,
    limit: int,
    report: PBIXRay | None = None,
    frame: Any = None,
) -> dict[str, Any]:
    report = report or PBIXRay(str(path), on_disk=True)
    if frame is None:
        frame = report.get_table(table_name)
    total = int(len(frame.index))
    page = frame.iloc[offset : offset + limit]
    columns = [str(column) for column in page.columns]
    rows = [[json_value(value) for value in row] for row in page.itertuples(index=False, name=None)]
    return {
        "table": table_name,
        "columns": columns,
        "rows": rows,
        "offset": offset,
        "limit": limit,
        "total": total,
    }


def serve() -> int:
    cached_file_key: tuple[str, int, int] | None = None
    cached_report: PBIXRay | None = None
    table_cache: OrderedDict[tuple[tuple[str, int, int], str], Any] = OrderedDict()
    for line in sys.stdin:
        try:
            request = json.loads(line)
            path = Path(request["path"])
            stat = path.stat()
            file_key = (str(path.resolve()), stat.st_size, stat.st_mtime_ns)
            if cached_report is None or cached_file_key != file_key:
                cached_report = PBIXRay(str(path), on_disk=True)
                cached_file_key = file_key
            if request.get("command") == "metadata":
                result = metadata(path, cached_report)
            elif request.get("command") == "table":
                table_name = str(request["table_name"])
                table_key = (file_key, table_name)
                frame = table_cache.get(table_key)
                if frame is None:
                    frame = cached_report.get_table(table_name)
                    table_cache[table_key] = frame
                    table_cache.move_to_end(table_key)
                    while len(table_cache) > 2:
                        table_cache.popitem(last=False)
                else:
                    table_cache.move_to_end(table_key)
                result = table_rows(
                    path,
                    table_name,
                    max(0, int(request.get("offset", 0))),
                    min(500, max(1, int(request.get("limit", 100)))),
                    cached_report,
                    frame,
                )
            else:
                raise ValueError("Unknown decoder command")
        except Exception as error:
            result = {"error": str(error)}
        print(
            json.dumps(result, ensure_ascii=False, separators=(",", ":"), allow_nan=False),
            flush=True,
        )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(prog="pbi-deep")
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("serve")
    meta = subparsers.add_parser("metadata")
    meta.add_argument("path", type=Path)
    table = subparsers.add_parser("table")
    table.add_argument("path", type=Path)
    table.add_argument("table_name")
    table.add_argument("--offset", type=int, default=0)
    table.add_argument("--limit", type=int, default=100)
    args = parser.parse_args()

    try:
        if args.command == "serve":
            return serve()
        if args.command == "metadata":
            result = metadata(args.path)
        else:
            result = table_rows(
                args.path,
                args.table_name,
                max(0, args.offset),
                min(500, max(1, args.limit)),
            )
        json.dump(result, sys.stdout, ensure_ascii=False, separators=(",", ":"), allow_nan=False)
        return 0
    except Exception as error:
        print(json.dumps({"error": str(error)}, ensure_ascii=False), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
