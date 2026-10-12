#!/usr/bin/env python3
"""Recalculate archived Transactions!J:Q formulas without writing to Sheets."""

import argparse
import json
import math
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path


ERROR = object()
OUTPUT_COLUMNS = "JKLMNOPQ"
RATE_COLUMNS = {"PHPEUR": (0, 1), "USDEUR": (2, 3), "PHPUSD": (4, 5)}


class FormulaReplay:
    def __init__(self, archive):
        self.sheets = archive["sheets"]
        self.transactions = self.sheets["transactions"]["rows"]
        self.account_currency = self._account_currencies()
        self.rates = self._rate_series()
        self.cache = {}
        self.visiting = set()

    @staticmethod
    def _effective(cell):
        return (cell or {}).get("effective") or {}

    @staticmethod
    def _entered(cell):
        return (cell or {}).get("entered") or {}

    @classmethod
    def _value(cls, cell):
        effective = cls._effective(cell)
        if "errorValue" in effective:
            return ERROR
        for key in ("stringValue", "numberValue", "boolValue"):
            if key in effective:
                return effective[key]
        return None

    @staticmethod
    def _number(value):
        if value is ERROR:
            return ERROR
        if value is None or value == "":
            # Google Sheets treats an empty cell as zero in multiplication.
            return 0.0
        if isinstance(value, (int, float)) and not isinstance(value, bool):
            return float(value)
        return ERROR

    @staticmethod
    def _row_reference(formula, column):
        # Skip references qualified by another sheet, such as Accounts!$A$1.
        match = re.search(
            rf"(?<![A-Za-z0-9_!$])\$?{column}\$?(\d+)", formula
        )
        return int(match.group(1)) if match else None

    @staticmethod
    def _require_formula_family(column, formula):
        expected = {
            "J": ("DGET", "H"),
            "K": ("IFS", "XLOOKUP", "D"),
            "L": ("*", "C", "K"),
            "M": ("IFS", "XLOOKUP", "D", "J"),
            "N": ("*", "C", "M"),
            "O": ("IF", "DGET", "I"),
            "P": ("IFS", "XLOOKUP", "O"),
            "Q": ("*", "P", "E"),
        }[column]
        upper = formula.upper()
        if not all(token.upper() in upper for token in expected):
            raise ValueError(f"unsupported formula template at Transactions!{column}")

    def _account_currencies(self):
        sheet = self.sheets["accounts"]
        headers = {
            str(self._value(cell)): index
            for index, cell in enumerate(sheet["rows"][0])
            if self._value(cell) is not None
        }
        name_column = headers["Name"]
        currency_column = headers["Currency"]
        currencies = {}
        for row in sheet["rows"][1:]:
            name = self._value(row[name_column] if name_column < len(row) else None)
            currency = self._value(
                row[currency_column] if currency_column < len(row) else None
            )
            if isinstance(name, str) and name.strip():
                currencies[name.strip().casefold()] = (
                    currency.strip() if isinstance(currency, str) else ""
                )
        return currencies

    def _rate_series(self):
        rows = self.sheets["exchangeRates"]["rows"]
        series = {}
        for pair, (date_column, rate_column) in RATE_COLUMNS.items():
            values = defaultdict(set)
            for row in rows[2:]:
                date_cell = row[date_column] if date_column < len(row) else None
                rate_cell = row[rate_column] if rate_column < len(row) else None
                date = self._number(self._value(date_cell))
                rate = self._number(self._value(rate_cell))
                if date is ERROR or rate is ERROR:
                    continue
                values[date].add(rate)
            if any(len(rates) > 1 for rates in values.values()):
                raise ValueError(f"ambiguous duplicate dates in {pair} rate table")
            series[pair] = [(date, next(iter(rates))) for date, rates in values.items()]
        return series

    def _prior_rate(self, pair, date):
        if date is ERROR or pair not in self.rates:
            return ERROR
        eligible = [item for item in self.rates[pair] if item[0] <= date]
        return max(eligible, key=lambda item: item[0])[1] if eligible else ERROR

    def _cell(self, column, row_number):
        row = (
            self.transactions[row_number - 1]
            if 1 <= row_number <= len(self.transactions)
            else None
        )
        index = ord(column) - ord("A")
        return row[index] if row is not None and index < len(row) else None

    def evaluate(self, column, row_number):
        key = (column, row_number)
        if key in self.cache:
            return self.cache[key]

        cell = self._cell(column, row_number)
        effective = self._effective(cell)
        if column not in OUTPUT_COLUMNS:
            return ERROR if "errorValue" in effective else self._value(cell)

        formula = self._entered(cell).get("formulaValue")
        if not isinstance(formula, str):
            return ERROR if "errorValue" in effective else self._value(cell)
        if "#REF!" in formula:
            self.cache[key] = ERROR
            return ERROR
        self._require_formula_family(column, formula)
        if key in self.visiting:
            return ERROR

        self.visiting.add(key)
        try:
            if column == "J":
                source_row = self._row_reference(formula, "H")
                name = self.evaluate("H", source_row) if source_row else None
                if name is ERROR:
                    result = ERROR
                elif not name:
                    result = ""
                else:
                    result = self.account_currency.get(str(name).strip().casefold(), ERROR)
            elif column == "K":
                currency_row = self._row_reference(formula, "D")
                date_row = self._row_reference(formula, "A")
                currency = self.evaluate("D", currency_row) if currency_row else None
                date = self._number(self.evaluate("A", date_row)) if date_row else ERROR
                if currency is ERROR:
                    result = ERROR
                else:
                    currency = currency.strip().upper() if isinstance(currency, str) else ""
                    pair = {"PHP": "PHPEUR", "USD": "USDEUR"}.get(currency)
                    rate = self._prior_rate(pair, date) if pair else None
                    result = ERROR if pair and rate is ERROR else 1.02 * rate if pair else 1.0
            elif column == "L":
                amount_row = self._row_reference(formula, "C")
                rate_row = self._row_reference(formula, "K")
                amount = self._number(self.evaluate("C", amount_row)) if amount_row else ERROR
                rate = self._number(self.evaluate("K", rate_row)) if rate_row else ERROR
                result = amount * rate if amount is not ERROR and rate is not ERROR else ERROR
            elif column == "M":
                currency_row = self._row_reference(formula, "D")
                account_currency_row = self._row_reference(formula, "J")
                amount_eur_row = self._row_reference(formula, "L")
                date_row = self._row_reference(formula, "A")
                currency = self.evaluate("D", currency_row) if currency_row else None
                account_currency = (
                    self.evaluate("J", account_currency_row)
                    if account_currency_row
                    else None
                )
                if currency is ERROR or account_currency is ERROR:
                    result = ERROR
                else:
                    currency = currency.strip().casefold() if isinstance(currency, str) else ""
                    account_currency = (
                        account_currency.strip().casefold()
                        if isinstance(account_currency, str)
                        else ""
                    )
                    if currency == account_currency:
                        result = 1.0
                    elif account_currency == "eur":
                        result = self.evaluate("L", amount_eur_row) if amount_eur_row else ERROR
                    elif account_currency == "php":
                        date = self._number(self.evaluate("A", date_row)) if date_row else ERROR
                        pair = {"eur": "PHPEUR", "usd": "PHPUSD"}.get(currency)
                        rate = self._prior_rate(pair, date) if pair else ERROR
                        result = ERROR if rate is ERROR or not rate else 1.02 / rate
                    else:
                        result = ERROR
            elif column == "N":
                amount_row = self._row_reference(formula, "C")
                rate_row = self._row_reference(formula, "M")
                amount = self._number(self.evaluate("C", amount_row)) if amount_row else ERROR
                rate = self._number(self.evaluate("M", rate_row)) if rate_row else ERROR
                result = amount * rate if amount is not ERROR and rate is not ERROR else ERROR
            elif column == "O":
                destination_row = self._row_reference(formula, "I")
                source_currency_row = self._row_reference(formula, "J")
                destination = self.evaluate("I", destination_row) if destination_row else None
                if destination is ERROR:
                    result = ERROR
                elif not destination:
                    result = ""
                elif isinstance(destination, str) and destination.strip().casefold() == "expense":
                    result = (
                        self.evaluate("J", source_currency_row)
                        if source_currency_row
                        else ERROR
                    )
                else:
                    result = self.account_currency.get(
                        str(destination).strip().casefold(), ERROR
                    )
            elif column == "P":
                currency_row = self._row_reference(formula, "O")
                date_row = self._row_reference(formula, "A")
                currency = self.evaluate("O", currency_row) if currency_row else None
                date = self._number(self.evaluate("A", date_row)) if date_row else ERROR
                if currency is ERROR:
                    result = ERROR
                else:
                    currency = currency.strip().upper() if isinstance(currency, str) else ""
                    pair = {"PHP": "PHPEUR", "USD": "USDEUR"}.get(currency)
                    result = self._prior_rate(pair, date) if pair else 1.0
            else:  # Q
                rate_row = self._row_reference(formula, "P")
                amount_row = self._row_reference(formula, "E")
                rate = self._number(self.evaluate("P", rate_row)) if rate_row else ERROR
                amount = self._number(self.evaluate("E", amount_row)) if amount_row else ERROR
                result = rate * amount if rate is not ERROR and amount is not ERROR else ERROR
        except (TypeError, ValueError, ZeroDivisionError):
            result = ERROR
        finally:
            self.visiting.discard(key)

        self.cache[key] = result
        return result


