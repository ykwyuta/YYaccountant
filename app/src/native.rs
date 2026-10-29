//! Native Windows UI (`yyaccountantw`) built from standard Win32 controls. Parsing,
//! decimal arithmetic, validation and persistence stay in the core; this module only
//! moves text between controls and [`Change`]s.
use std::{cell::RefCell, collections::BTreeMap, path::PathBuf};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::*,
        UI::{
            Controls::Dialogs::*, Controls::*, HiDpi::*, Input::KeyboardAndMouse::*,
            WindowsAndMessaging::*,
        },
    },
    core::{HSTRING, PCWSTR, PWSTR, w},
};
use yy_accountant::{
    Book, BookInfo, BookKind, Change, Entry, Error as CoreError, Line, NewBook, QuickEntry,
    QuickKind, Store, num::Rounding, reports,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
mod form;
mod sheets;
mod smoke;
use form::Field;
use sheets::Table;

thread_local! {static APP:RefCell<Option<State>>=const{RefCell::new(None)};}
thread_local! {static SMOKE:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
thread_local! {static SMOKE_MESSAGES:RefCell<Vec<String>>=const{RefCell::new(Vec::new())};}
thread_local! {static SMOKE_FILE:RefCell<Option<PathBuf>>=const{RefCell::new(None)};}

const PAGES: [&str; 4] = ["かんたん入力", "仕訳帳", "仕訳入力", "帳票"];
const KINDS: [QuickKind; 3] = [QuickKind::Expense, QuickKind::Income, QuickKind::Transfer];
/// A static label for a field has the field's ID plus this offset.
const LABEL: u16 = 1000;
const TIMER: usize = 1;
/// SS_LEFT | SS_NOPREFIX | SS_CENTERIMAGE (these constants live in a feature we do not need).
const STATIC_STYLE: u32 = 0x0080 | 0x0200;

const ID_OPEN: u16 = 10;
const ID_OPEN_READ_ONLY: u16 = 11;
const ID_NEW_DATABASE: u16 = 12;
const ID_DEMO: u16 = 13;
const ID_BACKUP: u16 = 14;
const ID_CSV: u16 = 15;
const ID_EXIT: u16 = 16;
const ID_NEW_BOOK: u16 = 20;
const ID_SETTINGS: u16 = 21;
const ID_REFRESH: u16 = 22;
const ID_ABOUT: u16 = 30;
const ID_NEXT_PAGE: u16 = 31;
const ID_PREV_PAGE: u16 = 32;
const ID_PAGE: u16 = 40;

const ID_BOOK: u16 = 100;
const ID_TABS: u16 = 101;
const ID_STATUS: u16 = 102;

const ID_Q_KIND: u16 = 200;
const ID_Q_DATE: u16 = 201;
const ID_Q_AMOUNT: u16 = 202;
const ID_Q_ACCOUNT: u16 = 203;
const ID_Q_VIA: u16 = 204;
const ID_Q_PARTNER: u16 = 205;
const ID_Q_DESC: u16 = 206;
const ID_Q_RATIO: u16 = 207;
const ID_Q_MEMO: u16 = 208;
const ID_Q_SAVE: u16 = 209;
const ID_Q_SEARCH: u16 = 210;
const ID_Q_SUGGEST: u16 = 211;
const ID_Q_GUIDE: u16 = 212;

const ID_J_FROM: u16 = 300;
const ID_J_TO: u16 = 301;
const ID_J_QUERY: u16 = 302;
const ID_J_VOIDED: u16 = 303;
const ID_J_LIST: u16 = 304;
const ID_J_LINES: u16 = 305;
const ID_J_TOGGLE: u16 = 306;
const ID_J_EDIT: u16 = 307;
const ID_J_COPY: u16 = 308;
const ID_J_COUNT: u16 = 309;

const ID_E_DATE: u16 = 400;
const ID_E_DESC: u16 = 401;
const ID_E_PARTNER: u16 = 402;
const ID_E_ACCOUNT: u16 = 403;
const ID_E_DEBIT: u16 = 404;
const ID_E_CREDIT: u16 = 405;
const ID_E_MEMO: u16 = 406;
const ID_E_ADD: u16 = 407;
const ID_E_REMOVE: u16 = 408;
const ID_E_LIST: u16 = 409;
const ID_E_TOTALS: u16 = 410;
const ID_E_CLEAR: u16 = 411;
const ID_E_SAVE: u16 = 412;
const ID_E_MODE: u16 = 413;

const ID_R_KIND: u16 = 500;
const ID_R_FROM: u16 = 501;
const ID_R_TO: u16 = 502;
const ID_R_ACCOUNT: u16 = 503;
const ID_R_LIST: u16 = 504;
const ID_R_SUMMARY: u16 = 505;

/// Menu commands that ask for input (a file or a form) before using the state.
const DIALOGS: [u16; 10] = [
    ID_OPEN,
    ID_OPEN_READ_ONLY,
    ID_NEW_DATABASE,
    ID_DEMO,
    ID_BACKUP,
    ID_CSV,
    ID_EXIT,
    ID_NEW_BOOK,
    ID_SETTINGS,
    ID_ABOUT,
];
/// Controls that change the book; disabled when the file is opened read-only.
const WRITERS: [u16; 7] = [
    ID_Q_SAVE,
    ID_J_TOGGLE,
    ID_J_EDIT,
    ID_E_ADD,
    ID_E_REMOVE,
    ID_E_SAVE,
    ID_E_CLEAR,
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Label,
    Edit,
    Combo,
    Date,
    Button,
    List,
    Table,
}
struct Control {
    hwnd: HWND,
    page: Option<usize>,
    kind: Kind,
}
struct State {
    hwnd: HWND,
    tabs: HWND,
    status: HWND,
    font: HFONT,
    dpi: i32,
    store: Store,
    path: PathBuf,
    read_only: bool,
    actor: String,
    books: Vec<BookInfo>,
    book: Option<Book>,
    page: usize,
    controls: BTreeMap<u16, Control>,
    headings: BTreeMap<u16, Vec<&'static str>>,
    /// Lines of the compound entry being written.
    draft: Vec<Line>,
    /// Draft line loaded for correction.
    line_edit: Option<usize>,
    /// Original entry when correcting an existing one.
    editing: Option<Entry>,
    version: i64,
    note: String,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn send(h: HWND, m: u32, w: usize, l: isize) -> LRESULT {
    unsafe { SendMessageW(h, m, Some(WPARAM(w)), Some(LPARAM(l))) }
}
fn text(h: HWND) -> String {
    unsafe {
        let mut b = vec![0; GetWindowTextLengthW(h).max(0) as usize + 1];
        let n = GetWindowTextW(h, &mut b);
        String::from_utf16_lossy(&b[..n.max(0) as usize])
    }
}
fn set_text(h: HWND, value: &str) {
    if text(h) != value {
        unsafe {
            let _ = SetWindowTextW(h, &HSTRING::from(value));
        }
    }
}
fn scale(dpi: i32, value: i32) -> i32 {
    value * dpi / 96
}
fn today() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}
/// Shows a message, or records it during the smoke test so no dialog blocks the run.
fn message(h: HWND, value: &str, error: bool) {
    if SMOKE.with(|s| s.get()) {
        SMOKE_MESSAGES.with(|m| m.borrow_mut().push(value.into()));
        return;
    }
    unsafe {
        MessageBoxW(
            Some(h),
            &HSTRING::from(value),
            w!("YYaccountant"),
            MB_OK
                | if error {
                    MB_ICONWARNING
                } else {
                    MB_ICONINFORMATION
                },
        );
    }
}
fn fill_combo(h: HWND, items: &[(i64, String)], keep: Option<i64>) {
    send(h, CB_RESETCONTENT, 0, 0);
    let mut chosen = 0;
    for (i, (id, label)) in items.iter().enumerate() {
        let value = wide(label);
        let index = send(h, CB_ADDSTRING, 0, value.as_ptr() as isize).0;
        send(h, CB_SETITEMDATA, index as usize, *id as isize);
        if Some(*id) == keep {
            chosen = i;
        }
    }
    if !items.is_empty() {
        send(h, CB_SETCURSEL, chosen, 0);
    }
}
fn combo_index(h: HWND) -> Option<usize> {
    let i = send(h, CB_GETCURSEL, 0, 0).0;
    (i >= 0).then_some(i as usize)
}
fn combo_value(h: HWND) -> Option<i64> {
    combo_index(h).map(|i| send(h, CB_GETITEMDATA, i, 0).0 as i64)
}
fn select_combo(h: HWND, id: i64) -> bool {
    let count = send(h, CB_GETCOUNT, 0, 0).0.max(0) as usize;
    for i in 0..count {
        if send(h, CB_GETITEMDATA, i, 0).0 as i64 == id {
            send(h, CB_SETCURSEL, i, 0);
            return true;
        }
    }
    false
}
fn get_date(h: HWND) -> Option<chrono::NaiveDate> {
    let mut st = SYSTEMTIME::default();
    (send(h, DTM_GETSYSTEMTIME, 0, &mut st as *mut _ as isize).0 == GDT_VALID.0 as isize)
        .then(|| chrono::NaiveDate::from_ymd_opt(st.wYear as i32, st.wMonth as u32, st.wDay as u32))
        .flatten()
}
fn set_date(h: HWND, day: Option<chrono::NaiveDate>) {
    use chrono::Datelike;
    match day {
        Some(d) => {
            let st = SYSTEMTIME {
                wYear: d.year() as u16,
                wMonth: d.month() as u16,
                wDay: d.day() as u16,
                wDayOfWeek: d.weekday().num_days_from_sunday() as u16,
                ..Default::default()
            };
            send(
                h,
                DTM_SETSYSTEMTIME,
                GDT_VALID.0 as usize,
                &st as *const _ as isize,
            );
        }
        None => {
            send(h, DTM_SETSYSTEMTIME, GDT_NONE.0 as usize, 0);
        }
    }
}
fn selected_row(h: HWND) -> Option<(usize, i64)> {
    let index = send(h, LVM_GETNEXTITEM, usize::MAX, LVNI_SELECTED as isize).0;
    if index < 0 {
        return None;
    }
    let mut item = LVITEMW {
        mask: LVIF_PARAM,
        iItem: index as i32,
        ..Default::default()
    };
    send(h, LVM_GETITEMW, 0, &mut item as *mut _ as isize);
    Some((index as usize, item.lParam.0 as i64))
}
fn select_row(h: HWND, index: Option<usize>) {
    let state = LVITEMW {
        stateMask: LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED.0 | LVIS_FOCUSED.0),
        state: if index.is_some() {
            LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED.0 | LVIS_FOCUSED.0)
        } else {
            LIST_VIEW_ITEM_STATE_FLAGS(0)
        },
        ..Default::default()
    };
    match index {
        Some(i) => {
            send(h, LVM_SETITEMSTATE, i, &state as *const _ as isize);
            send(h, LVM_ENSUREVISIBLE, i, 0);
        }
        None => {
            send(h, LVM_SETITEMSTATE, usize::MAX, &state as *const _ as isize);
        }
    }
}
fn row_count(h: HWND) -> usize {
    send(h, LVM_GETITEMCOUNT, 0, 0).0.max(0) as usize
}
fn cell(h: HWND, row: usize, column: usize) -> String {
    let mut buffer = vec![0u16; 512];
    let mut item = LVITEMW {
        iSubItem: column as i32,
        pszText: PWSTR(buffer.as_mut_ptr()),
        cchTextMax: buffer.len() as i32,
        ..Default::default()
    };
    let n = send(h, LVM_GETITEMTEXTW, row, &mut item as *mut _ as isize).0;
    String::from_utf16_lossy(&buffer[..n.max(0) as usize])
}
fn window_font(dpi: i32) -> HFONT {
    unsafe {
        let mut metrics = NONCLIENTMETRICSW {
            cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32,
            ..Default::default()
        };
        let _ = SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS.0,
            metrics.cbSize,
            Some(&mut metrics as *mut _ as _),
            0,
            dpi as u32,
        );
        CreateFontIndirectW(&metrics.lfMessageFont)
    }
}
/// File chooser; the smoke test supplies the path instead of a dialog.
fn choose_file(
    owner: HWND,
    save: bool,
    title: &str,
    filter: &str,
    suggested: &str,
) -> Option<PathBuf> {
    if SMOKE.with(|s| s.get()) {
        return SMOKE_FILE.with(|f| f.borrow_mut().take());
    }
    let mut buffer = vec![0u16; 4096];
    for (i, c) in suggested.encode_utf16().take(4000).enumerate() {
        buffer[i] = c;
    }
    let filter: Vec<u16> = filter
        .encode_utf16()
        .map(|c| if c == u16::from(b'|') { 0 } else { c })
        .chain([0, 0])
        .collect();
    let title = wide(title);
    let extension = wide(
        if filter.starts_with(&"CSV".encode_utf16().collect::<Vec<_>>()) {
            "csv"
        } else {
            "sqlite3"
        },
    );
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        lpstrTitle: PCWSTR(title.as_ptr()),
        lpstrDefExt: PCWSTR(extension.as_ptr()),
        Flags: OFN_EXPLORER
            | OFN_PATHMUSTEXIST
            | OFN_NOCHANGEDIR
            | if save {
                OFN_OVERWRITEPROMPT
            } else {
                OFN_FILEMUSTEXIST
            },
        ..Default::default()
    };
    let chosen = unsafe {
        if save {
            GetSaveFileNameW(&mut dialog)
        } else {
            GetOpenFileNameW(&mut dialog)
        }
    };
    if !chosen.as_bool() {
        return None;
    }
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(PathBuf::from(String::from_utf16_lossy(&buffer[..end])))
}
const DATABASE_FILTER: &str =
    "YYaccountant データベース (*.sqlite3)|*.sqlite3;*.db|すべてのファイル|*.*";

