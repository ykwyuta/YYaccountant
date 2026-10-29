//! Small modal form (text fields and choices) used for book creation and settings.
//! The owner is disabled while it is open; Tab, Enter and Esc work as in a dialog.
use super::*;

pub(super) enum Field {
    Text(&'static str, String),
    /// Label, options and the selected index. The value returned is the index.
    Choice(&'static str, Vec<String>, usize),
}
impl Field {
    fn label(&self) -> &'static str {
        match self {
            Self::Text(l, _) | Self::Choice(l, _, _) => l,
        }
    }
}
struct Form {
    inputs: Vec<(HWND, bool)>,
    labels: Vec<&'static str>,
    done: bool,
    accepted: Option<Vec<String>>,
}
thread_local! {static FORM:RefCell<Option<Form>>=const{RefCell::new(None)};}
// Values the smoke test types into the next form, by label (choices by option text).
thread_local! {pub(super) static SMOKE_VALUES:RefCell<BTreeMap<String,String>>=const{RefCell::new(BTreeMap::new())};}
const AUTO_FILL: u32 = WM_APP + 1;
const FIELD: u16 = 100;

pub(super) fn register(instance: HINSTANCE, icon: HICON) -> Result<()> {
    unsafe {
        if RegisterClassW(&WNDCLASSW {
            hInstance: instance,
            lpszClassName: w!("YYAccountantForm"),
            lpfnWndProc: Some(proc),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hIcon: icon,
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            ..Default::default()
        }) == 0
        {
            return Err("Cannot register the form window class".into());
        }
    }
    Ok(())
}

fn collect(f: &Form) -> Vec<String> {
    f.inputs
        .iter()
        .map(|(h, choice)| {
            if *choice {
                combo_index(*h).unwrap_or(0).to_string()
            } else {
                text(*h)
            }
        })
        .collect()
}

extern "system" fn proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        match m {
            WM_COMMAND => {
                let id = (w.0 & 0xffff) as i32;
                if id == IDOK.0 || id == IDCANCEL.0 {
                    FORM.with(|f| {
                        if let Some(f) = f.borrow_mut().as_mut() {
                            if id == IDOK.0 {
                                f.accepted = Some(collect(f));
                            }
                            f.done = true;
                        }
                    });
                    return LRESULT(0);
                }
            }
            WM_CLOSE => {
                FORM.with(|f| {
                    if let Some(f) = f.borrow_mut().as_mut() {
                        f.done = true;
                    }
                });
                return LRESULT(0);
            }
            AUTO_FILL if SMOKE.with(|s| s.get()) => {
                FORM.with(|f| {
                    let f = f.borrow();
                    let Some(f) = f.as_ref() else {
                        return;
                    };
                    SMOKE_VALUES.with(|values| {
                        for ((input, choice), label) in f.inputs.iter().zip(&f.labels) {
                            let Some(value) = values.borrow().get(*label).cloned() else {
                                continue;
                            };
                            if *choice {
                                let wanted = wide(&value);
                                let index = send(
                                    *input,
                                    CB_FINDSTRINGEXACT,
                                    usize::MAX,
                                    wanted.as_ptr() as isize,
                                );
                                send(*input, CB_SETCURSEL, index.0.max(0) as usize, 0);
                            } else {
                                set_text(*input, &value);
                            }
                        }
                        values.borrow_mut().clear();
                    });
                });
                let _ = PostMessageW(Some(h), WM_COMMAND, WPARAM(IDOK.0 as usize), LPARAM(0));
                return LRESULT(0);
            }
            _ => {}
        }
        DefWindowProcW(h, m, w, l)
    }
}

