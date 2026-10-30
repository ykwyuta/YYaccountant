# YYaccountant 開発文書

## 範囲

| 提供するもの | 備考 |
|---|---|
| `yyaccountant --web` | パスワード認証のWeb UI（Linux／Windows） |
| `yyaccountant --mcp` | stdio MCPサーバー（Linux／Windows） |
| `--init-demo` `--backup` `--generate-password` | 補助コマンド |
| `yyaccountantw` | Windowsネイティブ画面（Web UIと同じ機能。グラフは表で代える） |

ネイティブ画面は標準のWin32コントロールだけで作り、YYprojectが使うyyeditorの `yy-ui-win`（Direct2D描画の表・エディター）には依存しません。Streamable HTTP MCP、PostgreSQLはこの版では提供しません。データは要件どおり1つのSQLiteファイルにまとめています。

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
  tax.rs       年分ごとの金額基準（Rules）と所得税・住民税の計算
  lifeplan.rs  キャッシュフロー表、比較、6つの係数、ローン返済額、教育費、帳簿からの雛形、CSV
  tutorial.rs  チュートリアル（Web UI・ネイティブ画面・MCP・docs/user/tutorial.md の共通の内容）
  study.rs     問題集：4つのシナリオ（帳簿をメモリー上で組み立て）、問題と正解、採点と指摘、進み具合、docs/user/study.md の生成
app/src
  main.rs      CLI
  mcp.rs       MCPツール（Web UIも同じ `call` を使用）
  web.rs       HTTPサーバー、セッション、CSRF、Host/Origin検査、バックアップ
  auth.rs      パスワード（SHA-256ダイジェストだけ保存）
  demo.rs      決定的なデモデータ
  web/         埋め込みのHTML・CSS・JavaScript
  gui_main.rs  yyaccountantw（windows_subsystem、エラーはメッセージボックス）
  native.rs    ウィンドウ・メニュー・ページの配置・コマンド・定期読み込み
  native/sheets.rs  記帳4画面の表と見出しの中身（十進数のまま整形、窓なしで試験）
  native/records.rs 記録の画面（科目・取引先・債権債務・予算・計画・原価・統計・履歴）の表、フォーム、Change
  native/planning.rs 税金とライフプランの画面の表、2段階のフォーム、Change
  native/form.rs    帳簿と各記録の入力に使うモーダルフォーム（文字・選択肢・IDの選択）と、チュートリアルの読み取り専用の窓
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

`application_id = 0x59594143`（"YYAC"）、`user_version = 3`。他の形式のファイルは開かない。版1・版2のファイルは書き込み可能で開いたときに足りないテーブル（版2で `tax_returns` と `life_plans` と子テーブル、版3で `study_attempts`）を作って版3にする。読み取り専用で開いた古いファイルは変更せず、税額計算・ライフプラン・問題集の解答を空として読む。

- `books(id, name, kind, currency, minor_digits, rounding, fiscal_start_month, revision, next_ids, created_at)`
- 各コレクションは `(book_id, id)` を主キーとするテーブル: `accounts` `partners` `claims` `entries` `budgets` `plans` `materials` `material_moves` `equipment` `products` `productions` `tax_returns` `life_plans`
- 入れ子のリストは `(book_id, parent_id, seq)` の子テーブル: `entry_lines` `budget_lines` `product_materials` `product_equipment` `production_materials` `production_equipment` `tax_dependents` `life_plan_members` `life_plan_flows` `life_plan_events`
- `audit(id, book_id, at, actor, revision, change)`
- `study_attempts(id, problem, actor, at, correct, answer)`：問題集の解答（帳簿に属さない。`actor` ごとの進み具合に使う）

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

## 税額計算の仕様