impl State {
    fn px(&self, value: i32) -> i32 {
        scale(self.dpi, value)
    }
    fn c(&self, id: u16) -> HWND {
        self.controls[&id].hwnd
    }
    fn book(&self) -> Result<&Book> {
        Ok(self
            .book
            .as_ref()
            .ok_or("帳簿がありません。「帳簿」メニューの「新しい帳簿」で作成してください")?)
    }
    fn quick_kind(&self) -> QuickKind {
        KINDS[combo_index(self.c(ID_Q_KIND)).unwrap_or(0).min(2)]
    }
    /// 事業割合 only applies to expenses of sole proprietors.
    fn ratio_applies(&self) -> bool {
        self.book
            .as_ref()
            .is_some_and(|b| b.kind == BookKind::SoleProprietor)
            && self.quick_kind() == QuickKind::Expense
    }
    fn wanted(&self, id: u16, page: Option<usize>) -> bool {
        let ratio = id == ID_Q_RATIO || id == ID_Q_RATIO + LABEL;
        page.is_none_or(|p| p == self.page) && (!ratio || self.ratio_applies())
    }
    fn update_visibility(&self) {
        for (id, c) in &self.controls {
            let show = self.wanted(*id, c.page);
            unsafe {
                if IsWindowVisible(c.hwnd).as_bool() != show {
                    let _ = ShowWindow(c.hwnd, if show { SW_SHOWNA } else { SW_HIDE });
                }
            }
        }
        let ledger = combo_index(self.c(ID_R_KIND)) == Some(3);
        unsafe {
            let _ = EnableWindow(self.c(ID_R_ACCOUNT), ledger);
        }
    }
    fn set_page(&mut self, page: usize) {
        self.page = page.min(PAGES.len() - 1);
        send(self.tabs, TCM_SETCURSEL, self.page, 0);
        self.update_visibility();
        self.layout();
        let first = [ID_Q_AMOUNT, ID_J_QUERY, ID_E_ACCOUNT, ID_R_KIND][self.page];
        unsafe {
            let _ = SetFocus(Some(self.c(first)));
        }
    }
    fn fill_table(&mut self, id: u16, table: &Table, keep: Option<i64>) {
        let h = self.c(id);
        send(h, WM_SETREDRAW, 0, 0);
        let headings: Vec<&'static str> = table.columns.iter().map(|c| c.0).collect();
        if self.headings.get(&id) != Some(&headings) {
            while send(h, LVM_DELETECOLUMN, 0, 0).0 != 0 {}
            for (i, (title, width, align)) in table.columns.iter().enumerate() {
                let mut value = wide(title);
                let column = LVCOLUMNW {
                    mask: LVCF_TEXT | LVCF_WIDTH | LVCF_FMT,
                    fmt: if *align == sheets::Align::Right {
                        LVCFMT_RIGHT
                    } else {
                        LVCFMT_LEFT
                    },
                    cx: self.px(*width),
                    pszText: PWSTR(value.as_mut_ptr()),
                    ..Default::default()
                };
                send(h, LVM_INSERTCOLUMNW, i, &column as *const _ as isize);
            }
            self.headings.insert(id, headings);
        }
        send(h, LVM_DELETEALLITEMS, 0, 0);
        let mut chosen = None;
        for (row, (record, cells)) in table.rows.iter().enumerate() {
            for (column, value) in cells.iter().enumerate() {
                let mut value = wide(value);
                let mut item = LVITEMW {
                    mask: if column == 0 {
                        LVIF_TEXT | LVIF_PARAM
                    } else {
                        LVIF_TEXT
                    },
                    iItem: row as i32,
                    iSubItem: column as i32,
                    pszText: PWSTR(value.as_mut_ptr()),
                    lParam: LPARAM(*record as isize),
                    ..Default::default()
                };
                if column == 0 {
                    send(h, LVM_INSERTITEMW, 0, &mut item as *mut _ as isize);
                } else {
                    send(h, LVM_SETITEMTEXTW, row, &mut item as *mut _ as isize);
                }
            }
            if keep == Some(*record) && chosen.is_none() {
                chosen = Some(row);
            }
        }
        if chosen.is_some() {
            select_row(h, chosen);
        }
        send(h, WM_SETREDRAW, 1, 0);
        unsafe {
            let _ = InvalidateRect(Some(h), None, true);
        }
    }

