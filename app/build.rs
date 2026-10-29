//! Windows icon and manifest embedding for `yyaccountantw`, adapted from yyeditor
//! (see NOTICE.md). The icon is drawn here from the same shapes as the Web favicon,
//! so no binary image is kept in the repository.

use std::path::{Path, PathBuf};

/// リソースの種類
const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
/// アイコンのリソース ID（`native.rs` の `LoadIconW` と同じ）
const APP_ICON_ID: u16 = 1;
const SIZES: [u32; 5] = [16, 24, 32, 48, 256];

fn main() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("res/yyaccountantw.manifest");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rerun-if-changed=build.rs");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os != "windows" {
        return;
    }
    let res = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("yyaccountantw.res");
    std::fs::write(&res, icon_res()).expect("write .res");
    if target_env == "msvc" {
        // 両方の実行ファイルにコモンコントロール v6 と DPI 対応を宣言する
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
            manifest.display()
        );
        // link.exe は .res をそのまま受け付ける
        println!("cargo:rustc-link-arg-bins={}", res.display());
    } else {
        // GNU ld は .res を読めないので、windres で COFF のオブジェクトにする
        let obj = res.with_extension("o");
        match to_coff(&res, &obj) {
            Ok(()) => println!("cargo:rustc-link-arg-bins={}", obj.display()),
            Err(e) => println!("cargo:warning=アイコンを埋め込めません（windres: {e}）"),
        }
    }
}

/// .res を windres（環境変数 `WINDRES` か、MinGW の windres）で COFF のオブジェクトにする。
fn to_coff(res: &Path, obj: &Path) -> Result<(), String> {
    println!("cargo:rerun-if-env-changed=WINDRES");
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let candidates = match std::env::var("WINDRES") {
        Ok(w) => vec![w],
        Err(_) => vec![format!("{arch}-w64-mingw32-windres"), "windres".into()],
    };
    let mut last = String::new();
    for w in candidates {
        let status = std::process::Command::new(&w)
            .args(["-J", "res", "-O", "coff", "-i"])
            .arg(res)
            .arg("-o")
            .arg(obj)
            .status();
        match status {
            Ok(s) if s.success() => return Ok(()),
            Ok(s) => last = format!("{w}: {s}"),
            Err(e) => last = format!("{w}: {e}"),
        }
    }
    Err(last)
}

/// 64×64 の座標系での被覆。角丸の緑の四角に白い棒グラフ（Web の favicon と同じ形）。
fn shade(x: f32, y: f32) -> Option<[u8; 3]> {
    let r = 14.0;
    let cx = x.clamp(r, 64.0 - r);
    let cy = y.clamp(r, 64.0 - r);
    if !(0.0..64.0).contains(&x) || !(0.0..64.0).contains(&y) {
        return None;
    }
    if (x - cx).powi(2) + (y - cy).powi(2) > r * r {
        return None;
    }
    let bars = [
        (14.0, 46.0, 50.0, 46.0),
        (20.0, 40.0, 20.0, 28.0),
        (32.0, 40.0, 32.0, 18.0),
        (44.0, 40.0, 44.0, 24.0),
    ];
    let white = bars.iter().any(|&(x1, y1, x2, y2)| {
        let (dx, dy) = (x2 - x1, y2 - y1);
        let t = (((x - x1) * dx + (y - y1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        (x - x1 - t * dx).powi(2) + (y - y1 - t * dy).powi(2) <= 9.0
    });
    Some(if white {
        [0xff, 0xff, 0xff]
    } else {
        [0x5c, 0x6f, 0x1f]
    })
}

/// 4×4 の超標本化で描いた 32 ビット BMP（ICO 内の形式: 高さ 2 倍、下から上、AND マスク付き）。
fn bitmap(size: u32) -> Vec<u8> {
    let mut out = Vec::new();
    for v in [40u32, size, size * 2] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    let scale = 64.0 / size as f32;
    for row in (0..size).rev() {
        for col in 0..size {
            let (mut sum, mut hits) = ([0u32; 3], 0u32);
            for sy in 0..4 {
                for sx in 0..4 {
                    let x = (col as f32 + (sx as f32 + 0.5) / 4.0) * scale;
                    let y = (row as f32 + (sy as f32 + 0.5) / 4.0) * scale;
                    if let Some(c) = shade(x, y) {
                        hits += 1;
                        for i in 0..3 {
                            sum[i] += c[i] as u32;
                        }
                    }
                }
            }
            let avg = |i: usize| sum[i].checked_div(hits).unwrap_or(0) as u8;
            out.extend_from_slice(&[avg(0), avg(1), avg(2), (hits * 255 / 16) as u8]);
        }
    }
    let mask_row = size.div_ceil(32) * 4;
    out.resize(out.len() + (mask_row * size) as usize, 0);
    out
}

fn icon_res() -> Vec<u8> {
    // 先頭は空のリソース（.res ファイルの目印）
    let mut out = Vec::new();
    push_resource(&mut out, 0, 0, 0, &[]);
    let mut group = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(SIZES.len() as u16).to_le_bytes());
    for (i, size) in SIZES.into_iter().enumerate() {
        let image = bitmap(size);
        let id = (i + 1) as u16;
        push_resource(&mut out, RT_ICON, id, 0x1010, &image);
        // GRPICONDIRENTRY: 幅・高さ（256 は 0）、色数、予約、面、ビット数、大きさ、ID
        let side = if size >= 256 { 0 } else { size as u8 };
        group.extend_from_slice(&[side, side, 0, 0]);
        group.extend_from_slice(&1u16.to_le_bytes());
        group.extend_from_slice(&32u16.to_le_bytes());
        group.extend_from_slice(&(image.len() as u32).to_le_bytes());
        group.extend_from_slice(&id.to_le_bytes());
    }
    push_resource(&mut out, RT_GROUP_ICON, APP_ICON_ID, 0x1030, &group);
    out
}

/// 種類・名前が番号のリソースを 1 つ書く。
fn push_resource(out: &mut Vec<u8>, kind: u16, name: u16, flags: u16, data: &[u8]) {
    let header_size = 32u32;
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&header_size.to_le_bytes());
    for v in [0xFFFF, kind, 0xFFFF, name] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
    out.extend_from_slice(&flags.to_le_bytes()); // MemoryFlags
    // 言語: 日本語。空のリソースは 0
    let lang: u16 = if kind == 0 { 0 } else { 0x0411 };
    out.extend_from_slice(&lang.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // Version
    out.extend_from_slice(&0u32.to_le_bytes()); // Characteristics
    out.extend_from_slice(data);
    while out.len() & 3 != 0 {
        out.push(0);
    }
}