- 金額基準は `tax::rules(year)` の表（`Rules`）で年分ごとに持つ。対応していない年分は保存も計算も拒否する（`tax::YEARS`）。2026年分は令和8年度税制改正大綱の内容（基礎控除は合計所得489万円以下104万円・655万円以下67万円・2,350万円以下62万円、給与所得控除の最低74万円は給与収入220万円まで、扶養親族・同一生計配偶者の所得要件62万円、23歳未満の扶養親族がいるときの一般生命保険料控除の上限6万円）。新しい年分は表を足し、手計算の試験を加える。
- 給与所得は給与収入660万円未満で4,000円単位の表（収入÷4を千円未満切り捨て×2.8−8万円 または ×3.2−44万円）に合わせる。所得金額調整控除（給与850万円超で23歳未満の扶養親族か特別障害者がいるとき）を引く。
- 副業: 雑所得の赤字は0、事業所得の赤字は給与と通算、青色申告特別控除は所得を限度とする。2か所目の給与は給与収入に合算する。
- 所得控除は所得税と住民税を並べて計算する（生命保険料・地震保険料・配偶者・扶養・特定親族特別控除・障害者・基礎控除の額が異なる）。端数は法定どおり（課税所得は1,000円未満、税額控除後の所得割と申告納税額は100円未満を切り捨て、生命保険料控除の算式は切り上げ）。
- 所得税: 速算表 → 住宅借入金等特別控除（年末残高と借入限度額の少ない方×控除率、100円未満切り捨て、合計所得2,000万円以下）→ 復興特別所得税2.1% → 源泉徴収税額との差。
- 住民税: 所得割10% → 調整控除（人的控除差。配偶者特別控除・特定親族特別控除の差は0とする概算）→ 住宅ローン控除の残り（所得税の課税所得の5%と上限の少ない方）→ 寄附金税額控除（基本分10%＋特例分。特例分の割合は課税所得から人的控除差を引いた額の所得税率で決め、調整控除後の所得割の2割まで）→ 均等割。
- ふるさと納税の上限の目安は「調整控除後の所得割×20%÷特例分の割合＋2,000円」。実質負担は寄附を0にした計算との税額の差から求める。

## ライフプランの仕様

- 金額は最初の年の価格で入力し、各年の値は `round(年額×(1＋変動率)^(年−最初の年))`（帳簿の端数処理）。期間の外の収支は0で、表の期間に1年もかからない収支は列を作らない。
- 年末の金融資産 = 前年末 + 前年末×利回り（前年末が正のときだけ、帳簿の端数処理）+ 収入 − 支出。最初の年の前年末は「最初の金融資産」。
- `compare` は2つのプランの年末の金融資産を年ごとに並べ、最終年の差と最少残高を返す。`coefficients` は `powi` による十進数で計算し小数6桁に丸める。`loan_payment` は元利均等（月利＝年利/12）の月額を帳簿の端数処理で丸めて12倍する。
- 教育費の目安（`EDUCATION`）は年末時点の年齢で段階を決める（幼稚園4〜6歳、小学校7〜12歳、中学校13〜15歳、高校16〜18歳、大学19〜22歳）。

## 問題集の仕様

- シナリオは `study.rs` の関数で、`Book::blank` から `Change` を順に `Book::apply` して帳簿を組み立てる（`Store::create` と同じ科目・IDの払い出し）。数値問題の正解は、その帳簿に対する `reports`・`tax::calculate`・`lifeplan`・`cost`・`claims` の結果から作り、解説の数値も同じ値を埋め込む。正解のずれは `core/tests/study.rs` の手計算値で検出する。
- 問題の形式は選択（選択肢ごとの解説つき）、仕訳（科目コードと借方・貸方）、数値（小数桁と単位。答えを四捨五入して比べる）。`ProblemView`・`ScenarioView` は正解と解説を含まず、解説は `grade` の結果でだけ返す。
- 仕訳の採点は科目ごとに借方−貸方を集計して比べ、足りない科目・余分な科目・借方と貸方の逆・金額の違い・左右の合計の不一致を指摘する。数値は差が1%以内なら端数処理を、符号だけ違えば符号を指摘する。
- 「練習用の帳簿」は `Store::create` の後に同じ `Change` 列を `Store::apply` するので、通常の検証・版・監査を通り、問題の前提と同じ帳簿になる（`core/tests/study.rs` で一致を確かめる）。
- 進み具合（`progress`）は解答の記録から、問題ごとの回数・一度でも正解・最後の解答の正否、シナリオ・分野ごとの正解数、最後に間違えたままの問題（復習）を求める。
- MCPの `study_list` `study_answer` `study_practice_book`。Web UIでは `study_list`・`study_answer` の `actor` をログインユーザーに置き換え、読み取り専用ユーザーの解答は採点だけして記録しない。ネイティブ画面には問題集の画面はまだない（チュートリアルの文章で案内だけする）。