    // ----- loading -----
    fn reload_books(&mut self) -> Result<()> {
        self.books = self.store.list()?;
        let keep = self
            .book
            .as_ref()
            .map(|b| b.id)
            .filter(|id| self.books.iter().any(|b| b.id == *id))
            .or_else(|| self.books.first().map(|b| b.id));
        let items: Vec<(i64, String)> = self
            .books
            .iter()
            .map(|b| (b.id, format!("{}（{}）", b.name, b.kind.label())))
            .collect();
        fill_combo(self.c(ID_BOOK), &items, keep);
        match keep {
            Some(id) => self.load_book(id),
            None => {
                self.book = None;
                self.refill();
                Ok(())
            }
        }
    }
    fn load_book(&mut self, id: i64) -> Result<()> {
        let changed = self.book.as_ref().is_none_or(|b| b.id != id);
        self.book = Some(self.store.get(id)?);
        if changed {
            self.note.clear();
            self.draft.clear();
            self.line_edit = None;
            self.editing = None;
            let b = self.book.as_ref().unwrap();
            let start = sheets::fiscal_start(b, today());
            set_date(self.c(ID_R_FROM), Some(start));
            set_date(self.c(ID_R_TO), Some(today()));
        }
        select_combo(self.c(ID_BOOK), id);
        self.refill();
        Ok(())
    }
    /// Refreshes every view from the loaded book, keeping selections and typed text.
    fn refill(&mut self) {
        let Some(book) = self.book.clone() else {
            for id in [
                ID_Q_ACCOUNT,
                ID_Q_VIA,
                ID_Q_PARTNER,
                ID_E_ACCOUNT,
                ID_E_PARTNER,
                ID_R_ACCOUNT,
            ] {
                fill_combo(self.c(id), &[], None);
            }
            for id in [ID_J_LIST, ID_J_LINES, ID_E_LIST, ID_R_LIST] {
                self.fill_table(id, &Table::default(), None);
            }
            set_text(self.c(ID_Q_GUIDE), "");
            self.update_status();
            return;
        };
        self.fill_quick_choices();
        let partners = sheets::partners(&book);
        for id in [ID_Q_PARTNER, ID_E_PARTNER] {
            let keep = combo_value(self.c(id));
            fill_combo(self.c(id), &partners, keep);
        }
        let accounts = sheets::all_accounts(&book);
        for id in [ID_E_ACCOUNT, ID_R_ACCOUNT] {
            let keep = combo_value(self.c(id));
            fill_combo(self.c(id), &accounts, keep);
        }
        self.refresh_journal();
        self.refresh_draft();
        self.refresh_report();
        self.refresh_guide();
        self.update_visibility();
        self.layout();
        self.update_status();
    }
    fn fill_quick_choices(&mut self) {
        let Some(book) = self.book.as_ref() else {
            return;
        };
        let kind = self.quick_kind();
        let (account, via) = sheets::kind_labels(kind);
        set_text(self.c(ID_Q_ACCOUNT + LABEL), account);
        set_text(self.c(ID_Q_VIA + LABEL), via);
        for (id, is_via) in [(ID_Q_ACCOUNT, false), (ID_Q_VIA, true)] {
            let keep = combo_value(self.c(id));
            fill_combo(
                self.c(id),
                &sheets::quick_accounts(book, kind, is_via),
                keep,
            );
        }
    }
    fn refresh_guide(&self) {
        let value = match (self.book.as_ref(), combo_value(self.c(ID_Q_ACCOUNT))) {
            (Some(b), Some(a)) => sheets::guide(b, a),
            _ => String::new(),
        };
        set_text(self.c(ID_Q_GUIDE), &value);
    }
    fn refresh_suggestions(&self) {
        let h = self.c(ID_Q_SUGGEST);
        send(h, LB_RESETCONTENT, 0, 0);
        let Some(book) = self.book.as_ref() else {
            return;
        };
        for (id, label) in sheets::suggestions(book, &text(self.c(ID_Q_SEARCH))) {
            let value = wide(&label);
            let index = send(h, LB_ADDSTRING, 0, value.as_ptr() as isize).0;
            send(h, LB_SETITEMDATA, index as usize, id as isize);
        }
    }
    /// Picks a suggested account, switching the kind when the account belongs elsewhere.
    fn use_suggestion(&mut self) {
        let h = self.c(ID_Q_SUGGEST);
        let index = send(h, LB_GETCURSEL, 0, 0).0;
        if index < 0 {
            return;
        }
        let account = send(h, LB_GETITEMDATA, index as usize, 0).0 as i64;
        if !select_combo(self.c(ID_Q_ACCOUNT), account) {
            for (i, kind) in KINDS.iter().enumerate() {
                let fits = self.book.as_ref().is_some_and(|b| {
                    sheets::quick_accounts(b, *kind, false)
                        .iter()
                        .any(|(id, _)| *id == account)
                });
                if fits {
                    send(self.c(ID_Q_KIND), CB_SETCURSEL, i, 0);
                    self.fill_quick_choices();
                    select_combo(self.c(ID_Q_ACCOUNT), account);
                    self.update_visibility();
                    self.layout();
                    break;
                }
            }
        }
        self.refresh_guide();
    }
    fn journal_filter(&self) -> reports::EntryFilter {
        let query = text(self.c(ID_J_QUERY));
        reports::EntryFilter {
            from: get_date(self.c(ID_J_FROM)),
            to: get_date(self.c(ID_J_TO)),
            query: (!query.trim().is_empty()).then(|| query.trim().to_string()),
            include_void: send(self.c(ID_J_VOIDED), BM_GETCHECK, 0, 0).0 == BST_CHECKED.0 as isize,
            limit: Some(500),
            ..Default::default()
        }
    }
    fn refresh_journal(&mut self) {
        let Some(book) = self.book.clone() else {
            return;
        };
        let keep = selected_row(self.c(ID_J_LIST)).map(|r| r.1);
        let (table, total) = sheets::journal(&book, &self.journal_filter());
        let shown = table.rows.len();
        self.fill_table(ID_J_LIST, &table, keep);
        set_text(
            self.c(ID_J_COUNT),
            &if total > shown {
                format!("{total}件中 新しい{shown}件を表示（条件で絞り込めます）")
            } else {
                format!("{total}件")
            },
        );
        self.refresh_entry_lines();
    }
    fn selected_entry(&self) -> Option<Entry> {
        let (_, id) = selected_row(self.c(ID_J_LIST))?;
        self.book
            .as_ref()?
            .entries
            .iter()
            .find(|e| e.id == id)
            .cloned()
    }
    fn refresh_entry_lines(&mut self) {
        let entry = self.selected_entry();
        let table = match (&entry, self.book.as_ref()) {
            (Some(e), Some(b)) => sheets::lines(b, &e.lines),
            (None, Some(b)) => sheets::lines(b, &[]),
            _ => Table::default(),
        };
        self.fill_table(ID_J_LINES, &table, None);
        set_text(
            self.c(ID_J_TOGGLE),
            if entry.as_ref().is_some_and(|e| e.void) {
                "取消を解除"
            } else {
                "取消（残して無効化）"
            },
        );
        unsafe {
            for id in [ID_J_TOGGLE, ID_J_EDIT, ID_J_COPY] {
                let writer = WRITERS.contains(&id) || id == ID_J_COPY;
                let _ = EnableWindow(self.c(id), entry.is_some() && !(writer && self.read_only));
            }
        }
    }
    fn refresh_draft(&mut self) {
        let Some(book) = self.book.clone() else {
            return;
        };
        let table = sheets::lines(&book, &self.draft);
        let keep = self.line_edit.map(|i| i as i64);
        self.fill_table(ID_E_LIST, &table, keep);
        set_text(
            self.c(ID_E_TOTALS),
            &sheets::totals_text(&book, &self.draft),
        );
        set_text(
            self.c(ID_E_MODE),
            &match &self.editing {
                Some(e) => format!(
                    "仕訳 No.{} を修正しています（保存すると置き換え、変更は監査記録に残ります）",
                    e.id
                ),
                None => "新しい仕訳（借方・貸方の合計が一致すると保存できます）".into(),
            },
        );
        set_text(
            self.c(ID_E_ADD),
            if self.line_edit.is_some() {
                "行を更新"
            } else {
                "行を追加"
            },
        );
    }
    fn refresh_report(&mut self) {
        let Some(book) = self.book.clone() else {
            return;
        };
        let kind = combo_index(self.c(ID_R_KIND)).unwrap_or(0);
        let to = get_date(self.c(ID_R_TO)).unwrap_or_else(today);
        let from = get_date(self.c(ID_R_FROM));
        let from = if kind == 0 {
            from
        } else {
            from.or_else(|| Some(sheets::fiscal_start(&book, to)))
        };
        let account = combo_value(self.c(ID_R_ACCOUNT));
        match sheets::report(&book, kind, from, to, account) {
            Ok((table, summary)) => {
                self.fill_table(ID_R_LIST, &table, None);
                set_text(self.c(ID_R_SUMMARY), &summary);
            }
            Err(e) => {
                self.fill_table(ID_R_LIST, &Table::default(), None);
                set_text(self.c(ID_R_SUMMARY), &e.to_string());
            }
        }
        self.update_visibility();
    }
    fn update_status(&self) {
        let file = self
            .path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        let read_only = if self.read_only {
            "（読み取り専用）"
        } else {
            ""
        };
        unsafe {
            let _ = SetWindowTextW(
                self.hwnd,
                &HSTRING::from(format!("YYaccountant — {file}{read_only}")),
            );
        }
        let book = match &self.book {
            Some(b) => format!("{}　{}　版 {}", b.name, b.currency, b.revision),
            None => "帳簿がありません".into(),
        };
        let value = wide(&format!(
            "{}{read_only}　{book}　{}",
            self.path.display(),
            self.note
        ));
        send(self.status, SB_SETTEXTW, 0, value.as_ptr() as isize);
        for id in WRITERS {
            unsafe {
                let _ = EnableWindow(self.c(id), !self.read_only && self.book.is_some());
            }
        }
        unsafe {
            let menu = GetMenu(self.hwnd);
            for id in [ID_NEW_BOOK, ID_SETTINGS] {
                let _ = EnableMenuItem(
                    menu,
                    id as u32,
                    if self.read_only {
                        MF_GRAYED
                    } else {
                        MF_ENABLED
                    },
                );
            }
        }
    }

