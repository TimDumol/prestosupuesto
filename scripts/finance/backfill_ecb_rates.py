#!/usr/bin/env python3
"""Backfill ECB daily EUR/PHP and EUR/USD reference rates into Presto SQLite."""

from __future__ import annotations

import argparse
import csv
from datetime import date, datetime, timedelta, timezone
from decimal import Decimal, InvalidOperation
import hashlib
import io
import json
import os
from pathlib import Path
import sqlite3
import sys
from urllib.parse import quote, urlencode
from urllib.request import HTTPError, Request, urlopen


ECB_SERIES_KEY = "D.PHP+USD.EUR.SP00.A"
ECB_BASE_URL = "https://data-api.ecb.europa.eu/service/data/EXR/"
SOURCE_WORKBOOK_ID = "ECB-DATA-PORTAL-EXR-PHP-USD"
SOURCE_SHEET = "ECB_EXR"
CURRENCIES = ("PHP", "USD")


def parse_iso_date(value: str, option_name: str) -> date:
    try:
        parsed = date.fromisoformat(value)
    except ValueError as error:
        raise argparse.ArgumentTypeError(
            f"{option_name} must be a valid YYYY-MM-DD date."
        ) from error
    if parsed.isoformat() != value:
        raise argparse.ArgumentTypeError(
            f"{option_name} must use YYYY-MM-DD format."
        )
    return parsed


def decimal_text(value: str, context: str) -> str:
    try:
        number = Decimal(value)
    except InvalidOperation as error:
        raise ValueError(f"ECB returned a non-decimal rate for {context}.") from error
    if not number.is_finite() or number <= 0:
        raise ValueError(f"ECB returned a non-positive rate for {context}.")
    text = format(number.normalize(), "f")
    if "." in text:
        text = text.rstrip("0").rstrip(".")
    return text


def fetch_window(start_date: date, through_date: date) -> list[dict[str, str]]:
    query = urlencode(
        {
            "startPeriod": start_date.isoformat(),
            "endPeriod": through_date.isoformat(),
            "format": "csvdata",
        }
    )
    url = f"{ECB_BASE_URL}{quote(ECB_SERIES_KEY, safe='.')}?{query}"
    request = Request(
        url,
        headers={
            "Accept": "text/csv",
            "User-Agent": "PrestoFinance-ECB-backfill/1.0",
        },
    )
    try:
        with urlopen(request, timeout=45) as response:
            payload = response.read().decode("utf-8-sig")
    except HTTPError as error:
        detail = error.read(500).decode("utf-8", errors="replace").strip()
        raise ValueError(f"ECB API returned HTTP {error.code}: {detail}") from error

    reader = csv.DictReader(io.StringIO(payload))
    required_columns = {
        "CURRENCY",
        "CURRENCY_DENOM",
        "EXR_TYPE",
        "EXR_SUFFIX",
        "TIME_PERIOD",
        "OBS_VALUE",
        "OBS_STATUS",
    }
    if not reader.fieldnames or not required_columns.issubset(reader.fieldnames):
        raise ValueError("ECB response did not contain the expected CSV columns.")
    return list(reader)


def fetch_observations(start_date: date, through_date: date) -> dict[str, dict[str, dict[str, str]]]:
    observations: dict[str, dict[str, dict[str, str]]] = {}
    window_start = start_date
    while window_start <= through_date:
        window_end = min(date(window_start.year, 12, 31), through_date)
        for row in fetch_window(window_start, window_end):
            currency = row["CURRENCY"]
            if (
                currency not in CURRENCIES
                or row["CURRENCY_DENOM"] != "EUR"
                or row["EXR_TYPE"] != "SP00"
                or row["EXR_SUFFIX"] != "A"
            ):
                continue
            rate_date_text = row["TIME_PERIOD"]
            try:
                rate_date = date.fromisoformat(rate_date_text)
            except ValueError as error:
                raise ValueError(f"ECB returned an invalid observation date: {rate_date_text}") from error
            if rate_date < start_date or rate_date > through_date:
                raise ValueError(f"ECB returned an observation outside the requested date range: {rate_date_text}")
            rate = decimal_text(row["OBS_VALUE"], f"{currency} on {rate_date_text}")
            by_currency = observations.setdefault(rate_date_text, {})
            if currency in by_currency:
                raise ValueError(f"ECB returned duplicate {currency} observations for {rate_date_text}.")
            by_currency[currency] = {"rate": rate, "status": row["OBS_STATUS"]}
        window_start = date(window_end.year + 1, 1, 1)

    if not observations:
        raise ValueError("ECB returned no PHP or USD observations for the requested dates.")
    for currency in CURRENCIES:
        if not any(currency in row for row in observations.values()):
            raise ValueError(f"ECB returned no observations for {currency}.")
    return observations


