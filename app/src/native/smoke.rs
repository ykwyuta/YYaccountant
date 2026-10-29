//! `yyaccountantw --ui-smoke-test DIR`: drives the real window through its messages
//! against a new database in DIR, writes `ui-smoke.txt` and screenshots, and fails on
//! the first unexpected state. Only app-owned windows and the new database are touched.
use super::*;
use std::path::Path;
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
use yy_accountant::num;

fn state<R>(f: impl FnOnce(&mut State) -> R) -> Result<R> {
    with(f).ok_or_else(|| "Window state is unavailable".into())
}
fn handle(id: u16) -> Result<HWND> {
    state(|s| s.c(id))
}
fn check(ok: bool, what: &str, log: &mut Vec<String>) -> Result<()> {
    if !ok {
        return Err(format!("UI smoke failed: {what}").into());
    }
    log.push(format!("ok  {what}"));
    Ok(())
}
fn messages() -> Vec<String> {
    SMOKE_MESSAGES.with(|m| m.borrow_mut().drain(..).collect())
}
/// Sends what a click on the button sends to the frame.
fn click(h: HWND, id: u16) -> Result<()> {
    let button = handle(id)?;
    send(
        h,
        WM_COMMAND,
        (BN_CLICKED as usize) << 16 | id as usize,
        button.0 as isize,
    );
    Ok(())
}
fn menu(h: HWND, id: u16) {
    send(h, WM_COMMAND, id as usize, 0);
}
fn type_into(id: u16, value: &str) -> Result<()> {
    // SetWindowText sends EN_CHANGE to the frame like typing does.
    set_text(handle(id)?, value);
    Ok(())
}
/// Picks the first item whose text contains `wanted` and notifies like a user choice.
fn choose(h: HWND, id: u16, wanted: &str) -> Result<()> {
    let combo = handle(id)?;
    let count = send(combo, CB_GETCOUNT, 0, 0).0.max(0) as usize;
    for i in 0..count {
        let length = send(combo, CB_GETLBTEXTLEN, i, 0).0.max(0) as usize;
        let mut buffer = vec![0u16; length + 1];
        send(combo, CB_GETLBTEXT, i, buffer.as_mut_ptr() as isize);
        if String::from_utf16_lossy(&buffer[..length]).contains(wanted) {
            send(combo, CB_SETCURSEL, i, 0);
            send(
                h,
                WM_COMMAND,
                (CBN_SELCHANGE as usize) << 16 | id as usize,
                combo.0 as isize,
            );
            return Ok(());
        }
    }
    Err(format!("No choice containing {wanted} in control {id}").into())
}
fn select(id: u16, row: usize) -> Result<()> {
    // LVM_SETITEMSTATE makes the list view send LVN_ITEMCHANGED to the frame.
    select_row(handle(id)?, Some(row));
    Ok(())
}
fn rows(id: u16) -> Result<usize> {
    Ok(row_count(handle(id)?))
}
fn find_row(id: u16, column: usize, wanted: &str) -> Result<usize> {
    let list = handle(id)?;
    (0..row_count(list))
        .find(|r| cell(list, *r, column).contains(wanted))
        .ok_or_else(|| format!("No row with {wanted} in list {id}").into())
}
fn account(name: &str) -> Result<i64> {
    state(|s| {
        s.book
            .as_ref()
            .and_then(|b| b.accounts.iter().find(|a| a.name == name))
            .map(|a| a.id)
    })?
    .ok_or_else(|| format!("No account {name}").into())
}
fn entries() -> Result<Vec<Entry>> {
    state(|s| {
        s.book
            .as_ref()
            .map(|b| b.entries.clone())
            .unwrap_or_default()
    })
}
fn resize(h: HWND, width: i32, height: i32) {
    unsafe {
        let _ = SetWindowPos(h, None, 0, 0, width, height, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}
/// Every visible control lies inside the client area and no two inputs overlap.
fn layout_is_clean(h: HWND) -> Result<String> {
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(h, &mut client);
    }
    let visible: Vec<(u16, RECT, Kind)> = state(|s| {
        s.controls
            .iter()
            .filter(|(_, c)| unsafe { IsWindowVisible(c.hwnd).as_bool() })
            .map(|(id, c)| {
                let mut r = RECT::default();
                unsafe {
                    let _ = GetWindowRect(c.hwnd, &mut r);
                    let mut corners = [
                        POINT {
                            x: r.left,
                            y: r.top,
                        },
                        POINT {
                            x: r.right,
                            y: r.bottom,
                        },
                    ];
                    MapWindowPoints(None, Some(h), &mut corners);
                    r = RECT {
                        left: corners[0].x,
                        top: corners[0].y,
                        right: corners[1].x,
                        bottom: corners[1].y,
                    };
                }
                // A closed drop-down list only occupies its edit part.
                if c.kind == Kind::Combo {
                    r.bottom = r.top + s.px(26);
                }
                (*id, r, c.kind)
            })
            .collect()
    })?;
    for (id, r, _) in &visible {
        if r.left < 0
            || r.top < 0
            || r.right > client.right
            || r.bottom > client.bottom
            || r.right <= r.left
        {
            return Err(format!(
                "Control {id} at {:?} leaves the client area {:?}",
                (r.left, r.top, r.right, r.bottom),
                (client.right, client.bottom)
            )
            .into());
        }
    }
    for (i, (a, ra, _)) in visible.iter().enumerate() {
        for (b, rb, _) in &visible[i + 1..] {
            if *a == ID_TABS || *b == ID_TABS {
                continue;
            }
            let overlap = ra.left < rb.right
                && rb.left < ra.right
                && ra.top < rb.bottom
                && rb.top < ra.bottom;
            if overlap {
                return Err(format!("Controls {a} and {b} overlap").into());
            }
        }
    }
    Ok(format!(
        "{} controls inside {}x{}",
        visible.len(),
        client.right,
        client.bottom
    ))
}
/// Saves the window as a 32-bit BMP (PrintWindow renders it even while unfocused).
fn capture(h: HWND, path: &Path) -> Result<()> {
    unsafe {
        let mut r = RECT::default();
        GetWindowRect(h, &mut r)?;
        let (width, height) = (r.right - r.left, r.bottom - r.top);
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        let old = SelectObject(dc, bitmap.into());
        let printed = PrintWindow(h, dc, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool();
        SelectObject(dc, old);
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let lines = GetDIBits(
            dc,
            bitmap,
            0,
            height as u32,
            Some(pixels.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        );
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        if !printed || lines != height {
            return Err(format!("Cannot capture {}", path.display()).into());
        }
        let mut file = Vec::with_capacity(pixels.len() + 54);
        file.extend_from_slice(b"BM");
        file.extend_from_slice(&(54 + pixels.len() as u32).to_le_bytes());
        file.extend_from_slice(&[0; 4]);
        file.extend_from_slice(&54u32.to_le_bytes());
        file.extend_from_slice(&40u32.to_le_bytes());
        file.extend_from_slice(&width.to_le_bytes());
        file.extend_from_slice(&(-height).to_le_bytes());
        file.extend_from_slice(&1u16.to_le_bytes());
        file.extend_from_slice(&32u16.to_le_bytes());
        file.extend_from_slice(&[0; 24]);
        file.extend_from_slice(&pixels);
        std::fs::write(path, file)?;
    }
    Ok(())
}

pub(super) fn run(h: HWND, directory: &Path) -> Result<()> {
    let mut log = vec![format!(
        "YYaccountant {} native UI smoke",
        env!("CARGO_PKG_VERSION")
    )];
    let outcome = steps(h, directory, &mut log);
    if let Err(e) = &outcome {
        log.push(format!("FAIL {e}"));
    }
    std::fs::write(directory.join("ui-smoke.txt"), log.join("\n") + "\n")?;
    outcome
}

fn steps(h: HWND, directory: &Path, log: &mut Vec<String>) -> Result<()> {
    let dpi = state(|s| s.dpi)?;
    resize(h, scale(dpi, 1180), scale(dpi, 780));
    let today = today();

    // A book is created through the real modal form.
    form::SMOKE_VALUES.with(|v| {
        *v.borrow_mut() = BTreeMap::from([
            ("帳簿名".into(), "UI検証 家計".into()),
            ("種類".into(), "家計".into()),
        ])
    });
    menu(h, ID_NEW_BOOK);
    let books = state(|s| s.books.clone())?;
    check(
        books.len() == 1 && books[0].name == "UI検証 家計",
        "新しい帳簿フォームで家計の帳簿を作成",
        log,
    )?;

    // Compound entry: unbalanced lines are refused, balanced ones are saved.
    menu(h, ID_PAGE + 2);
    check(
        state(|s| s.page)? == 2,
        "Ctrl+3相当で仕訳入力へ切り替え",
        log,
    )?;
    type_into(ID_E_DESC, "開始残高")?;
    for (name, debit, credit) in [("普通預金", "100,000", ""), ("現金", "２００００", "")]
    {
        choose(h, ID_E_ACCOUNT, name)?;
        type_into(ID_E_DEBIT, debit)?;
        type_into(ID_E_CREDIT, credit)?;
        click(h, ID_E_ADD)?;
    }
    check(rows(ID_E_LIST)? == 2, "明細2行を追加", log)?;
    click(h, ID_E_SAVE)?;
    let refused = messages();
    check(
        entries()?.is_empty() && refused.iter().any(|m| m.contains("貸借が一致しません")),
        "貸借不一致の仕訳は保存しない",
        log,
    )?;
    choose(h, ID_E_ACCOUNT, "開始残高")?;
    type_into(ID_E_CREDIT, "120000")?;
    send(h, WM_COMMAND, 1, 0); // Enter adds the line on this page.
    check(
        text(handle(ID_E_TOTALS)?).contains("差額 0")
            && text(handle(ID_E_TOTALS)?).ends_with("貸借一致"),
        "Enterで3行目を追加し貸借一致を表示",
        log,
    )?;
    click(h, ID_E_SAVE)?;
    let saved = entries()?;
    check(
        saved.len() == 1 && saved[0].total() == num::parse("120000")? && rows(ID_E_LIST)? == 0,
        "複合仕訳を保存して入力欄を新規に戻す",
        log,
    )?;

    // Quick input with full-width digits and 円.
    menu(h, ID_PAGE);
    choose(h, ID_Q_KIND, "支出")?;
    type_into(ID_Q_AMOUNT, "１，９８０円")?;
    choose(h, ID_Q_ACCOUNT, "食費")?;
    choose(h, ID_Q_VIA, "現金")?;
    type_into(ID_Q_DESC, "スーパーで食材")?;
    click(h, ID_Q_SAVE)?;
    let saved = entries()?;
    let quick = saved.iter().find(|e| e.description == "スーパーで食材");
    check(
        quick.is_some_and(|e| e.total() == num::parse("1980").unwrap())
            && text(handle(ID_Q_AMOUNT)?).is_empty(),
        "かんたん入力（全角・円付きの金額）で支出を登録",
        log,
    )?;
    type_into(ID_Q_AMOUNT, "12.5")?;
    send(h, WM_COMMAND, 1, 0);
    check(
        entries()?.len() == 2 && messages().iter().any(|m| m.contains("小数0桁")),
        "JPYで小数の金額を拒否",
        log,
    )?;
    type_into(ID_Q_AMOUNT, "")?;

    // Account finder and guide.
    type_into(ID_Q_SEARCH, "ガソリン")?;
    let list = handle(ID_Q_SUGGEST)?;
    check(
        send(list, LB_GETCOUNT, 0, 0).0 > 0,
        "「ガソリン」で科目の候補を表示",
        log,
    )?;
    send(list, LB_SETCURSEL, 0, 0);
    send(
        h,
        WM_COMMAND,
        (LBN_SELCHANGE as usize) << 16 | ID_Q_SUGGEST as usize,
        list.0 as isize,
    );
    let suggested = send(list, LB_GETITEMDATA, 0, 0).0 as i64;
    check(
        combo_value(handle(ID_Q_ACCOUNT)?) == Some(suggested)
            && text(handle(ID_Q_GUIDE)?).contains("の使い方"),
        "候補を選ぶと科目と使い分けの説明を表示",
        log,
    )?;
    choose(h, ID_Q_KIND, "収入")?;
    check(
        text(handle(ID_Q_ACCOUNT + LABEL)?) == "収入の科目"
            && text(handle(ID_Q_VIA + LABEL)?) == "入金先",
        "種類に合わせて科目の見出しを変更",
        log,
    )?;
    choose(h, ID_Q_KIND, "支出")?;

    // Journal: select, void (kept), show voided, restore.
    menu(h, ID_PAGE + 1);
    check(rows(ID_J_LIST)? == 2, "仕訳帳に2件を表示", log)?;
    let row = find_row(ID_J_LIST, 2, "スーパーで食材")?;
    select(ID_J_LIST, row)?;
    check(rows(ID_J_LINES)? == 2, "選んだ仕訳の明細を表示", log)?;
    click(h, ID_J_TOGGLE)?;
    let voided = entries()?;
    check(
        voided.len() == 2 && voided.iter().any(|e| e.void) && rows(ID_J_LIST)? == 1,
        "取消は削除せず残して一覧から外す",
        log,
    )?;
    let voided_check = handle(ID_J_VOIDED)?;
    send(voided_check, BM_SETCHECK, BST_CHECKED.0 as usize, 0);
    click(h, ID_J_VOIDED)?;
    let row = find_row(ID_J_LIST, 7, "取消")?;
    select(ID_J_LIST, row)?;
    check(
        rows(ID_J_LIST)? == 2 && text(handle(ID_J_TOGGLE)?) == "取消を解除",
        "取消も表示で取消済みの仕訳を確認",
        log,
    )?;
    click(h, ID_J_TOGGLE)?;
    check(!entries()?.iter().any(|e| e.void), "取消を解除", log)?;
    type_into(ID_J_QUERY, "1980")?;
    check(rows(ID_J_LIST)? == 1, "金額で仕訳を検索", log)?;
    type_into(ID_J_QUERY, "")?;

    // Correct an entry: the same ID is replaced.
    let row = find_row(ID_J_LIST, 2, "開始残高")?;
    select(ID_J_LIST, row)?;
    click(h, ID_J_EDIT)?;
    check(
        state(|s| s.page == 2 && s.editing.is_some() && s.draft.len() == 3)?,
        "修正で仕訳入力に読み込む",
        log,
    )?;
    select(ID_E_LIST, 1)?;
    check(
        text(handle(ID_E_DEBIT)?) == "20000" && text(handle(ID_E_ADD)?) == "行を更新",
        "明細行を選ぶと入力欄に戻る",
        log,
    )?;
    type_into(ID_E_DEBIT, "30000")?;
    click(h, ID_E_ADD)?;
    select(ID_E_LIST, 2)?;
    type_into(ID_E_CREDIT, "130000")?;
    click(h, ID_E_ADD)?;
    type_into(ID_E_DESC, "開始残高（修正）")?;
    click(h, ID_E_SAVE)?;
    let corrected = entries()?;
    check(
        corrected.len() == 2
            && corrected.iter().any(|e| {
                e.id == 1
                    && e.description == "開始残高（修正）"
                    && e.total() == num::parse("130000").unwrap()
            }),
        "修正は同じ仕訳番号で置き換える",
        log,
    )?;

    // Changes from another connection (MCP, Web UI) appear; a stale save is refused.
    let (path, book_id, revision) = state(|s| {
        let b = s.book.as_ref().unwrap();
        (s.path.clone(), b.id, b.revision)
    })?;
    let mut other = Store::open(&path, false)?;
    let cash = account("現金")?;
    let food = account("食費")?;
    let outside = |description: &str| Change::QuickEntry {
        quick: QuickEntry {
            kind: QuickKind::Expense,
            date: today,
            amount: num::parse("500").unwrap(),
            account: food,
            via: cash,
            description: description.into(),
            partner: None,
            business_percent: None,
            memo: String::new(),
        },
    };
    let applied = other.apply(book_id, revision, &outside("AIが登録"), "AI-test")?;
    send(h, WM_TIMER, TIMER, 0);
    menu(h, ID_PAGE + 1);
    check(
        entries()?.len() == 3 && find_row(ID_J_LIST, 2, "AIが登録").is_ok(),
        "ほかの接続の変更を自動で読み込む",
        log,
    )?;
    other.apply(
        book_id,
        applied.book.revision,
        &outside("AIが続けて登録"),
        "AI-test",
    )?;
    menu(h, ID_PAGE);
    type_into(ID_Q_AMOUNT, "300")?;
    choose(h, ID_Q_ACCOUNT, "食費")?;
    click(h, ID_Q_SAVE)?;
    check(
        messages()
            .iter()
            .any(|m| m.contains("他の利用者が先に更新しました"))
            && entries()?.len() == 4
            && text(handle(ID_Q_AMOUNT)?) == "300",
        "古い版の保存は競合として拒否し、最新を読み込む（入力は残す）",
        log,
    )?;
    click(h, ID_Q_SAVE)?;
    check(entries()?.len() == 5, "再保存で登録", log)?;

    // Reports.
    menu(h, ID_PAGE + 3);
    for (i, name) in sheets::REPORTS.iter().enumerate() {
        choose(h, ID_R_KIND, name)?;
        if i == 3 {
            choose(h, ID_R_ACCOUNT, "現金")?;
        }
        let summary = text(handle(ID_R_SUMMARY)?);
        let balanced =
            !matches!(i, 0 | 2) || summary.ends_with("一致") && !summary.ends_with("不一致");
        check(
            rows(ID_R_LIST)? > 1 && balanced,
            &format!("{name}: {summary}"),
            log,
        )?;
        if i == 3 {
            check(
                summary.ends_with("残高 26,720"),
                "現金の元帳残高 30,000−1,980−500−500−300",
                log,
            )?;
        }
    }

    // Settings, backup and CSV.
    form::SMOKE_VALUES.with(|v| {
        *v.borrow_mut() = BTreeMap::from([("帳簿名".into(), "UI検証 家計（改名）".into())])
    });
    menu(h, ID_SETTINGS);
    check(
        state(|s| {
            s.book
                .as_ref()
                .is_some_and(|b| b.name == "UI検証 家計（改名）")
        })?,
        "帳簿の設定フォームで名前を変更",
        log,
    )?;
    let backup = directory.join("backup.sqlite3");
    SMOKE_FILE.with(|f| *f.borrow_mut() = Some(backup.clone()));
    menu(h, ID_BACKUP);
    check(
        Store::open(&backup, true)?.list()?.len() == 1,
        "バックアップを新しいファイルに作成",
        log,
    )?;
    let csv = directory.join("journal.csv");
    SMOKE_FILE.with(|f| *f.borrow_mut() = Some(csv.clone()));
    menu(h, ID_CSV);
    let content = std::fs::read_to_string(&csv)?;
    check(
        content.starts_with("\u{feff}日付,") && content.contains("スーパーで食材"),
        "仕訳CSVを保存",
        log,
    )?;

    // Sole proprietor: the business share splits off to 事業主貸.
    form::SMOKE_VALUES.with(|v| {
        *v.borrow_mut() = BTreeMap::from([
            ("帳簿名".into(), "UI検証 事業".into()),
            ("種類".into(), "個人事業主".into()),
        ])
    });
    menu(h, ID_NEW_BOOK);
    menu(h, ID_PAGE);
    choose(h, ID_Q_KIND, "支出")?;
    let ratio = handle(ID_Q_RATIO)?;
    check(
        unsafe { IsWindowVisible(ratio).as_bool() },
        "個人事業主の支出で事業割合を表示",
        log,
    )?;
    type_into(ID_Q_AMOUNT, "100000")?;
    choose(h, ID_Q_ACCOUNT, "地代家賃")?;
    choose(h, ID_Q_VIA, "普通預金")?;
    type_into(ID_Q_RATIO, "30")?;
    click(h, ID_Q_SAVE)?;
    let draw = account("事業主貸")?;
    let split = entries()?;
    check(
        split.len() == 1
            && split[0]
                .lines
                .iter()
                .any(|l| l.account == draw && l.debit == num::parse("70000").unwrap()),
        "事業割合30%で残りを事業主貸に計上",
        log,
    )?;
    let household = state(|s| s.books[0].id)?;
    let books = handle(ID_BOOK)?;
    select_combo(books, household);
    send(
        h,
        WM_COMMAND,
        (CBN_SELCHANGE as usize) << 16 | ID_BOOK as usize,
        books.0 as isize,
    );
    check(
        state(|s| s.book.as_ref().is_some_and(|b| b.id == household))?
            && unsafe { !IsWindowVisible(ratio).as_bool() },
        "帳簿を切り替えると家計では事業割合を隠す",
        log,
    )?;

    // Layout at the minimum and a large size, on every page; screenshots at the large size.
    let mut shots = 0;
    for (width, height, shoot) in [(860, 600, false), (1400, 900, true)] {
        resize(h, scale(dpi, width), scale(dpi, height));
        for (page, title) in PAGES.iter().enumerate() {
            menu(h, ID_PAGE + page as u16);
            let detail = layout_is_clean(h)?;
            log.push(format!("ok  {title}（{width}x{height}）: {detail}"));
            if shoot {
                match capture(h, &directory.join(format!("page{}.bmp", page + 1))) {
                    Ok(()) => shots += 1,
                    Err(e) => log.push(format!("note {e}")),
                }
            }
        }
    }
    log.push(format!("ok  画面を{shots}枚保存"));

    // A DPI change (moving to a 150% monitor) rebuilds fonts and layout.
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(h, &mut r);
    }
    let bigger = RECT {
        left: r.left,
        top: r.top,
        right: r.left + scale(144, 1180),
        bottom: r.top + scale(144, 780),
    };
    send(
        h,
        WM_DPICHANGED,
        144 << 16 | 144,
        &bigger as *const _ as isize,
    );
    menu(h, ID_PAGE + 1);
    check(
        state(|s| s.dpi)? == 144,
        "DPI変更（150%）でフォントと配置を作り直す",
        log,
    )?;
    log.push(format!("ok  150%: {}", layout_is_clean(h)?));
    let back = RECT {
        left: r.left,
        top: r.top,
        right: r.left + scale(dpi, 1180),
        bottom: r.top + scale(dpi, 780),
    };
    send(
        h,
        WM_DPICHANGED,
        (dpi as usize) << 16 | dpi as usize,
        &back as *const _ as isize,
    );

    // Read-only: inputs that write are disabled and Enter does nothing.
    SMOKE_FILE.with(|f| *f.borrow_mut() = Some(path.clone()));
    menu(h, ID_OPEN_READ_ONLY);
    menu(h, ID_PAGE);
    let before = entries()?.len();
    type_into(ID_Q_AMOUNT, "100")?;
    send(h, WM_COMMAND, 1, 0);
    check(
        state(|s| s.read_only)?
            && unsafe { !IsWindowEnabled(handle(ID_Q_SAVE)?).as_bool() }
            && entries()?.len() == before
            && text(h).contains("読み取り専用"),
        "読み取り専用で開くと書き込みを無効にする",
        log,
    )?;
    SMOKE_FILE.with(|f| *f.borrow_mut() = Some(path.clone()));
    menu(h, ID_OPEN);
    check(!state(|s| s.read_only)?, "通常モードで開き直す", log)?;

    let left = messages();
    check(
        left.is_empty(),
        &format!("想定外のメッセージなし {left:?}"),
        log,
    )?;
    Ok(())
}