    // ----- changes -----
    /// Applies a change against the revision on screen; a stale revision reloads instead.
    fn apply(&mut self, change: Change) -> Result<Option<i64>> {
        let (id, revision) = {
            let b = self.book()?;
            (b.id, b.revision)
        };
        match self.store.apply(id, revision, &change, &self.actor) {
            Ok(applied) => {
                self.book = Some(applied.book);
                self.version = self.store.data_version()?;
                self.refill();
                Ok(applied.id)
            }
            Err(CoreError::Conflict) => {
                self.load_book(id)?;
                Err("他の利用者が先に更新しました。最新の内容を読み込んだので、確認してからもう一度保存してください".into())
            }
            Err(e) => Err(e.into()),
        }
    }
    fn quick_save(&mut self) -> Result<()> {
        let amount = sheets::amount(&text(self.c(ID_Q_AMOUNT)))?;
        if amount.is_zero() {
            return Err("金額を入れてください".into());
        }
        let business_percent = if self.ratio_applies() {
            let ratio = text(self.c(ID_Q_RATIO));
            if ratio.trim().is_empty() {
                None
            } else {
                Some(sheets::amount(&ratio)?)
            }
        } else {
            None
        };
        let quick = QuickEntry {
            kind: self.quick_kind(),
            date: get_date(self.c(ID_Q_DATE)).ok_or("日付を選んでください")?,
            amount,
            account: combo_value(self.c(ID_Q_ACCOUNT)).ok_or("科目を選んでください")?,
            via: combo_value(self.c(ID_Q_VIA)).ok_or("支払方法・入金先を選んでください")?,
            description: text(self.c(ID_Q_DESC)).trim().to_string(),
            partner: combo_value(self.c(ID_Q_PARTNER)).filter(|p| *p != 0),
            business_percent,
            memo: text(self.c(ID_Q_MEMO)).trim().to_string(),
        };
        let id = self.apply(Change::QuickEntry { quick })?;
        for c in [ID_Q_AMOUNT, ID_Q_DESC, ID_Q_MEMO] {
            set_text(self.c(c), "");
        }
        self.note = format!("仕訳 No.{} を登録しました", id.unwrap_or_default());
        self.update_status();
        unsafe {
            let _ = SetFocus(Some(self.c(ID_Q_AMOUNT)));
        }
        Ok(())
    }
    fn line_add(&mut self) -> Result<()> {
        let account = combo_value(self.c(ID_E_ACCOUNT)).ok_or("勘定科目を選んでください")?;
        let debit = sheets::amount(&text(self.c(ID_E_DEBIT)))?;
        let credit = sheets::amount(&text(self.c(ID_E_CREDIT)))?;
        if debit.is_zero() == credit.is_zero() {
            return Err("借方か貸方のどちらか一方に金額を入れてください".into());
        }
        if debit.is_sign_negative() || credit.is_sign_negative() {
            return Err("金額は正の値で入れてください（逆の側に入れると減少になります）".into());
        }
        let mut line = Line {
            account,
            debit,
            credit,
            memo: text(self.c(ID_E_MEMO)).trim().to_string(),
            claim: None,
        };
        match self.line_edit.take() {
            Some(i) if i < self.draft.len() => {
                line.claim = self.draft[i].claim;
                self.draft[i] = line;
            }
            _ => self.draft.push(line),
        }
        for c in [ID_E_DEBIT, ID_E_CREDIT, ID_E_MEMO] {
            set_text(self.c(c), "");
        }
        self.refresh_draft();
        select_row(self.c(ID_E_LIST), None);
        unsafe {
            let _ = SetFocus(Some(self.c(ID_E_ACCOUNT)));
        }
        Ok(())
    }
    fn line_selected(&mut self) {
        let Some((index, _)) = selected_row(self.c(ID_E_LIST)) else {
            return;
        };
        let Some(line) = self.draft.get(index).cloned() else {
            return;
        };
        let book = self.book.clone();
        let amount = |d: yy_accountant::num::Decimal| {
            if d.is_zero() {
                String::new()
            } else {
                yy_accountant::num::text(d)
            }
        };
        select_combo(self.c(ID_E_ACCOUNT), line.account);
        set_text(self.c(ID_E_DEBIT), &amount(line.debit));
        set_text(self.c(ID_E_CREDIT), &amount(line.credit));
        set_text(self.c(ID_E_MEMO), &line.memo);
        self.line_edit = Some(index);
        if book.is_some() {
            set_text(self.c(ID_E_ADD), "行を更新");
        }
    }
    fn line_remove(&mut self) {
        if let Some((index, _)) = selected_row(self.c(ID_E_LIST))
            && index < self.draft.len()
        {
            self.draft.remove(index);
            self.line_edit = None;
            self.refresh_draft();
        }
    }
    fn entry_clear(&mut self) {
        self.draft.clear();
        self.line_edit = None;
        self.editing = None;
        for c in [ID_E_DESC, ID_E_DEBIT, ID_E_CREDIT, ID_E_MEMO] {
            set_text(self.c(c), "");
        }
        set_date(self.c(ID_E_DATE), Some(today()));
        select_combo(self.c(ID_E_PARTNER), 0);
        self.refresh_draft();
    }
    fn entry_save(&mut self) -> Result<()> {
        let book = self.book()?.clone();
        let (debit, credit) = sheets::totals(&self.draft);
        if self.draft.len() < 2 || debit != credit || debit.is_zero() {
            return Err(sheets::totals_text(&book, &self.draft).into());
        }
        let original = self.editing.clone();
        let entry = Entry {
            id: original.as_ref().map(|e| e.id).unwrap_or(0),
            date: get_date(self.c(ID_E_DATE)).ok_or("日付を選んでください")?,
            description: text(self.c(ID_E_DESC)).trim().to_string(),
            partner: combo_value(self.c(ID_E_PARTNER)).filter(|p| *p != 0),
            source: original
                .as_ref()
                .map(|e| e.source.clone())
                .unwrap_or_default(),
            void: original.as_ref().is_some_and(|e| e.void),
            lines: self.draft.clone(),
        };
        let id = self.apply(Change::SaveEntry { entry })?;
        self.entry_clear();
        self.note = format!("仕訳 No.{} を保存しました", id.unwrap_or_default());
        self.update_status();
        Ok(())
    }
    /// Loads a journal entry into the compound editor, to correct it or to copy it.
    fn entry_load(&mut self, correct: bool) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        self.draft = entry.lines.clone();
        if !correct {
            for l in &mut self.draft {
                l.claim = None;
            }
        }
        self.line_edit = None;
        set_date(
            self.c(ID_E_DATE),
            Some(if correct { entry.date } else { today() }),
        );
        set_text(self.c(ID_E_DESC), &entry.description);
        if !select_combo(self.c(ID_E_PARTNER), entry.partner.unwrap_or(0)) {
            select_combo(self.c(ID_E_PARTNER), 0);
        }
        self.editing = correct.then_some(entry);
        self.refresh_draft();
        self.set_page(2);
    }
    fn toggle_void(&mut self) -> Result<()> {
        let entry = self.selected_entry().ok_or("仕訳を選んでください")?;
        self.apply(Change::VoidEntry {
            id: entry.id,
            void: !entry.void,
        })?;
        self.note = if entry.void {
            format!("仕訳 No.{} の取消を解除しました", entry.id)
        } else {
            format!("仕訳 No.{} を取消にしました（削除せず残します）", entry.id)
        };
        self.update_status();
        Ok(())
    }
    fn open(&mut self, path: PathBuf, read_only: bool) -> Result<()> {
        let store = Store::open(&path, read_only)?;
        self.store = store;
        self.path = path;
        self.read_only = read_only;
        self.book = None;
        self.version = self.store.data_version()?;
        self.note = String::new();
        self.reload_books()
    }
    /// Polls SQLite's data_version so changes from MCP, the Web UI or another window appear.
    fn poll(&mut self) -> Result<()> {
        let version = self.store.data_version()?;
        if version == self.version {
            return Ok(());
        }
        self.version = version;
        let before: Vec<(i64, i64)> = self.books.iter().map(|b| (b.id, b.revision)).collect();
        let books = self.store.list()?;
        let after: Vec<(i64, i64)> = books.iter().map(|b| (b.id, b.revision)).collect();
        let current = self.book.as_ref().map(|b| (b.id, b.revision));
        if before != after || current.is_some_and(|(id, rev)| !after.contains(&(id, rev))) {
            self.reload_books()?;
            self.note = "ほかの画面やAIの変更を読み込みました".into();
            self.update_status();
        }
        Ok(())
    }

    // ----- layout -----
    fn place(&self, id: u16, x: i32, y: i32, width: i32, height: i32) {
        let c = &self.controls[&id];
        let height = if c.kind == Kind::Combo {
            self.px(320)
        } else {
            height
        };
        unsafe {
            let _ = MoveWindow(c.hwnd, x, y, width.max(1), height.max(1), true);
        }
    }
    /// Label and field side by side; returns the x after the field.
    fn label_width(&self, id: u16) -> i32 {
        self.px(10 + 14 * text(self.c(id + LABEL)).chars().count() as i32)
    }
    /// Width left for a flexible field at `x` once its label and the fields after it fit.
    fn rest(&self, id: u16, x: i32, right: i32, after: i32) -> i32 {
        right - x - self.label_width(id) - self.px(14) - after
    }
    fn inline(&self, id: u16, x: i32, y: i32, width: i32) -> i32 {
        let lw = self.label_width(id);
        let h = self.px(26);
        self.place(id + LABEL, x, y, lw, h);
        self.place(id, x + lw, y, width, h);
        x + lw + width + self.px(14)
    }
    fn layout(&self) {
        let mut client = RECT::default();
        let mut bar = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut client);
            send(self.status, WM_SIZE, 0, 0);
            let _ = GetWindowRect(self.status, &mut bar);
        }
        let m = self.px(10);
        let h = self.px(26);
        let step = self.px(34);
        let width = client.right - 2 * m;
        self.place(ID_BOOK + LABEL, m, m, self.px(44), h);
        self.place(
            ID_BOOK,
            m + self.px(44),
            m,
            self.px(360).min(width - self.px(44)),
            h,
        );
        let mut tab = RECT::default();
        send(self.tabs, TCM_GETITEMRECT, 0, &mut tab as *mut _ as isize);
        let tab_top = m + h + self.px(8);
        let tab_height = (tab.bottom - tab.top).max(self.px(24)) + self.px(4);
        self.place(ID_TABS, m, tab_top, width, tab_height);
        let top = tab_top + tab_height + self.px(10);
        let bottom = client.bottom - (bar.bottom - bar.top) - m;
        let right = m + width;
        let x = m;
        match self.page {
            0 => {
                let col = (width * 11 / 20).clamp(self.px(380), self.px(560));
                let lw = self.px(104);
                let fw = col - lw;
                let mut y = top;
                for id in [
                    ID_Q_KIND,
                    ID_Q_DATE,
                    ID_Q_AMOUNT,
                    ID_Q_ACCOUNT,
                    ID_Q_VIA,
                    ID_Q_PARTNER,
                    ID_Q_DESC,
                    ID_Q_RATIO,
                    ID_Q_MEMO,
                ] {
                    if id == ID_Q_RATIO && !self.ratio_applies() {
                        continue;
                    }
                    self.place(id + LABEL, x, y, lw, h);
                    let w = if matches!(id, ID_Q_KIND | ID_Q_DATE | ID_Q_AMOUNT | ID_Q_RATIO) {
                        fw.min(self.px(200))
                    } else {
                        fw
                    };
                    self.place(id, x + lw, y, w, h);
                    y += step;
                }
                self.place(ID_Q_SAVE, x + lw, y + self.px(4), self.px(160), self.px(32));
                let rx = x + col + self.px(20);
                let rw = right - rx;
                let label = self.px(96);
                self.place(ID_Q_SEARCH + LABEL, rx, top, label, h);
                self.place(ID_Q_SEARCH, rx + label, top, rw - label, h);
                self.place(ID_Q_SUGGEST, rx, top + step, rw, self.px(118));
                let gy = top + step + self.px(126);
                self.place(ID_Q_GUIDE, rx, gy, rw, bottom - gy);
            }
            1 => {
                let mut cx = self.inline(ID_J_FROM, x, top, self.px(140));
                cx = self.inline(ID_J_TO, cx, top, self.px(140));
                let check = self.px(140);
                let query = self.rest(ID_J_QUERY, cx, right, check).max(self.px(120));
                cx = self.inline(ID_J_QUERY, cx, top, query);
                self.place(ID_J_VOIDED, cx, top, check, h);
                let buttons = bottom - self.px(32);
                let lines_h = self.px(130);
                let list_top = top + step;
                let list_bottom = buttons - lines_h - self.px(16);
                self.place(ID_J_LIST, x, list_top, width, list_bottom - list_top);
                self.place(ID_J_LINES, x, list_bottom + self.px(8), width, lines_h);
                self.place(ID_J_TOGGLE, x, buttons, self.px(180), self.px(32));
                self.place(
                    ID_J_EDIT,
                    x + self.px(190),
                    buttons,
                    self.px(110),
                    self.px(32),
                );
                self.place(
                    ID_J_COPY,
                    x + self.px(310),
                    buttons,
                    self.px(150),
                    self.px(32),
                );
                let count = x + self.px(476);
                self.place(ID_J_COUNT, count, buttons, right - count, self.px(32));
            }
            2 => {
                let partner = self.px(200);
                let mut cx = self.inline(ID_E_DATE, x, top, self.px(140));
                let after = self.label_width(ID_E_PARTNER) + partner;
                let desc = self.rest(ID_E_DESC, cx, right, after).max(self.px(120));
                cx = self.inline(ID_E_DESC, cx, top, desc);
                self.inline(ID_E_PARTNER, cx, top, partner);
                let y = top + step;
                let button = self.px(96);
                cx = self.inline(ID_E_ACCOUNT, x, y, self.px(220));
                cx = self.inline(ID_E_DEBIT, cx, y, self.px(110));
                cx = self.inline(ID_E_CREDIT, cx, y, self.px(110));
                let memo = self.rest(ID_E_MEMO, cx, right, 0).max(self.px(80));
                self.inline(ID_E_MEMO, cx, y, memo);
                // The line buttons sit at the right of the mode line, under the line fields.
                let y = y + step;
                let buttons_x = right - 2 * button - self.px(6);
                self.place(ID_E_MODE, x, y, buttons_x - x - self.px(10), h);
                self.place(ID_E_ADD, buttons_x, y, button, h);
                self.place(ID_E_REMOVE, right - button, y, button, h);
                let y = y - step;
                let buttons = bottom - self.px(32);
                let list_top = y + 2 * step;
                self.place(
                    ID_E_LIST,
                    x,
                    list_top,
                    width,
                    buttons - self.px(10) - list_top,
                );
                let save = self.px(150);
                let clear = self.px(130);
                self.place(
                    ID_E_TOTALS,
                    x,
                    buttons,
                    width - save - clear - self.px(20),
                    self.px(32),
                );
                self.place(
                    ID_E_CLEAR,
                    right - save - clear - self.px(8),
                    buttons,
                    clear,
                    self.px(32),
                );
                self.place(ID_E_SAVE, right - save, buttons, save, self.px(32));
            }
            _ => {
                let mut cx = self.inline(ID_R_KIND, x, top, self.px(150));
                cx = self.inline(ID_R_FROM, cx, top, self.px(150));
                cx = self.inline(ID_R_TO, cx, top, self.px(140));
                let account = self
                    .rest(ID_R_ACCOUNT, cx, right, 0)
                    .clamp(self.px(120), self.px(280));
                self.inline(ID_R_ACCOUNT, cx, top, account);
                let summary = bottom - self.px(28);
                self.place(
                    ID_R_LIST,
                    x,
                    top + step,
                    width,
                    summary - self.px(8) - top - step,
                );
                self.place(ID_R_SUMMARY, x, summary, width, self.px(28));
            }
        }
    }

    // ----- events -----
    fn command(&mut self, id: u16, code: u32) -> Result<()> {
        match (id, code) {
            (ID_BOOK, CBN_SELCHANGE) => {
                if let Some(book) = combo_value(self.c(ID_BOOK)) {
                    self.load_book(book)?;
                }
            }
            (ID_Q_KIND, CBN_SELCHANGE) => {
                self.fill_quick_choices();
                self.refresh_guide();
                self.update_visibility();
                self.layout();
            }
            (ID_Q_ACCOUNT, CBN_SELCHANGE) => self.refresh_guide(),
            (ID_Q_SEARCH, EN_CHANGE) => self.refresh_suggestions(),
            (ID_Q_SUGGEST, LBN_SELCHANGE) => self.use_suggestion(),
            (ID_Q_SAVE, BN_CLICKED) => self.quick_save()?,
            (ID_J_QUERY, EN_CHANGE) | (ID_J_VOIDED, BN_CLICKED) => self.refresh_journal(),
            (ID_J_TOGGLE, BN_CLICKED) => self.toggle_void()?,
            (ID_J_EDIT, BN_CLICKED) => self.entry_load(true),
            (ID_J_COPY, BN_CLICKED) => self.entry_load(false),
            (ID_E_ADD, BN_CLICKED) => self.line_add()?,
            (ID_E_REMOVE, BN_CLICKED) => self.line_remove(),
            (ID_E_CLEAR, BN_CLICKED) => self.entry_clear(),
            (ID_E_SAVE, BN_CLICKED) => self.entry_save()?,
            (ID_R_KIND | ID_R_ACCOUNT, CBN_SELCHANGE) => self.refresh_report(),
            // Enter in a field: register on the quick page, add the line in the editor.
            (1, _) => match self.page {
                0 if !self.read_only => self.quick_save()?,
                2 if !self.read_only => self.line_add()?,
                _ => {}
            },
            (ID_REFRESH, _) => {
                self.reload_books()?;
                self.note = "最新の内容を読み込みました".into();
                self.update_status();
            }
            (ID_NEXT_PAGE, _) => self.set_page((self.page + 1) % PAGES.len()),
            (ID_PREV_PAGE, _) => self.set_page((self.page + PAGES.len() - 1) % PAGES.len()),
            (p, _) if (ID_PAGE..ID_PAGE + PAGES.len() as u16).contains(&p) => {
                self.set_page((p - ID_PAGE) as usize)
            }
            _ => {}
        }
        Ok(())
    }
    fn notify(&mut self, header: &NMHDR) {
        let id = header.idFrom as u16;
        match (id, header.code) {
            (ID_TABS, TCN_SELCHANGE) => {
                let page = send(self.tabs, TCM_GETCURSEL, 0, 0).0.max(0) as usize;
                self.set_page(page);
            }
            (ID_J_LIST, LVN_ITEMCHANGED) => self.refresh_entry_lines(),
            (ID_J_LIST, NM_DBLCLK) if !self.read_only => self.entry_load(true),
            (ID_E_LIST, LVN_ITEMCHANGED) => self.line_selected(),
            (ID_J_FROM | ID_J_TO, DTN_DATETIMECHANGE) => self.refresh_journal(),
            (ID_R_FROM | ID_R_TO, DTN_DATETIMECHANGE) => self.refresh_report(),
            _ => {}
        }
    }
    fn set_font(&mut self) {
        unsafe {
            let _ = DeleteObject(self.font.into());
        }
        self.font = window_font(self.dpi);
        for c in self.controls.values() {
            send(c.hwnd, WM_SETFONT, self.font.0 as usize, 1);
        }
        send(self.tabs, WM_SETFONT, self.font.0 as usize, 1);
        send(self.status, WM_SETFONT, self.font.0 as usize, 1);
        self.headings.clear();
    }
}

