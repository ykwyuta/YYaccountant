# YYaccountant 開発文書

## 範囲

| 提供するもの | 備考 |
|---|---|
| `yyaccountant --web` | パスワード認証のWeb UI（Linux／Windows） |
| `yyaccountant --mcp` | stdio MCPサーバー（Linux／Windows） |
| `--init-demo` `--backup` `--generate-password` | 補助コマンド |
| `yyaccountantw` | Windowsネイティブ画面（かんたん入力・仕訳帳・仕訳入力・帳票、帳簿の作成と設定、バックアップ、仕訳CSV） |

ネイティブ画面は標準のWin32コントロールだけで作り、YYprojectが使うyyeditorの `yy-ui-win`（Direct2D描画の表・エディター）には依存しません。債権債務・予算・計画・原価・統計・科目と取引先の編集はWeb UIとMCPだけにあります。Streamable HTTP MCP、PostgreSQLはこの版では提供しません。データは要件どおり1つのSQLiteファイルにまとめています。

## 構成

```
core/src
  num.rs       十進数の解析・丸め・統計（平均・中央値・標準偏差・回帰）、serdeの文字列表現
  model.rs     帳簿モデルと全体検証
  chart.rs     家計・個人事業主の科目表、使い分け（Distinction）、科目検索、利用者文書の生成
  change.rs    Change（原子的な変更）とその適用
  store.rs     SQLite（テーブル定義、差分書き込み、版の競合検出、監査）
  reports.rs   試算表・損益計算書・貸借対照表・元帳・仕訳一覧・統計・CSV
  claims.rs    債権債務の返済予定・延滞・エイジング
  budget.rs    予算と実績
  plans.rs     計画の予定日
  cost.rs      移動平均の在庫評価、減価償却、時間当たり費用、標準原価、実際原価と差異
  forecast.rs  統計的予測と予定の加算
app/src
  main.rs      CLI
  mcp.rs       MCPツール（Web UIも同じ `call` を使用）
  web.rs       HTTPサーバー、セッション、CSRF、Host/Origin検査、バックアップ
  auth.rs      パスワード（SHA-256ダイジェストだけ保存）
  demo.rs      決定的なデモデータ
  web/         埋め込みのHTML・CSS・JavaScript
  gui_main.rs  yyaccountantw（windows_subsystem、エラーはメッセージボックス）
  native.rs    ウィンドウ・メニュー・ページの配置・コマンド・定期読み込み
  native/sheets.rs  表と見出しの中身（十進数のまま整形、窓なしで試験）
  native/form.rs    帳簿の作成・設定に使う小さなモーダルフォーム
  native/smoke.rs   --ui-smoke-test の自己検証
app/build.rs   Windowsマニフェスト（コモンコントロールv6、PerMonitorV2）とアイコンの埋め込み
```

## 十進数

- 計算はすべて `rust_decimal::Decimal`（28桁）。保存は正規化したTEXT（例: `"1234.5"`）。
- JSON入力は文字列または整数。小数を含むJSON数値は `visit_f64` で拒否する。全角数字・桁区切り・「円」を受け付け、指数表記は拒否する。
- 金額は帳簿の `minor_digits`（0〜4）を超える小数を拒否する。単価・率は小数10桁まで。
- 帳簿の `rounding`（四捨五入／切り捨て／切り上げ／銀行丸め）を、按分・利息・償却・統計値の丸めに使う。原価の単価・行は小数4桁で保持し、合計を通貨の桁で丸める。
- 標準偏差は `Decimal::sqrt`（`maths` 機能）。予測の幅の係数1.96も十進数。
- ブラウザーは入力中の借方・貸方合計をBigInt固定小数点（小数12桁）で計算する。グラフの座標だけ `Number` を使う。

## SQLiteスキーマ

`application_id = 0x59594143`（"YYAC"）、`user_version = 1`。他の形式のファイルは開かない。

