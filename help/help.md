# YYaccountant

家計から個人事業主までの収支・債権債務・予算・計画・原価・支出予測を、1つのSQLiteファイルで管理します。金額・数量・率の計算はすべて十進数で行います。

## 起動

```sh
# Web UI（ブラウザーで http://127.0.0.1:8750/ を開く）
yyaccountant --generate-password あなたの名前            # パスワードを1回だけ表示
yyaccountant --web books.sqlite3

# AI（MCP stdio）
yyaccountant --mcp books.sqlite3 [--read-only]

# デモ（家計と個人事業主のパン工房、約18か月分）
yyaccountant --init-demo NEW_DATABASE [--today YYYY-MM-DD]

# バックアップ（一貫したコピーを新しいファイルに作成）
yyaccountant --backup books.sqlite3 NEW_FILE
```

`--web` の主なオプション:

- `--listen IP:PORT`（既定 127.0.0.1:8750）。ループバック以外で待ち受けるときは認証設定の `server.allowed_hosts` が必要です。
- `--auth-config FILE`（既定 `yyaccountant-server.toml`）。`--generate-password USER [--read-only]` で作成・更新します。パスワードはSHA-256ダイジェストだけを保存します。
- `--read-only` 書き込みを禁止します。

## MCP設定の例

```json
{
  "mcpServers": {
    "yyaccountant": {
      "command": "/path/to/yyaccountant",
      "args": ["--mcp", "/path/to/books.sqlite3"]
    }
  }
}
```

ツール: `book_list` `book_create` `book_get` `book_change` `entry_list` `account_guide` `report` `statistics` `forecast` `budget_report` `claim_report` `plan_calendar` `cost_report` `csv_export` `audit_list` `metadata`。

- 金額は `"1234.5"` のような文字列で渡します。JSONの小数（`1234.5`）は誤差を避けるため拒否します。
- 変更は `book_change` に `expected_revision`（`book_get` の `revision`）を付けます。他の利用者が先に更新していれば上書きせずに失敗します。
- 科目に迷ったら `account_guide` に内容（例: 「ガソリン」「打合せのコーヒー」）を渡すと、候補と使い分けの説明を返します。

詳しい使い方は `docs/user/yyaccountant.md` を参照してください。