/// Runs `f` on the window state unless a handler further up the stack already holds it
/// (notifications sent while a handler updates controls are ignored that way).
fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    APP.with(|a| {
        let mut a = a.try_borrow_mut().ok()?;
        a.as_mut().map(f)
    })
}
fn report(h: HWND, outcome: Option<Result<()>>) {
    if let Some(Err(e)) = outcome {
        message(h, &e.to_string(), true);
    }
}

/// Menu and dialog commands that need user input before touching the state.
fn menu_command(h: HWND, id: u16) {
    match id {
        ID_OPEN | ID_OPEN_READ_ONLY => {
            if let Some(path) = choose_file(h, false, "データベースを開く", DATABASE_FILTER, "")
            {
                report(h, with(|s| s.open(path, id == ID_OPEN_READ_ONLY)));
            }
        }
        ID_NEW_DATABASE => {
            if let Some(path) = choose_file(
                h,
                true,
                "新しいデータベース",
                DATABASE_FILTER,
                "books.sqlite3",
            ) {
                if path.exists() {
                    message(
                        h,
                        "新しいデータベースには、まだないファイル名を指定してください",
                        true,
                    );
                    return;
                }
                report(h, with(|s| s.open(path, false)));
                new_book(h);
            }
        }
        ID_DEMO => {
            if let Some(path) = choose_file(
                h,
                true,
                "デモデータベースの作成",
                DATABASE_FILTER,
                "yyaccountant-demo.sqlite3",
            ) {
                let outcome = crate::demo::create(&path, today())
                    .and_then(|()| with(|s| s.open(path, false)).unwrap_or(Ok(())));
                report(h, Some(outcome));
            }
        }
        ID_BACKUP => {
            if let Some(path) = choose_file(
                h,
                true,
                "バックアップ（新しいファイル）",
                DATABASE_FILTER,
                "yyaccountant-backup.sqlite3",
            ) {
                let outcome = with(|s| -> Result<()> {
                    s.store.backup(&path)?;
                    s.note = format!("{} にバックアップしました", path.display());
                    s.update_status();
                    Ok(())
                });
                report(h, outcome);
            }
        }
        ID_CSV => {
            let name = with(|s| s.book.as_ref().map(|b| format!("{}-仕訳.csv", b.name)))
                .flatten()
                .unwrap_or_else(|| "仕訳.csv".into());
            if let Some(path) = choose_file(h, true, "仕訳CSVを保存", "CSV (*.csv)|*.csv", &name)
            {
                let outcome = with(|s| -> Result<()> {
                    let csv = reports::journal_csv(s.book()?, None, None);
                    std::fs::write(&path, csv)?;
                    s.note = format!("{} に保存しました", path.display());
                    s.update_status();
                    Ok(())
                });
                report(h, outcome);
            }
        }
        ID_NEW_BOOK => new_book(h),
        ID_SETTINGS => book_settings(h),
        ID_ABOUT => message(
            h,
            &format!(
                "YYaccountant {}\n\n家計から個人事業主までの複式簿記を1つのSQLiteファイルで管理します。金額はすべて十進数で計算します。\n\nCtrl+1〜4: 画面の切り替え（Ctrl+Tabでも移動）\nEnter: かんたん入力では登録、仕訳入力では行を追加\nF5: 最新の内容に更新\n\nほかの画面やAI（MCP）が同じファイルを更新すると自動で読み込みます。",
                env!("CARGO_PKG_VERSION")
            ),
            false,
        ),
        ID_EXIT => unsafe {
            let _ = DestroyWindow(h);
        },
        _ => {}
    }
}
fn book_fields(
    name: &str,
    kind: usize,
    currency: &str,
    digits: u32,
    month: u32,
    rounding: usize,
) -> Vec<Field> {
    vec![
        Field::Text("帳簿名", name.into()),
        Field::Choice(
            "種類",
            sheets::book_kinds().into_iter().map(|k| k.1).collect(),
            kind,
        ),
        Field::Text("通貨（ISO 4217）", currency.into()),
        Field::Choice(
            "小数の桁数",
            (0..=4).map(|d| format!("{d}桁")).collect(),
            digits as usize,
        ),
        Field::Choice(
            "年度の開始月",
            (1..=12).map(|m| format!("{m}月")).collect(),
            month as usize - 1,
        ),
        Field::Choice(
            "端数処理",
            Rounding::ALL
                .iter()
                .map(|r| r.label().to_string())
                .collect(),
            rounding,
        ),
    ]
}
fn new_book(h: HWND) {
    if with(|s| s.read_only) != Some(false) {
        return;
    }
    let Some(v) = form::show(h, "新しい帳簿", book_fields("", 0, "JPY", 0, 1, 0)) else {
        return;
    };
    let outcome = with(|s| -> Result<()> {
        let request = NewBook {
            name: v[0].trim().into(),
            kind: BookKind::ALL[v[1].parse::<usize>()?.min(1)],
            currency: v[2].trim().to_uppercase(),
            minor_digits: v[3].parse()?,
            rounding: Rounding::ALL[v[5].parse::<usize>()?.min(3)],
            fiscal_start_month: v[4].parse::<u32>()? + 1,
        };
        let book = s.store.create(&request, &s.actor)?;
        s.version = s.store.data_version()?;
        s.books = s.store.list()?;
        s.book = None;
        s.reload_books()?;
        s.load_book(book.id)?;
        s.note = format!("帳簿「{}」を作成しました", book.name);
        s.update_status();
        Ok(())
    });
    report(h, outcome);
}
fn book_settings(h: HWND) {
    let Some(Some(b)) = with(|s| s.book.clone()) else {
        return;
    };
    let kind = BookKind::ALL.iter().position(|k| *k == b.kind).unwrap_or(0);
    let rounding = Rounding::ALL
        .iter()
        .position(|r| *r == b.rounding)
        .unwrap_or(0);
    let mut fields = book_fields(
        &b.name,
        kind,
        &b.currency,
        b.minor_digits,
        b.fiscal_start_month,
        rounding,
    );
    // The chart of accounts depends on the kind, so it is fixed after creation.
    fields.remove(1);
    let Some(v) = form::show(h, "帳簿の設定", fields) else {
        return;
    };
    let outcome = with(|s| -> Result<()> {
        s.apply(Change::UpdateBook {
            name: v[0].trim().into(),
            currency: v[1].trim().to_uppercase(),
            minor_digits: v[2].parse()?,
            fiscal_start_month: v[3].parse::<u32>()? + 1,
            rounding: Rounding::ALL[v[4].parse::<usize>()?.min(3)],
        })?;
        s.reload_books()?;
        s.note = "帳簿の設定を保存しました".into();
        s.update_status();
        Ok(())
    });
    report(h, outcome);
}

