//! Регрессионные тесты рендера по эталонным картинкам.
//!
//! Каждый образец из `mxl-demo-collection` рендерится в масштабе 50% и
//! сравнивается с эталоном из `tests/golden/`. Допускаются отличия
//! сглаживания: не больше 0,1% пикселей с отличием канала больше 24.
//!
//! Обновление эталонов после намеренного изменения рендера:
//!
//! ```text
//! SPREADSHEET_UPDATE_GOLDEN=1 cargo test -p spreadsheet --test golden
//! ```
//!
//! Изменённые эталоны нужно просмотреть глазами перед коммитом.

use std::path::{Path, PathBuf};

use spreadsheet::converter::{render, render_page, PrintJob};
use spreadsheet::mxl_parser::parse_file;

const SCALE: u32 = 50;
/// Отличие канала, начиная с которого пиксель считается другим
const CHANNEL_TOLERANCE: u8 = 24;
/// Доля отличающихся пикселей, допустимая без ошибки
const MAX_DIFF_RATIO: f64 = 0.001;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn update_requested() -> bool {
    std::env::var_os("SPREADSHEET_UPDATE_GOLDEN").is_some()
}

/// Сравнивает картинку с эталоном `name` (или записывает эталон).
fn check(name: &str, pixmap: &tiny_skia::Pixmap) -> Result<(), String> {
    let rgb = image::RgbImage::from_fn(pixmap.width(), pixmap.height(), |x, y| {
        let i = ((y * pixmap.width() + x) * 4) as usize;
        let d = pixmap.data();
        image::Rgb([d[i], d[i + 1], d[i + 2]])
    });
    let path = golden_dir().join(format!("{name}.png"));
    if update_requested() {
        rgb.save(&path).map_err(|e| format!("{name}: {e}"))?;
        return Ok(());
    }
    let expected = image::open(&path)
        .map_err(|e| {
            format!(
                "{name}: нет эталона {} ({e}) — создайте его: \
                 SPREADSHEET_UPDATE_GOLDEN=1 cargo test -p spreadsheet --test golden",
                path.display()
            )
        })?
        .to_rgb8();
    if expected.dimensions() != rgb.dimensions() {
        return Err(format!(
            "{name}: размер {:?}, эталон {:?}",
            rgb.dimensions(),
            expected.dimensions()
        ));
    }
    let differing = rgb
        .pixels()
        .zip(expected.pixels())
        .filter(|(a, b)| {
            a.0.iter()
                .zip(b.0.iter())
                .any(|(x, y)| x.abs_diff(*y) > CHANNEL_TOLERANCE)
        })
        .count();
    let ratio = differing as f64 / (rgb.width() as f64 * rgb.height() as f64);
    if ratio > MAX_DIFF_RATIO {
        return Err(format!(
            "{name}: отличается {differing} пикселей ({:.3}%) от эталона",
            ratio * 100.0
        ));
    }
    Ok(())
}

fn mxl_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "mxl"))
        .collect();
    files.sort();
    files
}

fn stem(path: &Path) -> String {
    path.file_stem().unwrap().to_string_lossy().into_owned()
}

#[test]
fn demo_collection_matches_golden_images() {
    let mut errors = Vec::new();
    for path in mxl_files(&root().join("mxl-demo-collection")) {
        let doc = parse_file(&path).unwrap();
        let pixmap = render(&doc, SCALE).unwrap();
        if let Err(e) = check(&stem(&path), &pixmap) {
            errors.push(e);
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn printing_demo_pages_match_golden_images() {
    let path = root().join("mxl-demo-collection/ДемонстрацияПечатиРазрывов_A4_Книжная.mxl");
    let doc = parse_file(&path).unwrap();
    let mut errors = Vec::new();
    for page in 1..=3 {
        let pixmap = render_page(&doc, SCALE, page).unwrap();
        if let Err(e) = check(&format!("{}_стр{page}", stem(&path)), &pixmap) {
            errors.push(e);
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

/// Варианты печатного демо из `mxl-demo-collection/generated` (создаются
/// скриптом `tools/mxl_variants.py`) отличаются только параметрами печати.
/// В режиме «экран» параметры печати на картинку не влияют.
#[test]
fn print_settings_do_not_change_screen_render() {
    let source =
        parse_file(&root().join("mxl-demo-collection/ДемонстрацияПечатиРазрывов_A4_Книжная.mxl"))
            .unwrap();
    let expected = render(&source, SCALE).unwrap();
    for path in mxl_files(&root().join("mxl-demo-collection/generated")) {
        let name = stem(&path);
        if !name.starts_with("Печать_") {
            continue;
        }
        let doc = parse_file(&path).unwrap();
        let pixmap = render(&doc, SCALE).unwrap();
        assert!(pixmap.data() == expected.data(), "{name}");
    }
}

#[test]
fn long_generated_table_renders() {
    let path = root().join("mxl-demo-collection/generated/ДлиннаяТаблица_БезРазрывов.mxl");
    let doc = parse_file(&path).unwrap();
    assert_eq!(doc.rows.iter().map(|r| r.index).max(), Some(348));
    assert_eq!(doc.print_area, None);
    assert!(doc.row_breaks().is_empty());
    let pixmap = render(&doc, SCALE).unwrap();
    assert!(pixmap.height() > 5_000, "{}", pixmap.height());
}

/// Листы режима «как при печати» (50 DPI): A4 книжная с ч/б печатью и A5.
#[test]
fn print_sheets_match_golden_images() {
    let cases = [
        (
            "mxl-demo-collection/ДемонстрацияПечатиРазрывов_A4_Книжная.mxl",
            1,
        ),
        (
            "mxl-demo-collection/ДемонстрацияПечатиРазрывов_A4_Книжная.mxl",
            2,
        ),
        ("mxl-demo-collection/generated/Печать_A5_Книжная.mxl", 2),
        (
            "mxl-demo-collection/generated/Печать_A4_Альбомная_50.mxl",
            1,
        ),
    ];
    let mut errors = Vec::new();
    for (path, page) in cases {
        let path = root().join(path);
        let doc = parse_file(&path).unwrap();
        let sheet = PrintJob::new(&doc, 50).unwrap().render(&doc, page).unwrap();
        if let Err(e) = check(&format!("печать_{}_лист{page}", stem(&path)), &sheet) {
            errors.push(e);
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}
