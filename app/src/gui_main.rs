#![cfg_attr(windows, windows_subsystem = "windows")]
//! `yyaccountantw`: the native Windows UI. Without a console, errors are shown in a
//! message box (or written to `error.txt` during `--ui-smoke-test`).
#[cfg(windows)]
mod demo;
#[cfg(windows)]
mod native;
fn main() {
    #[cfg(windows)]
    if let Err(error) = native::run() {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        if args.first().is_some_and(|a| a == "--ui-smoke-test") {
            if let Some(path) = args.get(1) {
                let _ = std::fs::create_dir_all(path);
                let _ = std::fs::write(
                    std::path::Path::new(path).join("error.txt"),
                    error.to_string(),
                );
            }
        } else {
            unsafe {
                windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
                    None,
                    &windows::core::HSTRING::from(error.to_string()),
                    windows::core::w!("YYaccountant"),
                    windows::Win32::UI::WindowsAndMessaging::MB_OK
                        | windows::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
                );
            }
        }
        std::process::exit(1);
    }
    #[cfg(not(windows))]
    {
        eprintln!(
            "yyaccountantw requires Windows; use yyaccountant --web DATABASE or --mcp DATABASE on Linux."
        );
        std::process::exit(2);
    }
}