/// Shows the form and returns the values (text, or the index of a choice) on OK.
pub(super) fn show(owner: HWND, title: &str, fields: Vec<Field>) -> Option<Vec<String>> {
    unsafe {
        let dpi = GetDpiForWindow(owner).max(96) as i32;
        let px = |v: i32| scale(dpi, v);
        let instance: HINSTANCE = GetModuleHandleW(None).ok()?.into();
        let (label_w, field_w, row, m) = (px(150), px(260), px(36), px(16));
        let width = m * 2 + label_w + field_w;
        let height = m * 2 + row * fields.len() as i32 + px(44);
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        let style = WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN;
        let ex = WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT;
        let _ = AdjustWindowRectExForDpi(&mut frame, style, false, ex, dpi as u32);
        let mut owner_rect = RECT::default();
        let _ = GetWindowRect(owner, &mut owner_rect);
        let (fw, fh) = (frame.right - frame.left, frame.bottom - frame.top);
        let h = CreateWindowExW(
            ex,
            w!("YYAccountantForm"),
            &HSTRING::from(title),
            style,
            owner_rect.left + ((owner_rect.right - owner_rect.left - fw) / 2).max(0),
            owner_rect.top + ((owner_rect.bottom - owner_rect.top - fh) / 3).max(0),
            fw,
            fh,
            Some(owner),
            None,
            Some(instance),
            None,
        )
        .ok()?;
        let font = window_font(dpi);
        let child =
            |class: PCWSTR, text: &str, style: u32, ex: WINDOW_EX_STYLE, id: i32, r: RECT| {
                let c = CreateWindowExW(
                    ex,
                    class,
                    &HSTRING::from(text),
                    WS_CHILD | WS_VISIBLE | WINDOW_STYLE(style),
                    r.left,
                    r.top,
                    r.right,
                    r.bottom,
                    Some(h),
                    Some(HMENU(id as usize as *mut _)),
                    Some(instance),
                    None,
                )
                .unwrap_or_default();
                send(c, WM_SETFONT, font.0 as usize, 0);
                c
            };
        let rect = |x, y, w, hh| RECT {
            left: x,
            top: y,
            right: w,
            bottom: hh,
        };
        let mut inputs = vec![];
        let mut labels = vec![];
        for (i, field) in fields.iter().enumerate() {
            let y = m + row * i as i32;
            child(
                w!("STATIC"),
                field.label(),
                STATIC_STYLE,
                WINDOW_EX_STYLE(0),
                -1,
                rect(m, y, label_w, px(26)),
            );
            let id = (FIELD + i as u16) as i32;
            let input = match field {
                Field::Text(_, value) => child(
                    w!("EDIT"),
                    value,
                    ES_AUTOHSCROLL as u32 | WS_TABSTOP.0,
                    WS_EX_CLIENTEDGE,
                    id,
                    rect(m + label_w, y, field_w, px(26)),
                ),
                Field::Choice(_, options, selected) => {
                    let c = child(
                        w!("COMBOBOX"),
                        "",
                        CBS_DROPDOWNLIST as u32 | WS_VSCROLL.0 | WS_TABSTOP.0,
                        WINDOW_EX_STYLE(0),
                        id,
                        rect(m + label_w, y, field_w, px(300)),
                    );
                    let items: Vec<(i64, String)> = options
                        .iter()
                        .enumerate()
                        .map(|(i, o)| (i as i64, o.clone()))
                        .collect();
                    fill_combo(c, &items, Some(*selected as i64));
                    c
                }
            };
            inputs.push((input, matches!(field, Field::Choice(..))));
            labels.push(field.label());
        }
        let y = height - m - px(32);
        child(
            w!("BUTTON"),
            "OK",
            (BS_DEFPUSHBUTTON as u32) | WS_TABSTOP.0,
            WINDOW_EX_STYLE(0),
            IDOK.0,
            rect(width - m - px(220), y, px(104), px(32)),
        );
        child(
            w!("BUTTON"),
            "キャンセル",
            (BS_PUSHBUTTON as u32) | WS_TABSTOP.0,
            WINDOW_EX_STYLE(0),
            IDCANCEL.0,
            rect(width - m - px(108), y, px(108), px(32)),
        );
        FORM.with(|f| {
            *f.borrow_mut() = Some(Form {
                inputs: inputs.clone(),
                labels,
                done: false,
                accepted: None,
            })
        });
        let _ = EnableWindow(owner, false);
        let smoke = SMOKE.with(|s| s.get());
        let _ = ShowWindow(h, if smoke { SW_SHOWNOACTIVATE } else { SW_SHOW });
        if let Some((first, _)) = inputs.first() {
            let _ = SetFocus(Some(*first));
        }
        if smoke {
            let _ = PostMessageW(Some(h), AUTO_FILL, WPARAM(0), LPARAM(0));
        }
        let mut msg = MSG::default();
        while !FORM.with(|f| f.borrow().as_ref().is_none_or(|f| f.done)) {
            let result = GetMessageW(&mut msg, None, 0, 0).0;
            if result <= 0 {
                if result == 0 {
                    PostQuitMessage(msg.wParam.0 as i32);
                }
                break;
            }
            if IsDialogMessageW(h, &msg).as_bool() {
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = EnableWindow(owner, true);
        let _ = DestroyWindow(h);
        let _ = DeleteObject(font.into());
        if !smoke {
            let _ = SetForegroundWindow(owner);
        }
        FORM.with(|f| f.borrow_mut().take())
            .and_then(|f| f.accepted)
    }
}
