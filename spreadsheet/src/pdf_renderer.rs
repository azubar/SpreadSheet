#[cfg(windows)]
pub use win_pdf::render_pdf;

#[cfg(not(windows))]
pub use nix_pdf::render_pdf;

/// Перевіряє, що файл структурно схожий на завершений PDF (є заголовок і кінцевий
/// маркер `%%EOF`). Обрізаний/пошкоджений файл (наприклад, недописаний під час
/// експорту з 1С) не містить жодних об'єктів чи таблиці xref, і системні PDF-рушії
/// (WinRT, poppler) відповідають на нього незрозумілим низькорівневим кодом помилки
/// (напр. `0x80048018`). Ця перевірка дає користувачу зрозуміле повідомлення замість
/// нього.
fn validate_pdf_bytes(bytes: &[u8]) -> Result<(), String> {
    if !bytes.starts_with(b"%PDF-") {
        return Err("файл не є PDF (відсутній заголовок %PDF-)".to_string());
    }
    if !bytes.windows(5).any(|w| w == b"%%EOF") {
        return Err(format!(
            "PDF файл пошкоджений або обрізаний: розмір {} байт, відсутній кінцевий маркер %%EOF. Файл, судячи з усього, не був повністю збережений або скопійований — перевірте джерело файлу (наприклад, повторіть експорт/друк у PDF з 1С).",
            bytes.len()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_pdf_bytes;

    #[test]
    fn rejects_truncated_pdf_stub() {
        let bytes = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n";
        let e = validate_pdf_bytes(bytes).unwrap_err();
        assert!(e.contains("%%EOF"), "{e}");
        assert!(e.contains("15"), "{e}");
    }

    #[test]
    fn rejects_non_pdf_bytes() {
        let e = validate_pdf_bytes(b"not a pdf").unwrap_err();
        assert!(e.contains("%PDF-"), "{e}");
    }

    #[test]
    fn accepts_bytes_with_header_and_eof_marker() {
        let bytes = b"%PDF-1.7\n...content...\n%%EOF";
        assert!(validate_pdf_bytes(bytes).is_ok());
    }
}

#[cfg(windows)]
mod win_pdf {
    use std::path::Path;
    use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
    use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    pub fn render_pdf(
        path: &Path,
        scale: u32,
        page_number: u32,
        base_dpi: u32,
        spacing: u32,
    ) -> Result<tiny_skia::Pixmap, String> {
        let path_buf = path.to_path_buf();
        std::thread::spawn(move || {
            // Initialize COM MTA for the current thread
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }

            let res = (|| {
                // Read file bytes
                let bytes = std::fs::read(&path_buf)
                    .map_err(|e| format!("Не вдалося прочитати PDF файл: {e}"))?;
                super::validate_pdf_bytes(&bytes)?;

                // Create InMemoryRandomAccessStream and write bytes to it
                let stream = InMemoryRandomAccessStream::new()
                    .map_err(|e| format!("Не вдалося створити WinRT потік: {e}"))?;
                let writer = DataWriter::CreateDataWriter(&stream)
                    .map_err(|e| format!("Не вдалося створити DataWriter: {e}"))?;
                writer.WriteBytes(&bytes)
                    .map_err(|e| format!("Помилка запису байтів у DataWriter: {e}"))?;
                writer.StoreAsync()
                    .map_err(|e| format!("Помилка збереження DataWriter: {e}"))?
                    .get()
                    .map_err(|e| format!("Помилка завершення запису в потік: {e}"))?;
                writer.FlushAsync()
                    .map_err(|e| format!("Помилка очищення DataWriter: {e}"))?
                    .get()
                    .map_err(|e| format!("Помилка флешу потоку: {e}"))?;

                // Load PDF Document
                let doc = PdfDocument::LoadFromStreamAsync(&stream)
                    .map_err(|e| format!("Помилка завантаження PDF документа: {e}"))?
                    .get()
                    .map_err(|e| format!("Помилка отримання PDF документа: {e}"))?;

                let page_count = doc.PageCount()
                    .map_err(|e| format!("Помилка отримання кількості сторінок: {e}"))?;
                if page_count == 0 {
                    return Err("PDF документ не містить сторінок".to_string());
                }

                // Determine which pages to render
                let pages_to_render: Vec<u32> = if page_number == 0 {
                    (0..page_count).collect()
                } else {
                    if page_number > page_count {
                        return Err(format!(
                            "Запитано сторінку {}, але документ містить лише {} сторінок",
                            page_number, page_count
                        ));
                    }
                    vec![page_number - 1]
                };

                // Calculate DPI (clamped to 50..600)
                let dpi_final = ((base_dpi * scale) as f32 / 100.0).round().clamp(50.0, 600.0);
                let dpi_scale = dpi_final / 96.0;

                let mut rendered_pages = Vec::with_capacity(pages_to_render.len());
                let mut max_w = 0u32;
                let mut total_h = 0u32;

                for (idx, &p_idx) in pages_to_render.iter().enumerate() {
                    let page = doc.GetPage(p_idx)
                        .map_err(|e| format!("Помилка отримання сторінки {}: {e}", p_idx + 1))?;

                    let size = page.Size()
                        .map_err(|e| format!("Помилка отримання розміру сторінки {}: {e}", p_idx + 1))?;

                    let dest_w = (size.Width * dpi_scale).round() as u32;
                    let dest_h = (size.Height * dpi_scale).round() as u32;

                    if dest_w == 0 || dest_h == 0 {
                        return Err(format!("Некоректний розмір сторінки {}: {}x{}", p_idx + 1, dest_w, dest_h));
                    }

                    let render_options = PdfPageRenderOptions::new()
                        .map_err(|e| format!("Не вдалося створити PdfPageRenderOptions: {e}"))?;
                    render_options.SetDestinationWidth(dest_w)
                        .map_err(|e| format!("Не вдалося встановити ширину рендерингу: {e}"))?;
                    render_options.SetDestinationHeight(dest_h)
                        .map_err(|e| format!("Не вдалося встановити висоту рендерингу: {e}"))?;

                    // BitmapEncoderId навмисно не встановлюється: RenderWithOptionsToStreamAsync
                    // без нього вже кодує PNG за замовчуванням, а явне встановлення цієї
                    // властивості (з будь-яким GUID) призводить до E_FAIL (0x80004005) —
                    // перевірено на реальних PDF-файлах 1С.
                    let page_stream = InMemoryRandomAccessStream::new()
                        .map_err(|e| format!("Не вдалося створити потік для сторінки: {e}"))?;

                    page.RenderWithOptionsToStreamAsync(&page_stream, &render_options)
                        .map_err(|e| format!("Помилка запуску рендерингу сторінки {}: {e}", p_idx + 1))?
                        .get()
                        .map_err(|e| format!("Помилка завершення рендерингу сторінки {}: {e}", p_idx + 1))?;

                    // Read bytes from page_stream
                    let size_u64 = page_stream.Size()
                        .map_err(|e| format!("Помилка отримання розміру потоку сторінки: {e}"))?;
                    let page_reader = windows::Storage::Streams::DataReader::CreateDataReader(&page_stream)
                        .map_err(|e| format!("Не вдалося створити DataReader для сторінки: {e}"))?;
                    page_reader.LoadAsync(size_u64 as u32)
                        .map_err(|e| format!("Помилка читання потоку сторінки: {e}"))?
                        .get()
                        .map_err(|e| format!("Помилка завантаження даних сторінки: {e}"))?;

                    let mut page_bytes = vec![0u8; size_u64 as usize];
                    page_reader.ReadBytes(&mut page_bytes)
                        .map_err(|e| format!("Помилка зчитування байтів сторінки: {e}"))?;

                    // Decode PNG bytes into Pixmap
                    let img = image::load_from_memory_with_format(&page_bytes, image::ImageFormat::Png)
                        .map_err(|e| format!("Помилка декодування зображення сторінки {}: {e}", p_idx + 1))?;

                    let rgba = img.to_rgba8();
                    let w = rgba.width();
                    let h = rgba.height();

                    let mut page_pixmap = tiny_skia::Pixmap::new(w, h)
                        .ok_or_else(|| "Не вдалося створити Pixmap для сторінки".to_string())?;
                    page_pixmap.data_mut().copy_from_slice(rgba.as_raw());

                    max_w = max_w.max(w);
                    total_h += h;
                    if idx > 0 {
                        total_h += spacing;
                    }

                    rendered_pages.push(page_pixmap);
                }

                // Respect MAX_DIMENSION (65_500)
                let max_dimension = 65_500;
                if max_w > max_dimension || total_h > max_dimension {
                    return Err(format!(
                        "Розмір результуючого зображення PDF ({}x{}) перевищує максимальний ліміт {}x{}. Зменшіть Масштаб або Роздільну здатність.",
                        max_w, total_h, max_dimension, max_dimension
                    ));
                }

                // Assemble pages into one Pixmap
                let mut final_pixmap = tiny_skia::Pixmap::new(max_w, total_h)
                    .ok_or_else(|| "Не вдалося створити підсумковий Pixmap".to_string())?;
                final_pixmap.fill(tiny_skia::Color::WHITE);

                let mut current_y = 0u32;
                for page_pixmap in rendered_pages {
                    let pw = page_pixmap.width();
                    let ph = page_pixmap.height();

                    // Center alignment:
                    let x_offset = (max_w - pw) / 2;

                    // Copy pixels row by row
                    let src_data = page_pixmap.data();
                    let dest_data = final_pixmap.data_mut();

                    for row in 0..ph {
                        let src_start = (row * pw * 4) as usize;
                        let src_end = src_start + (pw * 4) as usize;

                        let dest_row = current_y + row;
                        let dest_start = ((dest_row * max_w + x_offset) * 4) as usize;
                        let dest_end = dest_start + (pw * 4) as usize;

                        dest_data[dest_start..dest_end].copy_from_slice(&src_data[src_start..src_end]);
                    }

                    current_y += ph + spacing;
                }

                Ok(final_pixmap)
            })();

            unsafe {
                windows::Win32::System::Com::CoUninitialize();
            }
            res
        })
        .join()
        .map_err(|_| "Помилка потоку при рендерингу PDF".to_string())?
    }
}

#[cfg(not(windows))]
mod nix_pdf {
    use std::path::Path;
    use std::process::Command;

    pub fn render_pdf(
        path: &Path,
        scale: u32,
        page_number: u32,
        base_dpi: u32,
        spacing: u32,
    ) -> Result<tiny_skia::Pixmap, String> {
        // 1. Check if pdftoppm is available
        let pdftoppm_check = Command::new("pdftoppm").arg("-v").output();
        if pdftoppm_check.is_err() {
            return Err("Утиліта 'pdftoppm' не знайдена в системному шляху PATH. Переконайтеся, що встановлено пакет poppler-utils.".to_string());
        }

        // 1b. Sanity-check the PDF structure before invoking pdftoppm
        let bytes =
            std::fs::read(path).map_err(|e| format!("Не вдалося прочитати PDF файл: {e}"))?;
        super::validate_pdf_bytes(&bytes)?;

        // 2. Create a unique temporary directory
        let temp_dir = tempfile::Builder::new()
            .prefix("spreadsheet_pdf_")
            .tempdir()
            .map_err(|e| format!("Не вдалося створити тимчасовий каталог: {e}"))?;

        let dpi_final = ((base_dpi * scale) as f32 / 100.0)
            .round()
            .clamp(50.0, 600.0) as u32;

        // 3. Prepare pdftoppm execution
        let mut cmd = Command::new("pdftoppm");
        cmd.arg("-png").arg("-r").arg(dpi_final.to_string());

        if page_number > 0 {
            cmd.arg("-f")
                .arg(page_number.to_string())
                .arg("-l")
                .arg(page_number.to_string());
        }

        let prefix = temp_dir.path().join("page");
        cmd.arg(path).arg(&prefix);

        let output = cmd
            .output()
            .map_err(|e| format!("Помилка при запуску pdftoppm: {e}"))?;
        if !output.status.success() {
            let err_msg = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Помилка виконання pdftoppm: {}", err_msg.trim()));
        }

        // 4. Scan temporary directory for generated files
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(temp_dir.path())
            .map_err(|e| format!("Не вдалося прочитати тимчасовий каталог: {e}"))?
        {
            let entry = entry.map_err(|e| format!("Помилка запису в каталозі: {e}"))?;
            let file_name = entry.file_name();
            let name_str = file_name.to_string_lossy();
            if name_str.starts_with("page-") && name_str.ends_with(".png") {
                // Extract number between "page-" and ".png"
                let num_str = &name_str["page-".len()..name_str.len() - ".png".len()];
                if let Ok(num) = num_str.parse::<u32>() {
                    entries.push((num, entry.path()));
                }
            }
        }

        if entries.is_empty() {
            return Err("Рендеринг PDF не згенерував жодного файлу".to_string());
        }

        // Sort by page number ascending
        entries.sort_by_key(|k| k.0);

        let mut rendered_pages = Vec::with_capacity(entries.len());
        let mut max_w = 0u32;
        let mut total_h = 0u32;

        for (idx, (_, file_path)) in entries.iter().enumerate() {
            let img = image::open(file_path)
                .map_err(|e| format!("Помилка читання тимчасового зображення сторінки: {e}"))?;
            let rgba = img.to_rgba8();
            let w = rgba.width();
            let h = rgba.height();

            let mut page_pixmap = tiny_skia::Pixmap::new(w, h)
                .ok_or_else(|| "Не вдалося створити Pixmap для сторінки".to_string())?;
            page_pixmap.data_mut().copy_from_slice(rgba.as_raw());

            max_w = max_w.max(w);
            total_h += h;
            if idx > 0 {
                total_h += spacing;
            }
            rendered_pages.push(page_pixmap);
        }

        // Respect MAX_DIMENSION (65_500)
        let max_dimension = 65_500;
        if max_w > max_dimension || total_h > max_dimension {
            return Err(format!(
                "Розмір результуючого зображення PDF ({}x{}) перевищує максимальний ліміт {}x{}. Зменшіть Масштаб або Роздільну здатність.",
                max_w, total_h, max_dimension, max_dimension
            ));
        }

        // Assemble pages into one Pixmap
        let mut final_pixmap = tiny_skia::Pixmap::new(max_w, total_h)
            .ok_or_else(|| "Не вдалося створити підсумковий Pixmap".to_string())?;
        final_pixmap.fill(tiny_skia::Color::WHITE);

        let mut current_y = 0u32;
        for page_pixmap in rendered_pages {
            let pw = page_pixmap.width();
            let ph = page_pixmap.height();

            // Center alignment:
            let x_offset = (max_w - pw) / 2;

            // Copy pixels row by row
            let src_data = page_pixmap.data();
            let dest_data = final_pixmap.data_mut();

            for row in 0..ph {
                let src_start = (row * pw * 4) as usize;
                let src_end = src_start + (pw * 4) as usize;

                let dest_row = current_y + row;
                let dest_start = ((dest_row * max_w + x_offset) * 4) as usize;
                let dest_end = dest_start + (pw * 4) as usize;

                dest_data[dest_start..dest_end].copy_from_slice(&src_data[src_start..src_end]);
            }

            current_y += ph + spacing;
        }

        Ok(final_pixmap)
    }
}
