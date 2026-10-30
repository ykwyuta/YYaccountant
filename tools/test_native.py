"""Run the native Windows UI smoke test as a real process and check the file it wrote.

Usage: python tools/test_native.py target/release/yyaccountantw.exe

`yyaccountantw --ui-smoke-test DIR` drives its own window (forms, quick input, journal,
corrections, conflicts, reports, read-only, layout at several sizes and DPI) against a
new database in DIR. This script waits for the process to exit, then reads the SQLite
file independently: amounts are TEXT and every live entry balances in decimal.
Results, screenshots (PNG when Pillow is available) and the database stay in
target/native-validation/.
"""
from contextlib import closing
from decimal import Decimal
from pathlib import Path
import json
import shutil
import sqlite3
import subprocess
import sys


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    executable = Path(sys.argv[1]).resolve()
    out = Path("target/native-validation").resolve()
    shutil.rmtree(out, ignore_errors=True)
    result = subprocess.run([str(executable), "--ui-smoke-test", str(out)], timeout=180)
    report = out / "ui-smoke.txt"
    if report.exists():
        print(report.read_text(encoding="utf-8"), end="")
    if result.returncode != 0:
        error = out / "error.txt"
        raise SystemExit(f"UI smoke failed ({result.returncode}): "
                         + (error.read_text(encoding="utf-8") if error.exists() else "no error.txt"))

    with closing(sqlite3.connect(out / "ui-test.sqlite3")) as db:
        kinds = {kind for (kind,) in db.execute(
            "SELECT DISTINCT typeof(debit) FROM entry_lines UNION SELECT DISTINCT typeof(credit) FROM entry_lines")}
        assert kinds == {"text"}, kinds
        books = dict(db.execute("SELECT name, kind FROM books"))
        assert books == {"UI検証 家計（改名）": "household", "UI検証 事業": "sole_proprietor",
                         "学習：佐藤健さんの副業開業": "sole_proprietor"}, books
        for book, entry, debit, credit in db.execute(
                "SELECT e.book_id, e.id, group_concat(l.debit, ' '), group_concat(l.credit, ' ') "
                "FROM entries e JOIN entry_lines l ON l.book_id = e.book_id AND l.parent_id = e.id "
                "WHERE NOT e.void GROUP BY e.book_id, e.id"):
            total = lambda values: sum((Decimal(v) for v in values.split()), Decimal(0))
            assert total(debit) == total(credit) > 0, (book, entry, debit, credit)
        actors = {actor for (actor,) in db.execute("SELECT DISTINCT actor FROM audit")}
        assert actors == {"UI-test", "AI-test"}, actors
        changes = [c for (c,) in db.execute("SELECT json_extract(change, '$.operation') FROM audit")]
        for operation in ["create_book", "save_entry", "quick_entry", "void_entry", "update_book"]:
            assert operation in changes, operation
        # The corrected opening entry kept its number; the voided-and-restored one is live.
        assert db.execute("SELECT description FROM entries WHERE book_id = 1 AND id = 1").fetchone() == (
            "開始残高（修正）",)
        assert db.execute("SELECT count(*) FROM entries WHERE void").fetchone() == (0,)
        # Tax inputs and life plans are stored as decimal TEXT in their own tables.
        for table, column in [("tax_returns", "salary"), ("tax_dependents", "income"),
                              ("life_plans", "savings"), ("life_plan_flows", "amount")]:
            kinds = {kind for (kind,) in db.execute(f"SELECT DISTINCT typeof({column}) FROM {table}")}
            assert kinds == {"text"}, (table, kinds)
        typed = "SELECT id FROM books WHERE name NOT LIKE '学習：%'"
        assert db.execute(f"SELECT salary, count(*) FROM tax_returns WHERE book_id IN ({typed})").fetchone() == (
            "6000000", 1)
        assert db.execute("SELECT count(*) FROM life_plans").fetchone() == (2,)
        for operation in ["save_tax_return", "save_life_plan"]:
            assert operation in changes, operation
        # The practice book replays the scenario through ordinary audited changes.
        practice = "SELECT id FROM books WHERE name = '学習：佐藤健さんの副業開業'"
        assert db.execute(f"SELECT salary, side_kind FROM tax_returns WHERE book_id IN ({practice})").fetchall() == [
            ("5200000", "blue_business")]
        assert db.execute(f"SELECT count(*) > 30 FROM audit WHERE book_id IN ({practice})").fetchone() == (1,)
        # Problem-set answers: four graded attempts; the read-only one was not recorded.
        attempts = db.execute("SELECT actor, problem, correct, answer FROM study_attempts ORDER BY id").fetchall()
        assert [(a, p, c) for a, p, c, _ in attempts] == [
            ("UI-test", "aoki-02", 0), ("UI-test", "aoki-02", 1), ("UI-test", "aoki-06", 1),
            ("UI-test", "aoki-01", 0)], attempts
        assert json.loads(attempts[2][3]) == {"number": "435,600"}, attempts[2]
        assert db.execute("PRAGMA user_version").fetchone() == (3,)

    try:
        from PIL import Image
    except ImportError:
        Image = None
    shots = sorted(out.glob("page*.bmp"))
    if Image:
        for shot in shots:
            Image.open(shot).convert("RGB").save(shot.with_suffix(".png"))
            shot.unlink()
    print(f"native UI ok: database checked, {len(shots)} screenshots in {out}")


if __name__ == "__main__":
    main()
