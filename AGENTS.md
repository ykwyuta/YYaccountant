# 作業ルール

- このリポジトリは `core`（帳簿モデル・計算・SQLite）と `app`（CLI・stdio MCP・Web UI）のCargoワークスペース。変更前に `.github/workflows/ci.yml` を読む。
- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked` を実行する。警告を広いallowや試験の除外で隠さない。
- 実プロセスで `python tools/test_mcp.py target/release/yyaccountant` と `python tools/test_web.py target/release/yyaccountant --browser`（PlaywrightのChromium）を確認する。UI試験は固定sleepを使わず実状態で待つ。
- 金額・数量・率・統計の計算に浮動小数点を使わない（`rust_decimal`）。JSONでは文字列で受け渡し、SQLiteにはTEXTで保存する。ブラウザーで行う計算は入力確認用のBigInt固定小数点と、グラフの座標だけにする。
- 仕訳は借方合計と貸方合計の一致を必ず検証する。仕訳は削除せず取消で残す。自動計上（計画・回収返済・減価償却）は `source` キーで二重計上を防ぐ。
- SQLite更新ではトランザクション、帳簿の版による競合検出、監査記録を維持する。データは1つのSQLiteファイルにまとめ、外部ファイルに分けない。
- 勘定科目の説明を変えるときは、似た科目との使い分け（`core/src/chart.rs` の `Distinction`）と例も合わせて直す。税制の金額基準や期限は変わるため、断定せず確認を促す書き方にする。
- 操作説明は `docs/user` と `help`、現行内部仕様は `docs/developer`、提案・検証記録は `docs/history` に置く。READMEは入口にする。
- Web UIはCSP（`style-src 'self'`）を守る。style属性を使わず、CSSOMかクラスで指定する。