def formula_reference_exceptions(rows):
    affected = set()
    broken = set()
    pattern = re.compile(r"(?<![A-Za-z0-9_!$])\$?([A-Z]{1,3})\$?(\d+)")
    for row_number, row in enumerate(rows, 1):
        for index in range(9, 17):
            if index >= len(row) or not row[index]:
                continue
            formula = FormulaReplay._entered(row[index]).get("formulaValue")
            if not isinstance(formula, str):
                continue
            cell = (row_number, chr(ord("A") + index))
            if "#REF!" in formula:
                broken.add(cell)
                affected.add(cell)
            for match in pattern.finditer(formula):
                column, reference_row = match.group(1), int(match.group(2))
                if column in "ABCDEFGHIJKLMNOPQ" and reference_row != row_number:
                    affected.add(cell)
    return affected, broken


def matches(actual, expected):
    if expected is ERROR:
        return actual is ERROR
    if actual is ERROR:
        return False
    if isinstance(expected, str):
        if expected == "":
            return actual in (None, "")
        return isinstance(actual, str) and expected.casefold() == actual.casefold()
    if isinstance(expected, (int, float)):
        return (
            isinstance(actual, (int, float))
            and not isinstance(actual, bool)
            and math.isclose(
                float(actual), float(expected), rel_tol=2e-12, abs_tol=2e-12
            )
        )
    return actual is None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--input",
        required=True,
        type=Path,
        help="finance-export.json or its containing export directory",
    )
    args = parser.parse_args()
    path = args.input / "finance-export.json" if args.input.is_dir() else args.input
    try:
        archive = json.loads(path.read_text())
        replay = FormulaReplay(archive)
        rows = replay.transactions
        counts = Counter()
        errors = Counter()
        mismatches = []
        formula_counts = Counter()
        for row_number, row in enumerate(rows, 1):
            for column in OUTPUT_COLUMNS:
                cell = replay._cell(column, row_number)
                if not isinstance(replay._entered(cell).get("formulaValue"), str):
                    continue
                formula_counts[column] += 1
                expected = replay.evaluate(column, row_number)
                actual = replay._value(cell)
                if expected is ERROR:
                    if actual is ERROR:
                        errors[column] += 1
                    else:
                        mismatches.append(f"Transactions!{column}{row_number}")
                elif matches(actual, expected):
                    counts[column] += 1
                else:
                    mismatches.append(f"Transactions!{column}{row_number}")

        exceptions, broken = formula_reference_exceptions(rows)
        report = {
            "formulaCells": sum(formula_counts.values()),
            "formulaCellsByColumn": dict(sorted(formula_counts.items())),
            "matchedFormulaResults": sum(counts.values()),
            "matchedSourceErrors": sum(errors.values()),
            "sourceErrorsByColumn": dict(sorted(errors.items())),
            "formulaReferenceExceptionCells": len(exceptions),
            "formulaReferenceExceptionRows": len({row for row, _ in exceptions}),
            "brokenReferenceCells": len(broken),
            "mismatchCount": len(mismatches),
            "mismatchCells": mismatches[:20],
        }
        print(json.dumps(report, indent=2, sort_keys=True))
        return 1 if mismatches else 0
    except (KeyError, OSError, ValueError, json.JSONDecodeError) as error:
        print(f"formula replay failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
