use spreadsheet::converter::{convert, ImageKind};
use spreadsheet::mxl_parser::parse_file;
use std::path::Path;

fn main() {
    let input_dir = Path::new("D:\\Dev1C\\SpreadSheet\\Данные");
    let out_dir = Path::new("D:\\Work\\SpreadSheet\\tmpimg");
    std::fs::create_dir_all(out_dir).unwrap();

    let mut ok = 0;
    let mut failed = 0;

    let mut entries: Vec<_> = std::fs::read_dir(input_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("mxl"))
        .collect();
    entries.sort();

    println!("Checking MXL files in {}", input_dir.display());
    println!("Output images will be saved to {}", out_dir.display());
    println!("--------------------------------------------------");

    for path in entries {
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        println!("Processing: {}", stem);

        // Parse step
        let parse_result = parse_file(&path);
        let doc = match parse_result {
            Ok(doc) => {
                println!("  [OK] Parsed successfully. Rows: {}, Formats: {}, Fonts: {}, Merges: {}, Drawings: {}",
                    doc.rows.len(),
                    doc.formats.len(),
                    doc.fonts.len(),
                    doc.merges.len(),
                    doc.drawings.len()
                );
                doc
            }
            Err(e) => {
                failed += 1;
                println!("  [FAIL] Parsing error: {}", e);
                continue;
            }
        };

        // Convert JPG
        let jpg_path = out_dir.join(format!("{}.jpg", stem));
        let jpg_res = convert(&doc, &jpg_path, &ImageKind::Jpeg { quality: 90 }, 100);
        match jpg_res {
            Ok(()) => {
                println!("  [OK] Converted to JPG: {}", jpg_path.display());
            }
            Err(e) => {
                failed += 1;
                println!("  [FAIL] JPG conversion error: {}", e);
                continue;
            }
        }

        // Convert PNG
        let png_path = out_dir.join(format!("{}.png", stem));
        let png_res = convert(&doc, &png_path, &ImageKind::Png, 100);
        match png_res {
            Ok(()) => {
                ok += 1;
                println!("  [OK] Converted to PNG: {}", png_path.display());
            }
            Err(e) => {
                failed += 1;
                println!("  [FAIL] PNG conversion error: {}", e);
            }
        }
    }

    println!("--------------------------------------------------");
    println!(
        "Done: {} parsed & converted successfully, {} failed",
        ok, failed
    );
}
