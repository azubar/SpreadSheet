//! Служебный пример для разработки: конвертирует все mxl-files/*.mxl
//! в JPEG и PNG (в каталог из 1-го аргумента, по умолчанию target/render).
//! Не входит в состав компоненты.

use std::path::PathBuf;

use spreadsheet::converter::{convert, ImageKind};
use spreadsheet::mxl_parser::parse_file;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let out_dir = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target/render"));
    std::fs::create_dir_all(&out_dir).unwrap();
    let scale: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(100);

    let mut ok = 0;
    let mut failed = 0;
    let mut entries: Vec<_> = std::fs::read_dir(root.join("mxl-files"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("mxl"))
        .collect();
    entries.sort();
    for path in entries {
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        let result = parse_file(&path)
            .map_err(|e| e.to_string())
            .and_then(|doc| {
                convert(
                    &doc,
                    &out_dir.join(format!("{stem}.jpg")),
                    &ImageKind::Jpeg { quality: 90 },
                    scale,
                )?;
                convert(
                    &doc,
                    &out_dir.join(format!("{stem}.png")),
                    &ImageKind::Png,
                    scale,
                )
            });
        match result {
            Ok(()) => {
                ok += 1;
                println!("OK   {stem}");
            }
            Err(e) => {
                failed += 1;
                println!("FAIL {stem}: {e}");
            }
        }
    }
    println!("done: {ok} ok, {failed} failed");
    std::process::exit(if failed > 0 { 1 } else { 0 });
}