## Web UI

`/` と `/web/*` だけを公開する。Hostの許可リスト、POSTのOrigin一致とContent-Type、ログイン後のHttpOnly・SameSite=Strictクッキーと `X-YY-CSRF` を検査する。CSPは `default-src 'self'`（インラインのスクリプト・スタイル不可）。`/web/api` は `{tool, arguments}` を受け取り `mcp::call` を呼ぶ。変更系（`book_create` `book_change` `study_practice_book`）は読み取り専用ユーザーを拒否し、`actor` をログインユーザー名で上書きする。「はじめに」と「問題集」は帳簿がなくても表示する。ログイン失敗は1分30回で制限する。

## ネイティブ画面

`app/src/native.rs` は全ページのコントロールを起動時に1度作り、ページの切り替えでは表示・非表示だけを変える。タブコントロールは見出しとして使い、コントロールはその下にフレームの子として置く（通知がフレームに届く）。状態は `thread_local` の `State` にあり、メッセージ処理は `try_borrow_mut` で取得する。処理中のコントロール更新が同期的に送る通知（`EN_CHANGE`・`LVN_ITEMCHANGED` など）はこれで無視される。メッセージボックスやファイル選択・フォームは状態を借りていない所で開く。

- 計算と検証はcoreのまま。金額の入力は `num::parse`（全角・桁区切り・円を受け付け、指数表記は拒否）、表示は `num::fixed` の結果に桁区切りを付けるだけで浮動小数点を使わない。
- 変更は画面に表示している版を `expected_revision` として `Store::apply` に渡す。`Error::Conflict` なら帳簿を読み込み直し、入力欄を残したまま知らせる。
- 1秒ごとに `PRAGMA data_version` を調べ、他の接続（MCP・Web UI・別のウィンドウ）が書き込んでいれば帳簿の一覧と版を比べて読み込み直す。自分の書き込みでは値が変わらない。
- 操作者は `USERNAME`（Windows）。読み取り専用で開くと書き込むボタンと帳簿メニューを無効にする。
- DPIはPerMonitorV2。寸法は96DPI基準の値を `GetDpiForWindow` で拡大し、`WM_DPICHANGED` でフォントと一覧の列幅を作り直す。最小サイズは860×600（96DPI換算）。画面が小さく最小サイズに届かないときは、1行に並べる欄を同じ比率で縮める。
- 税金とライフプランの画面は予算と同じく表示の切り替えで記録（年分・プラン）を選ぶ（`Section::chooser`）。項目が多い入力は2段階で、最初の1項目のフォーム（`Prepared::Choose`）で編集する部分を選び、`Context::part` を付けてもう一度 `prepare` する。ライフプランの表は列の見出しが家族や収支の名前になるので、`Table` の見出しは `String` で持つ。
- 5ページ目以降（記録の画面）は1組の共有コントロール（表示の切り替え、基準日・期間、一覧、詳細、要約、最大5つのボタン）を使う。何を表示し、どのボタンがどのフォームを開き、どの `Change` を作るかは `native/records.rs` の `Section`・`Spec`・`view`・`prepare`・`finish` で決める。フォームは状態を借りずに開き、閉じた後に同じ版で適用する。削除と償却の計上は確認してから適用する。計画の予定は詳細行のIDに計画IDと日付を、製品の構成は材料を正、設備を負のIDで持つ。
- 各入力欄の直前に静的テキストを作るので、スクリーンリーダーとUI Automationは見出しを入力欄の名前として読む。Tab移動は `IsDialogMessageW`、ページ切り替えとF5はアクセラレーター。

