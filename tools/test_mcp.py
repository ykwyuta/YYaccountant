"""Exercise the real stdio executable with isolated SQLite files.

Usage: python tools/test_mcp.py target/release/yyaccountant
"""
from contextlib import closing
from decimal import Decimal
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile


def exchange(executable, database, messages, read_only=False):
    args = [str(executable), "--mcp", str(database)] + (["--read-only"] if read_only else [])
    result = subprocess.run(args, input="".join(json.dumps(m, ensure_ascii=False) + "\n" for m in messages),
                            text=True, encoding="utf-8", capture_output=True, timeout=60, check=True)
    assert not result.stderr, result.stderr
    return {row["id"]: row for row in map(json.loads, result.stdout.splitlines())}


def request(identifier, method, params=None):
    return {"jsonrpc": "2.0", "id": identifier, "method": method, "params": params or {}}


def initialize():
    return [request(1, "initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                                      "clientInfo": {"name": "yyaccountant-test", "version": "1"}}),
            {"jsonrpc": "2.0", "method": "notifications/initialized"}]


def tool(identifier, name, arguments):
    return request(identifier, "tools/call", {"name": name, "arguments": arguments})


def data(rows, identifier):
    result = rows[identifier]["result"]
    assert not result["isError"], result["content"][0]["text"]
    return result["structuredContent"]


def main():
    executable = Path(sys.argv[1]).resolve()
    with tempfile.TemporaryDirectory(prefix="yyaccountant-mcp-") as directory:
        database = Path(directory) / "日本語 家計簿.sqlite3"
        rows = exchange(executable, database, initialize() + [
            request(2, "tools/list"),
            tool(3, "book_create", {"name": "実接続の家計", "kind": "household", "actor": "test"}),
            tool(4, "account_guide", {"book_id": 1, "query": "スーパーで食材"}),
        ])
        assert rows[1]["result"]["protocolVersion"] == "2025-11-25"
        assert len(rows[2]["result"]["tools"]) == 16
        accounts = {a["code"]: a["id"] for a in data(rows, 3)["accounts"]}
        assert data(rows, 4)["suggestions"][0]["code"] == "501"
        food, bank, card = accounts["501"], accounts["111"], accounts["201"]
        changes = [
            {"operation": "quick_entry", "quick": {"kind": "expense", "date": "2026-10-01", "amount": "1980",
                                                   "account": food, "via": card, "description": "食材"}},
            {"operation": "quick_entry", "quick": {"kind": "transfer", "date": "2026-10-27", "amount": "1980",
                                                   "account": card, "via": bank, "description": "カード引落し"}},
        ]
        messages = initialize()
        for index, change in enumerate(changes):
            messages.append(tool(10 + index, "book_change", {"book_id": 1, "expected_revision": 1 + index,
                                                           "actor": "AI", "change": change}))
        messages += [
            tool(20, "book_change", {"book_id": 1, "expected_revision": 1, "actor": "stale", "change": changes[0]}),
            tool(21, "report", {"book_id": 1, "kind": "trial_balance", "to": "2026-12-31"}),
            tool(22, "entry_list", {"book_id": 1, "query": "食材"}),
            tool(23, "csv_export", {"book_id": 1}),
            tool(24, "audit_list", {"book_id": 1}),
            tool(25, "book_change", {"book_id": 1, "expected_revision": 3, "actor": "AI", "change": {
                "operation": "quick_entry", "quick": {"kind": "expense", "date": "2026-10-02", "amount": 19.8,
                                                      "account": food, "via": bank}}}),
        ]
        rows = exchange(executable, database, messages)
        assert data(rows, 11)["revision"] == 3
        assert rows[20]["result"]["isError"]
        balances = {r["code"]: r for r in data(rows, 21)["rows"]}
        assert balances["501"]["closing"] == "1980" and balances["201"]["closing"] == "0"
        assert balances["111"]["closing"] == "-1980"
        assert data(rows, 21)["total_debit"] == data(rows, 21)["total_credit"] == "3960"
        assert data(rows, 22)["total"] == 1
        assert data(rows, 23)["content"].startswith("﻿日付")
        assert [c["actor"] for c in data(rows, 24)["changes"]] == ["test", "AI", "AI"]
        assert rows[25]["result"]["isError"] and "文字列" in rows[25]["result"]["content"][0]["text"]
        readonly = exchange(executable, database, initialize() + [tool(30, "book_change", {
            "book_id": 1, "expected_revision": 3, "actor": "ro", "change": changes[0]})], True)
        assert readonly[30]["result"]["isError"]
        with closing(sqlite3.connect(database)) as connection:
            assert connection.execute("SELECT revision FROM books WHERE id=1").fetchone()[0] == 3
            types = {t for (t,) in connection.execute("SELECT typeof(debit) FROM entry_lines")}
            assert types == {"text"}, types

        demo = Path(directory) / "demo.sqlite3"
        subprocess.run([str(executable), "--init-demo", str(demo), "--today", "2026-10-29"], check=True,
                       capture_output=True, timeout=120)
        rows = exchange(executable, demo, initialize() + [
            tool(40, "book_list", {}),
            tool(41, "report", {"book_id": 1, "kind": "balance_sheet", "to": "2026-10-29"}),
            tool(42, "forecast", {"book_id": 1, "as_of": "2026-10-29", "months": 6}),
            tool(43, "claim_report", {"book_id": 1, "as_of": "2026-10-29"}),
            tool(44, "statistics", {"book_id": 1, "from": "2025-11-01", "to": "2026-10-31"}),
            tool(45, "cost_report", {"book_id": 2, "as_of": "2026-10-29"}),
            tool(46, "report", {"book_id": 2, "kind": "balance_sheet", "to": "2026-10-29"}),
            tool(47, "budget_report", {"book_id": 1, "budget_id": 1}),
            tool(48, "plan_calendar", {"book_id": 1, "from": "2026-10-01", "to": "2027-01-31"}),
            tool(49, "report", {"book_id": 2, "kind": "income_statement", "from": "2026-01-01", "to": "2026-10-29"}),
        ])
        assert [b["kind"] for b in data(rows, 40)["books"]] == ["household", "sole_proprietor"]
        for i in (41, 46):
            assert data(rows, i)["balanced"], data(rows, i)
        forecast = data(rows, 42)
        assert forecast["months"][0] == "2026-11" and len(forecast["totals"]) == 6
        for total in forecast["totals"]:
            assert Decimal(total["expense_low"]) <= Decimal(total["expense"]) <= Decimal(total["expense_high"])
        fridge = [a for a in forecast["accounts"] if a["code"] == "641"][0]
        assert Decimal(fridge["months"][1]["scheduled"]) == Decimal("168000")
        claims = data(rows, 43)["claims"]
        assert any(c["bucket"] != "current" and c["side"] == "receivable" for c in claims)
        cost = data(rows, 45)
        assert len(cost["products"]) == 2 and cost["productions"]
        assert all(Decimal(s["quantity"]) >= 0 for s in cost["stock"]), cost["stock"]
        assert data(rows, 47)["rows"]
        assert any(not o["posted"] for o in data(rows, 48)["occurrences"])
        print(json.dumps({
            "household_forecast": forecast["totals"][:3],
            "savings_rate": data(rows, 44)["savings_rate"],
            "bakery_net_income": data(rows, 49)["net_income"],
            "bakery_inventory": cost["inventory_value"],
            "standard_costs": {p["name"]: p["standard"]["total"] for p in cost["products"]},
        }, ensure_ascii=False, indent=1))
    print("MCP stdio validation passed")


if __name__ == "__main__":
    main()