extern "system" fn proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        match m {
            WM_COMMAND => {
                let id = (w.0 & 0xffff) as u16;
                let code = ((w.0 >> 16) & 0xffff) as u32;
                if l.0 == 0 && DIALOGS.contains(&id) {
                    menu_command(h, id);
                } else {
                    report(h, with(|s| s.command(id, code)));
                }
                return LRESULT(0);
            }
            WM_NOTIFY => {
                let header = &*(l.0 as *const NMHDR);
                with(|s| s.notify(header));
            }
            WM_SIZE => {
                with(|s| s.layout());
                return LRESULT(0);
            }
            WM_TIMER if w.0 == TIMER => {
                report(h, with(|s| s.poll()));
                return LRESULT(0);
            }
            WM_GETMINMAXINFO => {
                let dpi = GetDpiForWindow(h).max(96) as i32;
                let info = &mut *(l.0 as *mut MINMAXINFO);
                info.ptMinTrackSize = POINT {
                    x: scale(dpi, 860),
                    y: scale(dpi, 600),
                };
                return LRESULT(0);
            }
            WM_DPICHANGED => {
                let suggested = &*(l.0 as *const RECT);
                with(|s| {
                    s.dpi = ((w.0 >> 16) & 0xffff) as i32;
                    s.set_font();
                    s.refill();
                });
                let _ = SetWindowPos(
                    h,
                    None,
                    suggested.left,
                    suggested.top,
                    suggested.right - suggested.left,
                    suggested.bottom - suggested.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                return LRESULT(0);
            }
            WM_DESTROY => {
                let _ = KillTimer(Some(h), TIMER);
                PostQuitMessage(0);
                return LRESULT(0);
            }
            _ => {}
        }
        DefWindowProcW(h, m, w, l)
    }
}

