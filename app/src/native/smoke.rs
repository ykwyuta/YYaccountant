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
/// Values the next form receives, by field label (choices by option text).
fn fill(values: &[(&str, &str)]) {
    form::SMOKE_VALUES.with(|v| {
        *v.borrow_mut() = values
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    });
}
fn book() -> Result<Book> {
    state(|s| s.book.clone())?.ok_or_else(|| "No book".into())
}
fn summary() -> Result<String> {
    Ok(text(handle(ID_G_SUMMARY)?))
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

    records(h, log)?;
    planning(h, directory, log)?;

    // Layout at the minimum and a large size, on every page; screenshots at the large size.
    let mut shots = 0;
    // 680x500 is below the minimum: what a 1024x768 screen at 150% leaves (CI runners).
    SMOKE_MIN_SIZE.with(|m| m.set(Some((560, 400))));
    for (width, height, shoot) in [(680, 500, false), (860, 600, false), (1400, 900, true)] {
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
    SMOKE_MIN_SIZE.with(|m| m.set(None));
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
    menu(h, ID_PAGE + RECORDS as u16);
    check(
        unsafe { !IsWindowEnabled(handle(ID_G_NEW)?).as_bool() },
        "読み取り専用では記録の追加も無効",
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

/// Record pages: accounts, partners, claims, budgets, plans, costs, statistics, history.
fn records(h: HWND, log: &mut Vec<String>) -> Result<()> {
    let page = |n: usize| ID_PAGE + (RECORDS + n) as u16;
    // Accounts and partners (household book).
    menu(h, page(0));
    check(
        rows(ID_G_LIST)? == book()?.accounts.len(),
        "勘定科目の一覧を表示",
        log,
    )?;
    select(ID_G_LIST, find_row(ID_G_LIST, 1, "食費")?)?;
    check(
        rows(ID_G_DETAIL)? > 0 && summary()?.contains("食費"),
        "科目を選ぶと使い分けの例を表示",
        log,
    )?;
    fill(&[
        ("コード", "699"),
        ("科目名", "UI検証費"),
        ("区分", "費用"),
        ("使い分けの説明", "検証用の科目"),
    ]);
    click(h, ID_G_NEW)?;
    select(ID_G_LIST, find_row(ID_G_LIST, 1, "UI検証費")?)?;
    fill(&[("科目名", "UI検証費（改）"), ("使用停止", "はい")]);
    click(h, ID_G_EDIT)?;
    check(
        book()?
            .accounts
            .iter()
            .any(|a| a.code == "699" && a.name == "UI検証費（改）" && a.archived),
        "科目を追加・編集（使用停止）",
        log,
    )?;
    choose(h, ID_G_VIEW, "取引先")?;
    fill(&[("名前", "UI商店"), ("メモ", "検証")]);
    click(h, ID_G_NEW)?;
    let added = book()?.partners.iter().any(|p| p.name == "UI商店");
    select(ID_G_LIST, find_row(ID_G_LIST, 0, "UI商店")?)?;
    click(h, ID_G_REMOVE)?;
    check(
        added && !book()?.partners.iter().any(|p| p.name == "UI商店"),
        "取引先を追加して、未使用なので削除",
        log,
    )?;

    // Claims: a loan to a friend, repaid in five parts.
    menu(h, page(1));
    fill(&[
        ("区分", "債権"),
        ("件名", "友人への貸付"),
        ("科目（売掛金・貸付金・借入金など）", "貸付金"),
        ("元本", "50,000"),
        ("返済方法", "元金均等"),
        ("回数（月払い）", "5"),
        ("発生の仕訳の相手科目", "普通預金"),
    ]);
    click(h, ID_G_NEW)?;
    select(ID_G_LIST, find_row(ID_G_LIST, 1, "友人への貸付")?)?;
    check(
        rows(ID_G_DETAIL)? == 5
            && entries()?.iter().any(|e| {
                e.source.ends_with(":opening") && e.total() == num::parse("50000").unwrap()
            }),
        "債権を登録し、返済予定5回と発生の仕訳を作成",
        log,
    )?;
    fill(&[]);
    click(h, ID_G_FIRST)?;
    let row = find_row(ID_G_LIST, 1, "友人への貸付")?;
    check(
        cell(handle(ID_G_LIST)?, row, 6) == "40,000",
        "回収フォーム（初回分の既定値）で残高40,000",
        log,
    )?;

    // Budget: monthly food budget, then taken out again.
    menu(h, page(2));
    check(
        summary()?.contains("予算がありません"),
        "予算がないときの案内",
        log,
    )?;
    fill(&[]);
    click(h, ID_G_NEW)?;
    fill(&[
        ("科目", "食費"),
        ("毎月同額（入れると各月より優先）", "30000"),
    ]);
    click(h, ID_G_FIRST)?;
    let row = find_row(ID_G_LIST, 0, "食費")?;
    check(
        cell(handle(ID_G_LIST)?, row, 1) == "360,000",
        "予算を作成し、食費を毎月30,000（年360,000）に設定",
        log,
    )?;
    select(ID_G_LIST, row)?;
    click(h, ID_G_SECOND)?;
    check(book()?.budgets[0].lines.is_empty(), "科目の予算を外す", log)?;

    // Plans: monthly rent, one occurrence posted (and not twice).
    menu(h, page(3));
    fill(&[
        ("名前", "家賃"),
        ("借方科目", "住居費"),
        ("貸方科目", "普通預金"),
        ("金額", "80000"),
    ]);
    click(h, ID_G_NEW)?;
    check(
        rows(ID_G_DETAIL)? == 4,
        "毎月の計画の予定を3か月先まで表示",
        log,
    )?;
    select(ID_G_DETAIL, 0)?;
    click(h, ID_G_FIRST)?;
    select(ID_G_DETAIL, 0)?;
    click(h, ID_G_FIRST)?;
    let posted = entries()?
        .iter()
        .filter(|e| !e.void && e.source.starts_with("plan:"))
        .count();
    check(
        posted == 1
            && cell(handle(ID_G_DETAIL)?, 0, 5) == "計上済"
            && messages().iter().any(|m| m.contains("計上")),
        "予定を計上し、二重計上は拒否",
        log,
    )?;

    // Statistics, forecast and history.
    menu(h, page(5));
    check(rows(ID_G_LIST)? == 12, "月別の収支を12か月表示", log)?;
    choose(h, ID_G_VIEW, "科目別")?;
    check(rows(ID_G_LIST)? > 0, "科目別の統計を表示", log)?;
    choose(h, ID_G_VIEW, "予測")?;
    check(
        rows(ID_G_LIST)? == 6 && rows(ID_G_DETAIL)? > 0,
        "6か月の予測と科目ごとの方法を表示",
        log,
    )?;
    menu(h, page(8));
    let id = book()?.id;
    let audit = state(|s| s.store.audit(id, 0).map(|a| a.len()))??;
    check(
        rows(ID_G_LIST)? == audit && cell(handle(ID_G_LIST)?, 0, 2) == "UI-test",
        "変更履歴を新しい順に表示",
        log,
    )?;

    // Costs in the sole proprietor book.
    let business = state(|s| {
        s.books
            .iter()
            .find(|b| b.name == "UI検証 事業")
            .map(|b| b.id)
    })?
    .ok_or("No business book")?;
    let books = handle(ID_BOOK)?;
    select_combo(books, business);
    send(
        h,
        WM_COMMAND,
        (CBN_SELCHANGE as usize) << 16 | ID_BOOK as usize,
        books.0 as isize,
    );
    menu(h, page(4));
    fill(&[
        ("名前", "小麦粉"),
        ("単位（kg・個など）", "kg"),
        ("標準単価", "200"),
    ]);
    click(h, ID_G_NEW)?;
    choose(h, ID_G_VIEW, "仕入")?;
    fill(&[
        ("原材料", "小麦粉"),
        ("数量（調整は減少を負で）", "10"),
        ("単価（調整は空欄可）", "180"),
    ]);
    click(h, ID_G_NEW)?;
    choose(h, ID_G_VIEW, "設備")?;
    fill(&[
        ("名前", "オーブン"),
        ("取得価額", "600000"),
        ("耐用年数", "5"),
        ("年間稼働時間", "1000"),
        ("時間当たり運転費", "30"),
    ]);
    click(h, ID_G_NEW)?;
    select(ID_G_LIST, 0)?;
    check(
        cell(handle(ID_G_LIST)?, 0, 6) == "150" && rows(ID_G_DETAIL)? >= 5,
        "設備の時間当たり費用150と償却予定",
        log,
    )?;
    select(ID_G_DETAIL, 0)?;
    click(h, ID_G_FIRST)?;
    check(
        entries()?
            .iter()
            .any(|e| e.source.starts_with("depreciation:"))
            && cell(handle(ID_G_DETAIL)?, 0, 4) == "計上済",
        "選んだ年度の減価償却を計上",
        log,
    )?;
    choose(h, ID_G_VIEW, "製品")?;
    fill(&[
        ("名前", "食パン"),
        ("単位", "斤"),
        ("1単位の作業時間", "0.5"),
        ("時間当たり賃率", "1000"),
        ("間接費配賦率（%）", "10"),
    ]);
    click(h, ID_G_NEW)?;
    select(ID_G_LIST, 0)?;
    fill(&[("原材料", "小麦粉"), ("数量（0で外す）", "0.5")]);
    click(h, ID_G_FIRST)?;
    fill(&[("設備", "オーブン"), ("時間（0で外す）", "0.1")]);
    click(h, ID_G_SECOND)?;
    // 材料 0.5×200＝100、設備 0.1×150＝15、労務 500、間接費 10%＝61.5→62（四捨五入）
    check(
        cell(handle(ID_G_LIST)?, 0, 6) == "677" && rows(ID_G_DETAIL)? == 5,
        "製品の構成（材料・設備時間）から標準原価677",
        log,
    )?;
    choose(h, ID_G_VIEW, "製造実績")?;
    fill(&[
        ("製品", "食パン"),
        ("数量", "20"),
        ("作業時間（合計）", "10"),
    ]);
    click(h, ID_G_NEW)?;
    check(
        rows(ID_G_LIST)? == 1,
        "製造実績を登録して原価差異を表示",
        log,
    )?;
    choose(h, ID_G_VIEW, "原材料")?;
    check(
        cell(handle(ID_G_LIST)?, 0, 2) == "0",
        "製造で小麦粉10kgを払い出し在庫0",
        log,
    )?;
    let household = state(|s| s.books[0].id)?;
    select_combo(books, household);
    send(
        h,
        WM_COMMAND,
        (CBN_SELCHANGE as usize) << 16 | ID_BOOK as usize,
        books.0 as isize,
    );
    let left = messages();
    check(
        left.is_empty(),
        &format!("記録の画面で想定外のメッセージなし {left:?}"),
        log,
    )?;
    Ok(())
}

/// Tax and life plan pages, the tutorial and the six coefficients (household book).
fn planning(h: HWND, directory: &Path, log: &mut Vec<String>) -> Result<()> {
    let page = |n: usize| ID_PAGE + (RECORDS + n) as u16;
    menu(h, page(6));
    check(
        summary()?.contains("年分を追加"),
        "税金：年分がないときの案内",
        log,
    )?;
    fill(&[("年分", "2026")]);
    click(h, ID_G_NEW)?;
    check(
        book()?.tax_returns.len() == 1 && summary()?.contains("2026年分"),
        "年分を追加して選ぶ",
        log,
    )?;
    // Two-step forms: the part, then that part's fields.
    for values in [
        &[
            ("編集する入力", "収入（給与）"),
            ("給与の支払金額（源泉徴収票）", "6,000,000"),
            ("源泉徴収税額", "80000"),
            ("社会保険料（家族分を含む）", "870000"),
        ][..],
        &[
            ("編集する入力", "副業"),
            ("副業の種類", "雑所得"),
            ("副業の収入", "600000"),
            ("副業の経費", "200000"),
        ],
        &[
            ("編集する入力", "配偶者"),
            ("配偶者", "いる"),
            ("配偶者の合計所得金額", "260000"),
            ("配偶者の生まれ年", "1988"),
        ],
        &[
            ("編集する入力", "保険料"),
            ("一般生命保険料（新）", "100000"),
            ("介護医療保険料", "40000"),
            ("地震保険料", "30000"),
        ],
        &[
            ("編集する入力", "医療費"),
            ("医療費（家族分の合計）", "250000"),
            ("ふるさと納税の寄附額", "50000"),
        ],
        &[
            ("編集する入力", "住宅ローン"),
            ("住宅ローンの年末残高", "25000000"),
            ("借入限度額", "40000000"),
        ],
    ] {
        fill(values);
        click(h, ID_G_EDIT)?;
    }
    for (name, relation, born, income) in [
        ("長男", "子", "2010", ""),
        ("父", "父母", "1954", "400000"),
        ("母", "父母", "1956", ""),
    ] {
        fill(&[
            ("名前（例: 長男、父）", name),
            ("続柄", relation),
            ("生まれ年", born),
            ("合計所得金額", income),
        ]);
        click(h, ID_G_FIRST)?;
    }
    let t = book()?.tax_returns[0].clone();
    check(
        t.salary == num::parse("6000000").unwrap()
            && t.side_kind == yy_accountant::tax::SideKind::Miscellaneous
            && t.has_spouse
            && t.dependents.len() == 3
            && t.dependents[1].cohabiting,
        "収入・副業・配偶者・保険・医療費・寄附・住宅ローン・親族3人を入力",
        log,
    )?;
    // Same household as the core test: refund 80,000, resident tax 87,500.
    let donation = find_row(ID_G_LIST, 1, "寄附金税額控除")?;
    check(
        summary()?.contains("還付 80,000")
            && summary()?.contains("住民税 87,500")
            && cell(handle(ID_G_LIST)?, donation, 3) == "-34,360"
            && rows(ID_G_DETAIL)? == 3,
        "所得税の還付80,000・住民税87,500・ふるさと納税の税額控除34,360",
        log,
    )?;
    check(
        find_row(ID_G_LIST, 0, "注意").is_ok(),
        "確定申告の要否などの注意を表示",
        log,
    )?;
    select(ID_G_DETAIL, find_row(ID_G_DETAIL, 0, "母")?)?;
    fill(&[("外す", "はい")]);
    click(h, ID_G_SECOND)?;
    check(
        book()?.tax_returns[0].dependents.len() == 2,
        "親族を外す",
        log,
    )?;

    // Life plan from the book and the tax year's take-home pay, then a proposal.
    menu(h, page(7));
    check(
        summary()?.contains("新しいプラン"),
        "ライフプラン：プランがないときの案内",
        log,
    )?;
    fill(&[
        ("名前", "現状"),
        ("作り方", "帳簿から"),
        ("帳簿から作るときの収入", "手取り"),
        ("最初の年", "2027"),
        ("年数", "20"),
    ]);
    click(h, ID_G_NEW)?;
    let plan = book()?.life_plans[0].clone();
    check(
        rows(ID_G_LIST)? == 20
            && plan.flows.iter().any(|f| f.name.contains("税額計算"))
            && plan.flows.iter().all(|f| f.start_year == 2027),
        "帳簿と税額計算の手取りから20年のキャッシュフロー表",
        log,
    )?;
    for values in [
        &[
            ("追加する項目", "家族"),
            ("名前", "長男"),
            ("続柄（本人・配偶者・長男など）", "長男"),
            ("生まれ年", "2010"),
        ][..],
        &[
            ("追加する項目", "教育費"),
            ("高校", "公立"),
            ("大学", "私立"),
        ],
        &[
            ("追加する項目", "ローン"),
            ("借入額（残高）", "30000000"),
            ("金利（年%）", "1"),
            ("返済期間（年）", "35"),
        ],
        &[
            ("追加する項目", "イベント"),
            ("名前", "車の買い替え"),
            ("金額（今の価格）", "3000000"),
            ("年", "2030"),
        ],
    ] {
        fill(values);
        click(h, ID_G_FIRST)?;
    }
    let plan = book()?.life_plans[0].clone();
    check(
        plan.members.len() == 1
            && plan.flows.iter().any(|f| f.name == "長男の大学（私立）")
            && plan
                .flows
                .iter()
                .any(|f| f.name == "住宅ローン返済" && f.amount == num::parse("1016232").unwrap())
            && plan.events.len() == 2
            && find_row(ID_G_LIST, 0, "2030").is_ok(),
        "家族・教育費（進路から）・ローン返済額（年1,016,232）・イベントを追加",
        log,
    )?;
    fill(&[("名前", "提案"), ("作り方", "複製")]);
    click(h, ID_G_NEW)?;
    select(ID_G_DETAIL, find_row(ID_G_DETAIL, 1, "車の買い替え")?)?;
    fill(&[("外す", "はい")]);
    click(h, ID_G_SECOND)?;
    check(
        book()?.life_plans.len() == 2 && summary()?.contains("「現状」との差 3,000,000"),
        "複製した提案から車の買い替えを外し、現状との差3,000,000",
        log,
    )?;
    let csv = directory.join("plan.csv");
    SMOKE_FILE.with(|f| *f.borrow_mut() = Some(csv.clone()));
    menu(h, ID_PLAN_CSV);
    let text = std::fs::read_to_string(&csv).unwrap_or_default();
    check(
        text.starts_with("\u{feff}年,長男（歳）") && text.lines().count() == 21,
        "キャッシュフロー表をCSVで保存",
        log,
    )?;
    fill(&[("利率（年%）", "2"), ("年数", "10")]);
    menu(h, ID_COEFFICIENTS);
    menu(h, ID_TUTORIAL);
    let shown = messages();
    check(
        shown.iter().any(|m| m.contains("終価係数 1.218994"))
            && shown.iter().any(|m| {
                m.starts_with("チュートリアル")
                    && m.rsplit(' ')
                        .next()
                        .and_then(|n| n.parse::<usize>().ok())
                        .is_some_and(|n| n > 3000)
            }),
        "6つの係数とチュートリアルを表示",
        log,
    )?;
    check(
        shown.len() == 2,
        &format!("想定外のメッセージなし {shown:?}"),
        log,
    )?;
    Ok(())
}