- `books(id, name, kind, currency, minor_digits, rounding, fiscal_start_month, revision, next_ids, created_at)`
- 各コレクションは `(book_id, id)` を主キーとするテーブル: `accounts` `partners` `claims` `entries` `budgets` `plans` `materials` `material_moves` `equipment` `products` `productions`
- 入れ子のリストは `(book_id, parent_id, seq)` の子テーブル: `entry_lines` `budget_lines` `product_materials` `product_equipment` `production_materials` `production_equipment`
- `audit(id, book_id, at, actor, revision, change)`

列は `store.rs` の `SPECS` で定義し、serdeのJSON表現と列を相互に変換する。書き込み時は前後のJSONを比較し、変わったレコードだけを削除・挿入する。レコードのキーと列定義が一致しなければ保存を中止する（モデルと定義のずれを検出）。IDは帳簿ごとに `next_ids` で払い出し、削除後も再利用しない。

## 更新の流れ

1. `BEGIN IMMEDIATE`
2. 帳簿を読み込み、`expected_revision` と比較（不一致は `Error::Conflict`、Web UIでは409）
3. 複製に `Change` を適用し、帳簿全体を検証（科目・取引先・債権債務の参照、仕訳の貸借一致、金額の桁、計画・原価の整合性、自動計上キーの重複）
4. 版を1増やし、差分を書き込み、監査に変更内容と作成・更新したIDを記録
5. `COMMIT`

自動計上の仕訳は `source` を持つ: `plan:{計画}:{予定日}`、`settle:{債権債務}:{連番}`、`claim:{債権債務}:opening`、`depreciation:{設備}:{年度}`。取消していない仕訳で同じキーは1件だけ。

## 予測の仕様

- 履歴: 基準日の前月まで、`history_months` か月（最初の仕訳の月より前は除く）。`plan:` と `settle:` の仕訳は除外する。
- 予測月: 基準日の翌月から。履歴の最終月から見て2か月先が最初の予測月（`h = k + 2`）。
- 方法: 移動平均（直近6か月）、線形回帰、季節（`n ≥ 13`、比率は0.5〜2に制限）。自動選択は1か月先予測の平均絶対誤差（2点以上）で比較。負の予測は0にする。
- 幅: 選んだ方法の誤差の二乗平均平方根×1.96。誤差が計算できないときは履歴の標準偏差。
- 予定: 未計上の計画のうち費用（借方）・収益（貸方）の科目を加算。債権債務の未済の返済予定の利息を支払利息・受取利息の役割の科目に加算し、元本は `planned_payments`／`planned_receipts` に分ける。

## Web UI

`/` と `/web/*` だけを公開する。Hostの許可リスト、POSTのOrigin一致とContent-Type、ログイン後のHttpOnly・SameSite=Strictクッキーと `X-YY-CSRF` を検査する。CSPは `default-src 'self'`（インラインのスクリプト・スタイル不可）。`/web/api` は `{tool, arguments}` を受け取り `mcp::call` を呼ぶ。変更系（`book_create` `book_change`）は読み取り専用ユーザーを拒否し、`actor` をログインユーザー名で上書きする。ログイン失敗は1分30回で制限する。

## ネイティブ画面

`app/src/native.rs` は全ページのコントロールを起動時に1度作り、ページの切り替えでは表示・非表示だけを変える。タブコントロールは見出しとして使い、コントロールはその下にフレームの子として置く（通知がフレームに届く）。状態は `thread_local` の `State` にあり、メッセージ処理は `try_borrow_mut` で取得する。処理中のコントロール更新が同期的に送る通知（`EN_CHANGE`・`LVN_ITEMCHANGED` など）はこれで無視される。メッセージボックスやファイル選択・フォームは状態を借りていない所で開く。