def row_provenance(rate_date: str, observations: dict[str, dict[str, str]]) -> tuple[str, str]:
    source_record = {
        "source": "ECB Data Portal",
        "series": "EXR.D.PHP+USD.EUR.SP00.A",
        "date": rate_date,
        "observations": observations,
    }
    encoded = json.dumps(source_record, sort_keys=True, separators=(",", ":"))
    fingerprint = hashlib.sha256(encoded.encode("utf-8")).hexdigest()
    return encoded, fingerprint


def rate_rows(observations: dict[str, dict[str, dict[str, str]]]) -> list[tuple]:
    records: list[tuple] = []
    for rate_date, by_currency in sorted(observations.items()):
        source_row = int(rate_date.replace("-", ""))
        source_row_json, fingerprint = row_provenance(rate_date, by_currency)
        for currency in CURRENCIES:
            observed = by_currency.get(currency)
            if observed is None:
                continue
            source_value = json.dumps(
                {
                    "source": "ECB Data Portal",
                    "series": f"EXR.D.{currency}.EUR.SP00.A",
                    "date": rate_date,
                    "currency": currency,
                    "currencyDenom": "EUR",
                    "rate": observed["rate"],
                    "status": observed["status"],
                },
                sort_keys=True,
                separators=(",", ":"),
            )
            records.append(
                (
                    f"ecb-exr-{currency.lower()}-{rate_date}",
                    rate_date,
                    "EUR",
                    currency,
                    observed["rate"],
                    "ECB",
                    f"EUR{currency}",
                    currency,
                    source_value,
                    SOURCE_WORKBOOK_ID,
                    SOURCE_SHEET,
                    source_row,
                    fingerprint,
                    source_row_json,
                )
            )
    return records


def verify_schema(connection: sqlite3.Connection) -> None:
    expected = {
        "exchange_rates": {
            "id",
            "rate_date",
            "base_currency",
            "quote_currency",
            "rate_decimal",
            "provider",
            "source_pair",
            "source_column",
            "source_formula",
            "source_value_json",
            "source_workbook_id",
            "source_sheet",
            "source_row",
            "original_row_fingerprint",
        },
        "finance_exchange_rate_source_rows": {
            "source_workbook_id",
            "source_sheet",
            "source_row",
            "original_row_fingerprint",
            "source_row_json",
        },
    }
    for table, expected_columns in expected.items():
        try:
            columns = {
                row["name"]
                for row in connection.execute(f"PRAGMA table_info({table})")
            }
        except sqlite3.Error as error:
            raise ValueError(f"Could not inspect the {table} table.") from error
        if not expected_columns.issubset(columns):
            raise ValueError(f"The database schema for {table} is incomplete or outdated.")


def compare_existing(connection: sqlite3.Connection, records: list[tuple]) -> tuple[int, int]:
    rates = {
        (row["rate_date"], row["quote_currency"]): tuple(row)
        for row in connection.execute(
            "SELECT id, rate_date, base_currency, quote_currency, rate_decimal, provider, "
            "source_pair, source_column, source_value_json, source_workbook_id, source_sheet, "
            "source_row, original_row_fingerprint FROM exchange_rates "
            "WHERE source_workbook_id = ? AND source_sheet = ?",
            (SOURCE_WORKBOOK_ID, SOURCE_SHEET),
        )
    }
    source_rows = {
        row["source_row"]: tuple(row)
        for row in connection.execute(
            "SELECT source_row, original_row_fingerprint, source_row_json "
            "FROM finance_exchange_rate_source_rows WHERE source_workbook_id = ? AND source_sheet = ?",
            (SOURCE_WORKBOOK_ID, SOURCE_SHEET),
        )
    }

    expected_source_rows: dict[int, tuple[str, str]] = {}
    currencies_by_source_row: dict[int, set[str]] = {}
    inserted = 0
    updated = 0
    for record in records:
        (
            rate_id,
            rate_date,
            base_currency,
            quote_currency,
            rate_decimal,
            provider,
            source_pair,
            source_column,
            source_value_json,
            source_workbook_id,
            source_sheet,
            source_row,
            fingerprint,
            source_row_json,
        ) = record
        expected_source_rows[source_row] = (fingerprint, source_row_json)
        currencies_by_source_row.setdefault(source_row, set()).add(quote_currency)
        key = (rate_date, quote_currency)
        expected = (
            rate_id,
            rate_date,
            base_currency,
            quote_currency,
            rate_decimal,
            provider,
            source_pair,
            source_column,
            source_value_json,
            source_workbook_id,
            source_sheet,
            source_row,
            fingerprint,
        )
        existing = rates.get(key)
        if existing is None:
            inserted += 1
        elif existing != expected:
            updated += 1
    date_for_source_row = {
        int(record[1].replace("-", "")): record[1]
        for record in records
    }
    for source_row, expected in expected_source_rows.items():
        existing = source_rows.get(source_row)
        if existing is not None and existing != (source_row, *expected):
            updated += 1
        elif existing is None:
            rate_date = date_for_source_row[source_row]
            if all((rate_date, currency) in rates for currency in currencies_by_source_row[source_row]):
                updated += 1
    return inserted, updated