fn menu() -> Result<HMENU> {
    unsafe {
        let bar = CreateMenu()?;
        let add = |menu: HMENU, items: &[(u16, &str)]| -> Result<()> {
            for (id, label) in items {
                if *id == 0 {
                    AppendMenuW(menu, MF_SEPARATOR, 0, None)?;
                } else {
                    AppendMenuW(menu, MF_STRING, *id as usize, &HSTRING::from(*label))?;
                }
            }
            Ok(())
        };
        let file = CreatePopupMenu()?;
        add(
            file,
            &[
                (ID_OPEN, "開く(&O)...\tCtrl+O"),
                (ID_OPEN_READ_ONLY, "読み取り専用で開く(&R)..."),
                (ID_NEW_DATABASE, "新しいデータベース(&N)..."),
                (ID_DEMO, "デモデータベースを作成(&D)..."),
                (0, ""),
                (ID_BACKUP, "バックアップ(&B)..."),
                (ID_CSV, "仕訳CSVを保存(&C)..."),
                (0, ""),
                (ID_EXIT, "終了(&X)"),
            ],
        )?;
        let book = CreatePopupMenu()?;
        add(
            book,
            &[
                (ID_NEW_BOOK, "新しい帳簿(&N)..."),
                (ID_SETTINGS, "帳簿の設定(&S)..."),
                (0, ""),
                (ID_REFRESH, "最新の内容に更新(&R)\tF5"),
            ],
        )?;
        let view = CreatePopupMenu()?;
        for (i, page) in PAGES.iter().enumerate() {
            AppendMenuW(
                view,
                MF_STRING,
                (ID_PAGE + i as u16) as usize,
                &HSTRING::from(format!("{page}(&{})\tCtrl+{}", i + 1, i + 1)),
            )?;
        }
        let help = CreatePopupMenu()?;
        add(help, &[(ID_ABOUT, "YYaccountantについて(&A)")])?;
        for (popup, title) in [
            (file, "ファイル(&F)"),
            (book, "帳簿(&B)"),
            (view, "表示(&V)"),
            (help, "ヘルプ(&H)"),
        ] {
            AppendMenuW(bar, MF_POPUP, popup.0 as usize, &HSTRING::from(title))?;
        }
        Ok(bar)
    }
}
fn accelerators() -> Result<HACCEL> {
    let key = |flags: ACCEL_VIRT_FLAGS, key: u16, cmd: u16| ACCEL {
        fVirt: flags | FVIRTKEY,
        key,
        cmd,
    };
    let mut table = vec![
        key(FCONTROL, u16::from(b'O'), ID_OPEN),
        key(ACCEL_VIRT_FLAGS(0), VK_F5.0, ID_REFRESH),
        key(FCONTROL, VK_TAB.0, ID_NEXT_PAGE),
        key(FCONTROL | FSHIFT, VK_TAB.0, ID_PREV_PAGE),
        key(FCONTROL, VK_NEXT.0, ID_NEXT_PAGE),
        key(FCONTROL, VK_PRIOR.0, ID_PREV_PAGE),
    ];
    for i in 0..PAGES.len() as u16 {
        table.push(key(FCONTROL, u16::from(b'1') + i, ID_PAGE + i));
    }
    Ok(unsafe { CreateAcceleratorTableW(&table)? })
}