- 計算と検証はcoreのまま。金額の入力は `num::parse`（全角・桁区切り・円を受け付け、指数表記は拒否）、表示は `num::fixed` の結果に桁区切りを付けるだけで浮動小数点を使わない。
- 変更は画面に表示している版を `expected_revision` として `Store::apply` に渡す。`Error::Conflict` なら帳簿を読み込み直し、入力欄を残したまま知らせる。
- 1秒ごとに `PRAGMA data_version` を調べ、他の接続（MCP・Web UI・別のウィンドウ）が書き込んでいれば帳簿の一覧と版を比べて読み込み直す。自分の書き込みでは値が変わらない。
- 操作者は `USERNAME`（Windows）。読み取り専用で開くと書き込むボタンと帳簿メニューを無効にする。
- DPIはPerMonitorV2。寸法は96DPI基準の値を `GetDpiForWindow` で拡大し、`WM_DPICHANGED` でフォントと一覧の列幅を作り直す。最小サイズは860×600（96DPI換算）。画面が小さく最小サイズに届かないときは、1行に並べる欄を同じ比率で縮める。
- 各入力欄の直前に静的テキストを作るので、スクリーンリーダーとUI Automationは見出しを入力欄の名前として読む。Tab移動は `IsDialogMessageW`、ページ切り替えとF5はアクセラレーター。

`yyaccountantw --ui-smoke-test DIR` は新しい `DIR/ui-test.sqlite3` に対して、実際のウィンドウへメッセージを送って操作する: 帳簿作成フォーム、貸借不一致の拒否と複合仕訳、全角金額のかんたん入力、JPYの小数の拒否、科目検索と使い分けの表示、取消・解除・検索、同じ番号での修正、別接続の変更の自動読み込みと古い版の保存の競合、4種類の帳票（試算表・貸借対照表の一致と元帳残高）、帳簿の設定、バックアップとCSV、個人事業主の事業割合、帳簿の切り替え、最小と大きいサイズでの全ページの配置（はみ出しと重なり）、DPI変更、読み取り専用。結果を `DIR/ui-smoke.txt`、画面を `DIR/page1〜4.bmp` に書き、失敗時は `DIR/error.txt` に理由を書いて終了コード1で終わる。確認のためのメッセージボックスは出さず記録する。

## 検証

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --release --locked
python tools/test_mcp.py target/release/yyaccountant
NODE_PATH=... python tools/test_web.py target/release/yyaccountant --browser
python tools/test_native.py target/release/yyaccountantw.exe   # Windowsのみ
```

- `core/tests/books.rs`: 十進数の保存（SQLiteの型がtext）、貸借不一致・桁超過・JSON小数の拒否、競合と監査、取消とIDの非再利用、家事按分と財務諸表、返済予定（元利均等の検算）と延滞、年度予算、計画の二重計上防止と予測、原価（移動平均・償却・標準原価・差異の手計算値）、科目検索。
- `core/tests/docs.rs`: `docs/user/accounts.md` が科目表から生成した内容と一致すること。
- `tools/test_mcp.py`: 実プロセスのstdioで全体を操作し、デモDBの貸借一致・予測・延滞・原価を確認。
- `tools/test_native.py`: `--ui-smoke-test` を実プロセスで終了まで待って実行し、作られたSQLiteを別に読んで、金額の列が全行 `text`、取消していない仕訳が十進数で貸借一致、監査の操作者と操作の種類、修正した仕訳の番号を確認する。結果と画面（PNG）を `target/native-validation/` に残す。
- `tools/test_web.py`: 実サーバーでHost/Origin/CSRF/読み取り専用/競合/監査/バックアップを確認。`--browser` でChromiumを使い、両帳簿の全画面、科目検索からの入力と家事按分、明細3行の仕訳（JPYの小数を拒否→修正して保存）、グラフのツールチップ、ダーク表示、スマートフォン幅の横はみ出しを確認し、コンソールエラー（CSP違反を含む）があれば失敗する。画面は `target/web-validation/` に保存する。

デモDB `demo/yyaccountant-demo.sqlite3` は `yyaccountant --init-demo demo/yyaccountant-demo.sqlite3 --today 2026-10-29` で再生成できる（乱数を使わず決定的）。