def make_backup(database: Path) -> Path:
    timestamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    backup = database.with_name(f"{database.name}.before-ecb-{timestamp}")
    source = sqlite3.connect(database, timeout=30)
    destination = sqlite3.connect(backup)
    try:
        source.backup(destination, pages=256, sleep=0.01)
    finally:
        destination.close()
        source.close()
    os.chmod(backup, 0o600)
    return backup


def write_rows(connection: sqlite3.Connection, records: list[tuple]) -> None:
    source_sql = """
        INSERT INTO finance_exchange_rate_source_rows
            (source_workbook_id, source_sheet, source_row, original_row_fingerprint, source_row_json)
        VALUES (?, ?, ?, ?, ?)
        ON CONFLICT(source_workbook_id, source_sheet, source_row) DO UPDATE SET
            original_row_fingerprint = excluded.original_row_fingerprint,
            source_row_json = excluded.source_row_json
    """
    rate_sql = """
        INSERT INTO exchange_rates
            (id, rate_date, base_currency, quote_currency, rate_decimal, provider, source_pair,
             source_column, source_formula, source_value_json, source_workbook_id, source_sheet,
             source_row, original_row_fingerprint)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?, ?, ?, ?)
        ON CONFLICT(source_workbook_id, source_sheet, source_row, source_column) DO UPDATE SET
            id = excluded.id,
            rate_date = excluded.rate_date,
            base_currency = excluded.base_currency,
            quote_currency = excluded.quote_currency,
            rate_decimal = excluded.rate_decimal,
            provider = excluded.provider,
            source_pair = excluded.source_pair,
            source_formula = excluded.source_formula,
            source_value_json = excluded.source_value_json,
            original_row_fingerprint = excluded.original_row_fingerprint
    """
    with connection:
        connection.executemany(
            source_sql,
            [
                (record[9], record[10], record[11], record[12], record[13])
                for record in records
            ],
        )
        connection.executemany(
            rate_sql,
            [record[:8] + record[8:13] for record in records],
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", required=True, type=Path, help="Existing Presto SQLite database path.")
    parser.add_argument(
        "--start-date",
        help="First ECB observation date to fetch (default: first day of the previous month).",
    )
    parser.add_argument(
        "--through-date",
        help="Last date to fetch (default: today in UTC).",
    )
    parser.add_argument("--dry-run", action="store_true", help="Fetch and compare without writing or backing up.")
    args = parser.parse_args()

    try:
        today_utc = datetime.now(timezone.utc).date()
        previous_month = (today_utc.replace(day=1) - timedelta(days=1)).replace(day=1)
        start_date = parse_iso_date(args.start_date or previous_month.isoformat(), "--start-date")
        through_date = parse_iso_date(
            args.through_date or today_utc.isoformat(),
            "--through-date",
        )
        if through_date < start_date:
            raise ValueError("--through-date must be on or after --start-date.")
        database = args.database.expanduser().resolve()
        if not database.is_file():
            raise ValueError(f"Database does not exist: {database}")

        observations = fetch_observations(start_date, through_date)
        records = rate_rows(observations)
        connection = sqlite3.connect(database, timeout=30)
        connection.row_factory = sqlite3.Row
        connection.execute("PRAGMA busy_timeout = 30000")
        try:
            verify_schema(connection)
            inserted, updated = compare_existing(connection, records)
            if args.dry_run:
                print(
                    f"Dry run: {len(records)} ECB observations fetched "
                    f"({len([row for row in observations.values() if 'PHP' in row])} PHP dates, "
                    f"{len([row for row in observations.values() if 'USD' in row])} USD dates); "
                    f"{inserted} new rate rows and {updated} changed rows."
                )
                return 0
            if inserted == 0 and updated == 0:
                print(f"ECB rates are current through {max(observations)}; no database changes needed.")
                return 0

            backup = make_backup(database)
            write_rows(connection, records)
            print(
                f"Backfilled {len(records)} ECB observations through {max(observations)} "
                f"({inserted} new rate rows, {updated} changed rows)."
            )
            print(f"SQLite backup: {backup}")
            return 0
        finally:
            connection.close()
    except (OSError, sqlite3.Error, ValueError) as error:
        print(f"ECB rate backfill failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
