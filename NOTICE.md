# 素材と依存

- `app/src/mcp.rs` のstdioトランスポート、`app/src/auth.rs` のパスワード生成・照合、`app/src/web.rs` のセッション・CSRF・Host/Origin検査は [YYproject](https://github.com/ykwyuta/yyproject)（GPL-3.0-or-later）の実装を基に、YYaccountant用に整理しています。
- 十進数の計算に [rust_decimal](https://crates.io/crates/rust_decimal)（MIT）を使用します。
- SQLiteはrusqliteのbundled機能で同梱します。Web UIはaxum・tokioを使用します。Rust依存の正確な版と取得元は `Cargo.lock` を参照してください。
- Web UIのグラフ配色は色覚の違いを考慮して検証した2色（収入: 青、支出: 橙）を使用し、ライト・ダークそれぞれの背景で確認しています。