`yyaccountantw --ui-smoke-test DIR` は新しい `DIR/ui-test.sqlite3` に対して、実際のウィンドウへメッセージを送って操作する: 帳簿作成フォーム、貸借不一致の拒否と複合仕訳、全角金額のかんたん入力、JPYの小数の拒否、科目検索と使い分けの表示、取消・解除・検索、同じ番号での修正、別接続の変更の自動読み込みと古い版の保存の競合、4種類の帳票（試算表・貸借対照表の一致と元帳残高）、帳簿の設定、バックアップとCSV、個人事業主の事業割合、帳簿の切り替え、科目の追加・編集と取引先の追加・削除、債権の登録（発生の仕訳）と回収、予算の作成・科目の予算・外す、計画の計上と二重計上の拒否、統計・予測・変更履歴、原材料・仕入・設備（償却の計上）・製品の構成と標準原価・製造実績と在庫の払出し、税金（年分の追加、2段階のフォームでの入力、親族の追加と外す、計算結果の還付・住民税・ふるさと納税の税額控除の値）、ライフプラン（帳簿と手取りからの作成、家族・教育費・ローン返済額・イベントの追加、複製した提案との差、CSV）、6つの係数とチュートリアル、最小未満・最小・大きいサイズでの全ページの配置（はみ出しと重なり）、DPI変更、読み取り専用。結果を `DIR/ui-smoke.txt`、画面を `DIR/page1〜13.bmp` に書き、失敗時は `DIR/error.txt` に理由を書いて終了コード1で終わる。確認のためのメッセージボックスは出さず記録する。

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
- `core/src/tax.rs` の試験: 給与所得の表、生命保険料控除の新旧と特例、副業・配偶者・子・同居の父母・医療費・住宅ローン・ふるさと納税のある世帯の所得税・住民税（手計算値）、令和8年分の178万円の壁。`core/src/lifeplan.rs` の試験: 変動率・運用益・資金不足の年・比較・CSV・6つの係数・ローン返済額・教育費。`core/tests/books.rs`: 版1のファイルの移行と、税額計算・ライフプランの保存と読み込み。
- `core/tests/study.rs`: 4シナリオ・55問の組み立て、正解の手計算値（給与所得の別表第五の4,000円単位、医療費控除の所得5%、住宅ローン控除の所得税・住民税の配分、元利均等返済、移動平均・標準原価・差異など）、採点の指摘、練習用の帳簿の再現、解答の記録と進み具合、版2のファイルの移行、問題の表示に正解が含まれないこと。
- `core/tests/docs.rs`: `docs/user/accounts.md` が科目表から、`docs/user/tutorial.md` がチュートリアルから、`docs/user/study.md` が問題集から生成した内容と一致すること。
- `tools/test_mcp.py`: 実プロセスのstdioで全体を操作し、デモDBの貸借一致・予測・延滞・原価、税額計算（住宅ローン控除でふるさと納税の実質負担が2,000円を超える例を含む）、ライフプランと比較・CSV、係数・ローン、チュートリアル、未対応の年分の拒否、問題集（正解を含まない一覧、仕訳の指摘、進み具合と復習、練習用の帳簿の貸借一致と住宅ローン控除、読み取り専用での採点のみ）を確認。
- `tools/test_native.py`: `--ui-smoke-test` を実プロセスで終了まで待って実行し、作られたSQLiteを別に読んで、金額の列が全行 `text`、取消していない仕訳が十進数で貸借一致、監査の操作者と操作の種類、修正した仕訳の番号を確認する。結果と画面（PNG）を `target/native-validation/` に残す。
- `tools/test_web.py`: 実サーバーでHost/Origin/CSRF/読み取り専用/競合/監査/バックアップと、問題集の解答者がログインユーザーになること（読み取り専用ユーザーは記録しない）を確認。`--browser` でChromiumを使い、両帳簿の全画面、科目検索からの入力と家事按分、明細3行の仕訳（JPYの小数を拒否→修正して保存）、チュートリアルから税金の画面へ、親族の追加と寄附額の変更、ライフプランの比較・教育費の追加・CSVのダウンロード、問題集（仕訳の誤答の指摘→修正して正解、全角数字の数値問題、選択肢ごとの解説、復習の絞り込み、練習用の帳簿の作成と税金の画面）、グラフのツールチップ、ダーク表示、スマートフォン幅の横はみ出しを確認し、コンソールエラー（CSP違反を含む）があれば失敗する。画面は `target/web-validation/` に保存する。

デモDB `demo/yyaccountant-demo.sqlite3` は `yyaccountant --init-demo demo/yyaccountant-demo.sqlite3 --today 2026-10-29` で再生成できる（乱数を使わず決定的）。