/// Creates every control once; pages only show and hide them.
fn controls(h: HWND, instance: HINSTANCE) -> Result<BTreeMap<u16, Control>> {
    let mut map = BTreeMap::new();
    let edit = ES_AUTOHSCROLL as u32;
    let combo = CBS_DROPDOWNLIST as u32 | WS_VSCROLL.0;
    let date = DTS_SHORTDATECENTURYFORMAT;
    let optional_date = DTS_SHORTDATECENTURYFORMAT | DTS_SHOWNONE;
    let button = BS_PUSHBUTTON as u32;
    let table = LVS_REPORT | LVS_SHOWSELALWAYS | LVS_SINGLESEL;
    let spec: Vec<(Option<usize>, u16, Kind, &str, u32)> = vec![
        (None, ID_BOOK, Kind::Combo, "帳簿", combo),
        (Some(0), ID_Q_KIND, Kind::Combo, "種類", combo),
        (Some(0), ID_Q_DATE, Kind::Date, "日付", date),
        (Some(0), ID_Q_AMOUNT, Kind::Edit, "金額", edit),
        (Some(0), ID_Q_ACCOUNT, Kind::Combo, "費用の科目", combo),
        (Some(0), ID_Q_VIA, Kind::Combo, "支払方法", combo),
        (Some(0), ID_Q_PARTNER, Kind::Combo, "取引先", combo),
        (Some(0), ID_Q_DESC, Kind::Edit, "内容", edit),
        (Some(0), ID_Q_RATIO, Kind::Edit, "事業割合（%）", edit),
        (Some(0), ID_Q_MEMO, Kind::Edit, "メモ", edit),
        (
            Some(0),
            ID_Q_SAVE,
            Kind::Button,
            "登録",
            BS_DEFPUSHBUTTON as u32,
        ),
        (Some(0), ID_Q_SEARCH, Kind::Edit, "科目を探す", edit),
        (
            Some(0),
            ID_Q_SUGGEST,
            Kind::List,
            "候補",
            (LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32 | WS_VSCROLL.0,
        ),
        (
            Some(0),
            ID_Q_GUIDE,
            Kind::Edit,
            "科目の使い方",
            (ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32 | WS_VSCROLL.0,
        ),
        (Some(1), ID_J_FROM, Kind::Date, "開始", optional_date),
        (Some(1), ID_J_TO, Kind::Date, "終了", optional_date),
        (Some(1), ID_J_QUERY, Kind::Edit, "検索", edit),
        (
            Some(1),
            ID_J_VOIDED,
            Kind::Button,
            "取消も表示",
            BS_AUTOCHECKBOX as u32,
        ),
        (Some(1), ID_J_LIST, Kind::Table, "仕訳", table),
        (
            Some(1),
            ID_J_LINES,
            Kind::Table,
            "明細",
            table | LVS_NOSORTHEADER,
        ),
        (
            Some(1),
            ID_J_TOGGLE,
            Kind::Button,
            "取消（残して無効化）",
            button,
        ),
        (Some(1), ID_J_EDIT, Kind::Button, "修正...", button),
        (Some(1), ID_J_COPY, Kind::Button, "複写して入力...", button),
        (Some(1), ID_J_COUNT, Kind::Label, "", 0),
        (Some(2), ID_E_DATE, Kind::Date, "日付", date),
        (Some(2), ID_E_DESC, Kind::Edit, "摘要", edit),
        (Some(2), ID_E_PARTNER, Kind::Combo, "取引先", combo),
        (Some(2), ID_E_ACCOUNT, Kind::Combo, "科目", combo),
        (
            Some(2),
            ID_E_DEBIT,
            Kind::Edit,
            "借方",
            edit | ES_RIGHT as u32,
        ),
        (
            Some(2),
            ID_E_CREDIT,
            Kind::Edit,
            "貸方",
            edit | ES_RIGHT as u32,
        ),
        (Some(2), ID_E_MEMO, Kind::Edit, "メモ", edit),
        (Some(2), ID_E_ADD, Kind::Button, "行を追加", button),
        (Some(2), ID_E_REMOVE, Kind::Button, "行を削除", button),
        (Some(2), ID_E_MODE, Kind::Label, "", 0),
        (
            Some(2),
            ID_E_LIST,
            Kind::Table,
            "明細",
            table | LVS_NOSORTHEADER,
        ),
        (Some(2), ID_E_TOTALS, Kind::Label, "", 0),
        (Some(2), ID_E_CLEAR, Kind::Button, "新規に戻す", button),
        (Some(2), ID_E_SAVE, Kind::Button, "仕訳を保存", button),
        (Some(3), ID_R_KIND, Kind::Combo, "帳票", combo),
        (Some(3), ID_R_FROM, Kind::Date, "開始", optional_date),
        (Some(3), ID_R_TO, Kind::Date, "終了", date),
        (Some(3), ID_R_ACCOUNT, Kind::Combo, "科目", combo),
        (
            Some(3),
            ID_R_LIST,
            Kind::Table,
            "帳票",
            table | LVS_NOSORTHEADER,
        ),
        (Some(3), ID_R_SUMMARY, Kind::Label, "", 0),
    ];
    let create = |class: PCWSTR, title: &str, style: u32, ex: WINDOW_EX_STYLE, id: u16| unsafe {
        CreateWindowExW(
            ex,
            class,
            &HSTRING::from(title),
            WS_CHILD | WS_CLIPSIBLINGS | WINDOW_STYLE(style),
            0,
            0,
            1,
            1,
            Some(h),
            Some(HMENU(id as usize as *mut _)),
            Some(instance),
            None,
        )
    };
    let static_style = STATIC_STYLE;
    for (page, id, kind, title, style) in spec {
        let labelled =
            matches!(kind, Kind::Edit | Kind::Combo | Kind::Date) && !matches!(id, ID_Q_GUIDE);
        if labelled {
            // The label precedes its field so screen readers and UI Automation name the field.
            let hwnd = create(
                w!("STATIC"),
                title,
                static_style,
                WINDOW_EX_STYLE(0),
                id + LABEL,
            )?;
            map.insert(
                id + LABEL,
                Control {
                    hwnd,
                    page,
                    kind: Kind::Label,
                },
            );
        }
        let tab = if kind == Kind::Label { 0 } else { WS_TABSTOP.0 };
        let (class, ex, body) = match kind {
            Kind::Label => (w!("STATIC"), WINDOW_EX_STYLE(0), static_style),
            Kind::Edit => (w!("EDIT"), WS_EX_CLIENTEDGE, style),
            Kind::Combo => (w!("COMBOBOX"), WINDOW_EX_STYLE(0), style),
            Kind::Date => (DATETIMEPICK_CLASSW, WINDOW_EX_STYLE(0), style),
            Kind::Button => (w!("BUTTON"), WINDOW_EX_STYLE(0), style),
            Kind::List => (w!("LISTBOX"), WS_EX_CLIENTEDGE, style),
            Kind::Table => (WC_LISTVIEWW, WS_EX_CLIENTEDGE, style),
        };
        let shown_title = if labelled || kind == Kind::Label {
            ""
        } else {
            title
        };
        let hwnd = create(class, shown_title, body | tab, ex, id)?;
        if kind == Kind::Table {
            send(
                hwnd,
                LVM_SETEXTENDEDLISTVIEWSTYLE,
                0,
                (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER | LVS_EX_GRIDLINES | LVS_EX_LABELTIP)
                    as isize,
            );
        }
        if kind == Kind::List || kind == Kind::Table {
            // Name for UI Automation; list views read the window text.
            unsafe {
                let _ = SetWindowTextW(hwnd, &HSTRING::from(title));
            }
        }
        map.insert(id, Control { hwnd, page, kind });
    }
    Ok(map)
}

pub fn run() -> Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let common = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_TAB_CLASSES
                | ICC_LISTVIEW_CLASSES
                | ICC_DATE_CLASSES
                | ICC_BAR_CLASSES
                | ICC_STANDARD_CLASSES,
        };
        let _ = InitCommonControlsEx(&common);
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        let smoke_dir = if args.first().is_some_and(|a| a == "--ui-smoke-test") {
            Some(PathBuf::from(args.get(1).ok_or("Missing smoke directory")?))
        } else {
            None
        };
        SMOKE.with(|s| s.set(smoke_dir.is_some()));
        let read_only = args.iter().any(|a| a == "--read-only");
        let path = if let Some(directory) = &smoke_dir {
            std::fs::create_dir_all(directory)?;
            let path = directory.join("ui-test.sqlite3");
            if path.exists() {
                return Err("Smoke test requires a fresh directory".into());
            }
            path
        } else {
            match args.iter().find(|a| *a != "--read-only") {
                Some(p) => PathBuf::from(p),
                None => {
                    let dir = PathBuf::from(
                        std::env::var_os("LOCALAPPDATA").unwrap_or_else(|| ".".into()),
                    )
                    .join("YYaccountant");
                    std::fs::create_dir_all(&dir)?;
                    dir.join("books.sqlite3")
                }
            }
        };
        let store = Store::open(&path, read_only)?;
        let instance: HINSTANCE = GetModuleHandleW(None)?.into();
        let icon =
            LoadIconW(Some(instance), PCWSTR(std::ptr::without_provenance(1))).unwrap_or_default();
        RegisterClassW(&WNDCLASSW {
            hInstance: instance,
            lpszClassName: w!("YYAccountantFrame"),
            lpfnWndProc: Some(proc),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hIcon: icon,
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            ..Default::default()
        });
        form::register(instance, icon)?;
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            w!("YYAccountantFrame"),
            w!("YYaccountant"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1180,
            780,
            None,
            Some(menu()?),
            Some(instance),
            None,
        )?;
        let dpi = GetDpiForWindow(hwnd).max(96) as i32;
        if smoke_dir.is_none() {
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                scale(dpi, 1180),
                scale(dpi, 780),
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        let font = window_font(dpi);
        let tabs = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            WC_TABCONTROLW,
            w!(""),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_CLIPSIBLINGS,
            0,
            0,
            1,
            1,
            Some(hwnd),
            Some(HMENU(ID_TABS as usize as *mut _)),
            Some(instance),
            None,
        )?;
        let status = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            STATUSCLASSNAMEW,
            w!(""),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SBARS_SIZEGRIP),
            0,
            0,
            0,
            0,
            Some(hwnd),
            Some(HMENU(ID_STATUS as usize as *mut _)),
            Some(instance),
            None,
        )?;
        for (i, title) in PAGES.iter().enumerate() {
            let mut value = wide(title);
            let item = TCITEMW {
                mask: TCIF_TEXT,
                pszText: PWSTR(value.as_mut_ptr()),
                ..Default::default()
            };
            send(tabs, TCM_INSERTITEMW, i, &item as *const _ as isize);
        }
        let mut controls = controls(hwnd, instance)?;
        controls.insert(
            ID_TABS,
            Control {
                hwnd: tabs,
                page: None,
                kind: Kind::Label,
            },
        );
        let version = store.data_version()?;
        let actor = std::env::var("USERNAME")
            .ok()
            .filter(|u| !u.trim().is_empty())
            .map(|u| format!("{u}（Windows）"))
            .unwrap_or_else(|| "Windows".into());
        let mut state = State {
            hwnd,
            tabs,
            status,
            font,
            dpi,
            store,
            path,
            read_only,
            actor: if smoke_dir.is_some() {
                "UI-test".into()
            } else {
                actor
            },
            books: vec![],
            book: None,
            page: 0,
            controls,
            headings: BTreeMap::new(),
            draft: vec![],
            line_edit: None,
            editing: None,
            version,
            note: String::new(),
        };
        state.set_font();
        fill_combo(
            state.c(ID_Q_KIND),
            &[(0, "支出".into()), (1, "収入".into()), (2, "振替".into())],
            Some(0),
        );
        fill_combo(
            state.c(ID_R_KIND),
            &sheets::REPORTS
                .iter()
                .enumerate()
                .map(|(i, r)| (i as i64, r.to_string()))
                .collect::<Vec<_>>(),
            Some(0),
        );
        for id in [ID_Q_DATE, ID_E_DATE, ID_R_TO] {
            set_date(state.c(id), Some(today()));
        }
        for id in [ID_J_FROM, ID_J_TO] {
            set_date(state.c(id), None);
        }
        send(
            state.c(ID_Q_AMOUNT),
            EM_SETCUEBANNER,
            1,
            wide("例: 1,980").as_ptr() as isize,
        );
        send(
            state.c(ID_Q_SEARCH),
            EM_SETCUEBANNER,
            1,
            wide("例: ガソリン、打合せのコーヒー").as_ptr() as isize,
        );
        send(
            state.c(ID_Q_RATIO),
            EM_SETCUEBANNER,
            1,
            wide("100").as_ptr() as isize,
        );
        send(
            state.c(ID_J_QUERY),
            EM_SETCUEBANNER,
            1,
            wide("摘要・科目・取引先・金額").as_ptr() as isize,
        );
        state.reload_books()?;
        state.set_page(0);
        let empty = state.books.is_empty();
        APP.with(|a| *a.borrow_mut() = Some(state));
        SetTimer(Some(hwnd), TIMER, 1000, None);
        let _ = ShowWindow(
            hwnd,
            if smoke_dir.is_some() {
                SW_SHOWNOACTIVATE
            } else {
                SW_SHOW
            },
        );
        with(|s| s.layout());
        if let Some(directory) = &smoke_dir {
            smoke::run(hwnd, directory)?;
            DestroyWindow(hwnd)?;
        } else if empty && !read_only {
            new_book(hwnd);
        }
        let accelerators = accelerators()?;
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if TranslateAcceleratorW(hwnd, accelerators, &msg) != 0 {
                continue;
            }
            if IsDialogMessageW(hwnd, &msg).as_bool() {
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = DestroyAcceleratorTable(accelerators);
        APP.with(|a| {
            if let Some(s) = a.borrow_mut().take() {
                let _ = DeleteObject(s.font.into());
            }
        });
        Ok(())
    }
}
