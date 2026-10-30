//! Растеризация PDF средствами ОС: WinRT `Windows.Data.Pdf` на Windows,
//! `pdftoppm` (poppler-utils) на Linux. Страницы склеиваются вертикально
//! с выравниванием по центру.

#[cfg(windows)]
pub use win_pdf::{page_count, render_pdf};

#[cfg(not(windows))]
pub use nix_pdf::{page_count, render_pdf};

use std::path::Path;

/// Источник PDF: файл на диске или данные в памяти (ДвоичныеДанные 1С).
#[derive(Clone, Copy)]
pub enum PdfInput<'a> {
    File(&'a Path),
    Bytes(&'a [u8]),
}

impl PdfInput<'_> {
    /// Содержимое PDF, проверенное [`validate_pdf_bytes`].
    fn read_validated(&self) -> Result<std::borrow::Cow<'_, [u8]>, String> {
        let bytes = match self {
            PdfInput::File(path) => std::borrow::Cow::Owned(
                std::fs::read(path).map_err(|e| format!("не удалось прочитать PDF-файл: {e}"))?,
            ),
            PdfInput::Bytes(bytes) => std::borrow::Cow::Borrowed(*bytes),
        };
        validate_pdf_bytes(&bytes)?;
        Ok(bytes)
    }
}

use crate::converter::{check_image_size, copy_pixmap, white_pixmap};

/// Подсказка в сообщениях о слишком большой картинке PDF.
const PDF_SIZE_HINT: &str =
    "уменьшите Масштаб или РазрешениеPDF либо выберите одну страницу (НомерСтраницы)";

/// Итоговое разрешение растеризации: «РазрешениеPDF» × «Масштаб». Масштаб
/// применяется без ограничений — размер результата контролируется пределами
/// картинки (см. [`check_stacked_size`]), которые проверяются до рендера.
pub(crate) fn effective_dpi(base_dpi: u32, scale: u32) -> f32 {
    base_dpi as f32 * scale as f32 / 100.0
}

/// Проверяет, что файл структурно схожий на завершённый PDF (есть заголовок и
/// конечный маркер `%%EOF`). Обрезанный/повреждённый файл (например,
/// недописанный при экспорте из 1С) не содержит ни объектов, ни таблицы xref,
/// и системные PDF-движки (WinRT, poppler) отвечают на него невнятным
/// низкоуровневым кодом ошибки (напр. `0x80048018`). Эта проверка даёт
/// пользователю понятное сообщение вместо него.
fn validate_pdf_bytes(bytes: &[u8]) -> Result<(), String> {
    if !bytes.starts_with(b"%PDF-") {
        return Err("файл не является PDF (нет заголовка %PDF-)".to_string());
    }
    if !bytes.windows(5).any(|w| w == b"%%EOF") {
        return Err(format!(
            "PDF-файл повреждён или обрезан: размер {} байт, нет конечного маркера %%EOF. \
             Похоже, файл не был полностью сохранён или скопирован — проверьте источник \
             файла (например, повторите экспорт/печать в PDF из 1С).",
            bytes.len()
        ));
    }
    Ok(())
}

/// Проверяет размер картинки из страниц `sizes` (ширина, высота в px),
/// склеенных вертикально с интервалом `spacing`. Вызывается до выделения
/// памяти: на Windows — до рендера страниц, на Linux — по оценке `pdfinfo`.
pub(crate) fn check_stacked_size(sizes: &[(u64, u64)], spacing: u32) -> Result<(), String> {
    let max_w = sizes.iter().map(|s| s.0).max().unwrap_or(0);
    let total_h = sizes.iter().map(|s| s.1).sum::<u64>()
        + spacing as u64 * sizes.len().saturating_sub(1) as u64;
    check_image_size(max_w, total_h, PDF_SIZE_HINT)
}

/// Склеивает страницы вертикально (по центру по горизонтали) на белом фоне.
pub(crate) fn stack_pages(
    pages: &[tiny_skia::Pixmap],
    spacing: u32,
) -> Result<tiny_skia::Pixmap, String> {
    if pages.is_empty() {
        return Err("PDF-документ не содержит страниц".to_string());
    }
    let sizes: Vec<(u64, u64)> = pages
        .iter()
        .map(|p| (p.width() as u64, p.height() as u64))
        .collect();
    check_stacked_size(&sizes, spacing)?;
    let max_w = pages.iter().map(|p| p.width()).max().unwrap_or(0);
    let total_h =
        pages.iter().map(|p| p.height()).sum::<u32>() + spacing * (pages.len() as u32 - 1);
    let mut result = white_pixmap(max_w, total_h)?;
    let mut y = 0u32;
    for page in pages {
        copy_pixmap(&mut result, page, (max_w - page.width()) / 2, y);
        y += page.height() + spacing;
    }
    Ok(result)
}

/// Переводит растр страницы (RGBA без премультипликации) в непрозрачный
/// Pixmap, накладывая полупрозрачные пиксели на белый фон. Дальнейшие этапы
/// (кодирование, буфер обмена) рассчитывают на альфу 255 во всех пикселях.
fn rgba_to_opaque_pixmap(rgba: &image::RgbaImage) -> Result<tiny_skia::Pixmap, String> {
    let mut pixmap = white_pixmap(rgba.width(), rgba.height())?;
    for (dst, src) in pixmap
        .data_mut()
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(rgba.as_raw().as_chunks::<4>().0.iter())
    {
        let a = src[3] as u16;
        for c in 0..3 {
            dst[c] = ((src[c] as u16 * a + 255 * (255 - a)) / 255) as u8;
        }
        dst[3] = 255;
    }
    Ok(pixmap)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn solid(w: u32, h: u32, rgb: (u8, u8, u8)) -> tiny_skia::Pixmap {
        let mut p = tiny_skia::Pixmap::new(w, h).unwrap();
        p.fill(tiny_skia::Color::from_rgba8(rgb.0, rgb.1, rgb.2, 255));
        p
    }

    fn pixel(p: &tiny_skia::Pixmap, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * p.width() + x) * 4) as usize;
        p.data()[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn stack_pages_centers_pages_with_spacing() {
        let pages = [solid(100, 50, (255, 0, 0)), solid(200, 60, (0, 255, 0))];
        let r = stack_pages(&pages, 10).unwrap();
        assert_eq!((r.width(), r.height()), (200, 120));
        // первая страница по центру: поля по 50 px белые
        assert_eq!(pixel(&r, 10, 20), [255, 255, 255, 255]);
        assert_eq!(pixel(&r, 100, 20), [255, 0, 0, 255]);
        assert_eq!(pixel(&r, 190, 20), [255, 255, 255, 255]);
        // интервал между страницами белый
        assert_eq!(pixel(&r, 100, 55), [255, 255, 255, 255]);
        // вторая страница во всю ширину
        assert_eq!(pixel(&r, 0, 60), [0, 255, 0, 255]);
        assert_eq!(pixel(&r, 199, 119), [0, 255, 0, 255]);
    }

    #[test]
    fn stacked_size_is_checked_before_rendering() {
        // 1000 страниц A4 при 300 DPI — выше предела высоты
        let pages = vec![(2480u64, 3508u64); 1000];
        let e = check_stacked_size(&pages, 10).unwrap_err();
        assert!(e.contains("НомерСтраницы"), "{e}");
        // предел площади при допустимых сторонах
        let e = check_stacked_size(&[(60_000, 60_000)], 0).unwrap_err();
        assert!(e.contains("Мпикс"), "{e}");
        assert!(check_stacked_size(&[(2480, 3508), (2480, 3508)], 10).is_ok());
    }

    #[test]
    fn transparent_pixels_become_white() {
        let mut rgba = image::RgbaImage::new(2, 1);
        rgba.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
        rgba.put_pixel(1, 0, image::Rgba([0, 0, 0, 255]));
        let p = rgba_to_opaque_pixmap(&rgba).unwrap();
        assert_eq!(pixel(&p, 0, 0), [255, 255, 255, 255]);
        assert_eq!(pixel(&p, 1, 0), [0, 0, 0, 255]);
    }

    #[test]
    fn dpi_follows_scale_without_clamping() {
        assert_eq!(effective_dpi(96, 100), 96.0);
        assert_eq!(effective_dpi(600, 1000), 6000.0);
        assert_eq!(effective_dpi(50, 10), 5.0);
    }
}

#[cfg(windows)]
mod win_pdf {
    use super::PdfInput;
    use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
    use windows::Storage::Streams::{DataReader, DataWriter, InMemoryRandomAccessStream};
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

    /// Выполняет `f` в фоновом потоке COM MTA (WinRT PDF нельзя
    /// использовать из STA-потока клиента 1С без риска блокировки).
    fn in_mta<T: Send + 'static>(
        f: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        std::thread::spawn(move || {
            // CoUninitialize вызывается только при успешной инициализации
            // (включая S_FALSE — «уже инициализирован»), иначе счётчик
            // инициализаций COM потока нарушается.
            let com_initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
            let res = f();
            if com_initialized {
                unsafe { CoUninitialize() };
            }
            res
        })
        .join()
        .map_err(|_| "сбой потока обработки PDF".to_string())?
    }

    pub fn render_pdf(
        input: PdfInput,
        scale: u32,
        page_number: u32,
        base_dpi: u32,
        spacing: u32,
    ) -> Result<tiny_skia::Pixmap, String> {
        let bytes = input.read_validated()?.into_owned();
        in_mta(move || render_in_mta(&bytes, scale, page_number, base_dpi, spacing))
    }

    /// Число страниц PDF.
    pub fn page_count(input: PdfInput) -> Result<u32, String> {
        let bytes = input.read_validated()?.into_owned();
        in_mta(move || {
            load_document(&bytes)?
                .PageCount()
                .map_err(|e| format!("ошибка получения числа страниц: {e}"))
        })
    }

    fn load_document(bytes: &[u8]) -> Result<PdfDocument, String> {
        let stream = InMemoryRandomAccessStream::new()
            .map_err(|e| format!("не удалось создать поток WinRT: {e}"))?;
        let writer = DataWriter::CreateDataWriter(&stream)
            .map_err(|e| format!("не удалось создать DataWriter: {e}"))?;
        writer
            .WriteBytes(bytes)
            .map_err(|e| format!("ошибка записи байтов в DataWriter: {e}"))?;
        writer
            .StoreAsync()
            .map_err(|e| format!("ошибка сохранения DataWriter: {e}"))?
            .join()
            .map_err(|e| format!("ошибка завершения записи в поток: {e}"))?;
        writer
            .FlushAsync()
            .map_err(|e| format!("ошибка сброса DataWriter: {e}"))?
            .join()
            .map_err(|e| format!("ошибка сброса потока: {e}"))?;
        PdfDocument::LoadFromStreamAsync(&stream)
            .map_err(|e| format!("ошибка загрузки PDF-документа: {e}"))?
            .join()
            .map_err(|e| format!("ошибка открытия PDF-документа: {e}"))
    }

    fn render_in_mta(
        bytes: &[u8],
        scale: u32,
        page_number: u32,
        base_dpi: u32,
        spacing: u32,
    ) -> Result<tiny_skia::Pixmap, String> {
        let doc = load_document(bytes)?;

        let page_count = doc
            .PageCount()
            .map_err(|e| format!("ошибка получения числа страниц: {e}"))?;
        if page_count == 0 {
            return Err("PDF-документ не содержит страниц".to_string());
        }
        let pages_to_render: Vec<u32> = if page_number == 0 {
            (0..page_count).collect()
        } else if page_number > page_count {
            return Err(format!(
                "запрошена страница {page_number}, но в документе всего {page_count} стр."
            ));
        } else {
            vec![page_number - 1]
        };

        // Размеры всех страниц известны до рендера: проверяем итоговую
        // картинку сразу, а не после растеризации сотен страниц.
        let dpi_scale = super::effective_dpi(base_dpi, scale) / 96.0;
        let mut pages = Vec::with_capacity(pages_to_render.len());
        let mut sizes = Vec::with_capacity(pages_to_render.len());
        for &p_idx in &pages_to_render {
            let page = doc
                .GetPage(p_idx)
                .map_err(|e| format!("ошибка получения страницы {}: {e}", p_idx + 1))?;
            let size = page
                .Size()
                .map_err(|e| format!("ошибка получения размера страницы {}: {e}", p_idx + 1))?;
            let dest_w = (size.Width * dpi_scale).round() as u32;
            let dest_h = (size.Height * dpi_scale).round() as u32;
            if dest_w == 0 || dest_h == 0 {
                return Err(format!(
                    "некорректный размер страницы {}: {dest_w}x{dest_h} — увеличьте Масштаб \
                     или РазрешениеPDF",
                    p_idx + 1
                ));
            }
            sizes.push((dest_w as u64, dest_h as u64));
            pages.push((p_idx, page, dest_w, dest_h));
        }
        super::check_stacked_size(&sizes, spacing)?;

        let mut rendered = Vec::with_capacity(pages.len());
        for (p_idx, page, dest_w, dest_h) in pages {
            let render_options = PdfPageRenderOptions::new()
                .map_err(|e| format!("не удалось создать PdfPageRenderOptions: {e}"))?;
            render_options
                .SetDestinationWidth(dest_w)
                .map_err(|e| format!("не удалось задать ширину рендеринга: {e}"))?;
            render_options
                .SetDestinationHeight(dest_h)
                .map_err(|e| format!("не удалось задать высоту рендеринга: {e}"))?;

            // BitmapEncoderId намеренно не задаётся: RenderWithOptionsToStreamAsync
            // без него и так кодирует PNG, а явная установка этого свойства
            // (с любым GUID) приводит к E_FAIL (0x80004005) — проверено на
            // реальных PDF-файлах 1С.
            let page_stream = InMemoryRandomAccessStream::new()
                .map_err(|e| format!("не удалось создать поток для страницы: {e}"))?;
            page.RenderWithOptionsToStreamAsync(&page_stream, &render_options)
                .map_err(|e| format!("ошибка запуска рендеринга страницы {}: {e}", p_idx + 1))?
                .join()
                .map_err(|e| format!("ошибка рендеринга страницы {}: {e}", p_idx + 1))?;

            let size_u64 = page_stream
                .Size()
                .map_err(|e| format!("ошибка получения размера потока страницы: {e}"))?;
            let page_reader = DataReader::CreateDataReader(&page_stream)
                .map_err(|e| format!("не удалось создать DataReader для страницы: {e}"))?;
            page_reader
                .LoadAsync(size_u64 as u32)
                .map_err(|e| format!("ошибка чтения потока страницы: {e}"))?
                .join()
                .map_err(|e| format!("ошибка загрузки данных страницы: {e}"))?;
            let mut page_bytes = vec![0u8; size_u64 as usize];
            page_reader
                .ReadBytes(&mut page_bytes)
                .map_err(|e| format!("ошибка чтения байтов страницы: {e}"))?;

            let img = image::load_from_memory_with_format(&page_bytes, image::ImageFormat::Png)
                .map_err(|e| format!("ошибка декодирования страницы {}: {e}", p_idx + 1))?;
            rendered.push(super::rgba_to_opaque_pixmap(&img.to_rgba8())?);
        }

        super::stack_pages(&rendered, spacing)
    }
}

#[cfg(not(windows))]
mod nix_pdf {
    use super::PdfInput;
    use std::path::Path;
    use std::process::{Command, ExitStatus, Stdio};
    use std::time::{Duration, Instant};

    /// Предел времени работы `pdftoppm`/`pdfinfo`: на отдельных PDF poppler
    /// может зависнуть, а вызов из 1С без предела ждал бы вечно.
    const TOOL_TIMEOUT: Duration = Duration::from_secs(120);

    /// Результат запуска внешней утилиты.
    struct ToolOutput {
        status: ExitStatus,
        stdout: String,
        stderr: String,
    }

    /// Запускает утилиту с пределом времени. Вывод пишется в файлы
    /// временного каталога: канал (pipe) без чтения мог бы переполниться и
    /// заблокировать процесс.
    fn run_tool(mut cmd: Command, tool: &str, dir: &Path) -> Result<ToolOutput, String> {
        let out_path = dir.join(format!("{tool}.out"));
        let err_path = dir.join(format!("{tool}.err"));
        let create = |p: &Path| {
            std::fs::File::create(p)
                .map_err(|e| format!("не удалось создать файл вывода {tool}: {e}"))
        };
        cmd.stdin(Stdio::null())
            .stdout(create(&out_path)?)
            .stderr(create(&err_path)?);
        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!("утилита '{tool}' не найдена в PATH — установите пакет poppler-utils")
            } else {
                format!("ошибка запуска {tool}: {e}")
            }
        })?;
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() > TOOL_TIMEOUT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "{tool} не завершился за {} с — PDF слишком сложный или повреждён",
                        TOOL_TIMEOUT.as_secs()
                    ));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => return Err(format!("ошибка ожидания {tool}: {e}")),
            }
        };
        let read =
            |p: &Path| String::from_utf8_lossy(&std::fs::read(p).unwrap_or_default()).into_owned();
        Ok(ToolOutput {
            status,
            stdout: read(&out_path),
            stderr: read(&err_path),
        })
    }

    /// Сведения о страницах из `pdfinfo`: число страниц и размеры
    /// (ширина, высота в пунктах с учётом поворота) всех страниц.
    #[derive(Debug, PartialEq)]
    struct PdfInfo {
        pages: u32,
        sizes: Vec<(f64, f64)>,
    }

    fn parse_pdfinfo(text: &str) -> Option<PdfInfo> {
        let mut pages = None;
        // номер страницы → (размер по CropBox, размер по size, поворот)
        let mut crop: std::collections::BTreeMap<u32, (f64, f64)> = Default::default();
        let mut size: std::collections::BTreeMap<u32, (f64, f64)> = Default::default();
        let mut rot: std::collections::HashMap<u32, i64> = Default::default();
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("Pages:") {
                pages = v.trim().parse().ok();
                continue;
            }
            let Some(rest) = line.strip_prefix("Page ") else {
                continue;
            };
            let mut it = rest.split_whitespace();
            let Some(n) = it.next().and_then(|n| n.parse::<u32>().ok()) else {
                continue;
            };
            let nums: Vec<f64> = it.clone().skip(1).filter_map(|v| v.parse().ok()).collect();
            match it.next() {
                Some("CropBox:") if nums.len() == 4 => {
                    crop.insert(n, ((nums[2] - nums[0]).abs(), (nums[3] - nums[1]).abs()));
                }
                Some("size:") if nums.len() >= 2 => {
                    size.insert(n, (nums[0], nums[1]));
                }
                Some("rot:") => {
                    if let Some(&r) = nums.first() {
                        rot.insert(n, r as i64);
                    }
                }
                _ => {}
            }
        }
        let pages = pages?;
        let mut sizes = Vec::new();
        for n in 1..=pages {
            let (w, h) = crop.get(&n).or_else(|| size.get(&n)).copied()?;
            let swapped = rot.get(&n).is_some_and(|r| r.rem_euclid(180) == 90);
            sizes.push(if swapped { (h, w) } else { (w, h) });
        }
        Some(PdfInfo { pages, sizes })
    }

    /// Временный каталог и путь к PDF для утилит poppler: файл
    /// используется на месте, данные из памяти записываются в каталог.
    fn prepare(input: PdfInput) -> Result<(tempfile::TempDir, std::path::PathBuf), String> {
        let bytes = input.read_validated()?;
        let temp_dir = tempfile::Builder::new()
            .prefix("spreadsheet_pdf_")
            .tempdir()
            .map_err(|e| format!("не удалось создать временный каталог: {e}"))?;
        let path = match input {
            PdfInput::File(path) => path.to_path_buf(),
            PdfInput::Bytes(_) => {
                let path = temp_dir.path().join("input.pdf");
                std::fs::write(&path, &bytes)
                    .map_err(|e| format!("не удалось записать временный PDF-файл: {e}"))?;
                path
            }
        };
        Ok((temp_dir, path))
    }

    /// Сведения `pdfinfo` о документе; `None`, если утилита недоступна или
    /// её вывод не разобран.
    fn pdf_info(path: &Path, dir: &Path) -> Option<PdfInfo> {
        let mut cmd = Command::new("pdfinfo");
        cmd.arg("-box")
            .arg("-f")
            .arg("1")
            .arg("-l")
            .arg(i32::MAX.to_string())
            .arg(path);
        let out = run_tool(cmd, "pdfinfo", dir).ok()?;
        out.status
            .success()
            .then(|| parse_pdfinfo(&out.stdout))
            .flatten()
    }

    /// Число страниц PDF (по данным `pdfinfo`).
    pub fn page_count(input: PdfInput) -> Result<u32, String> {
        let (temp_dir, path) = prepare(input)?;
        let mut cmd = Command::new("pdfinfo");
        cmd.arg(&path);
        let out = run_tool(cmd, "pdfinfo", temp_dir.path())?;
        if !out.status.success() {
            return Err(format!("ошибка выполнения pdfinfo: {}", out.stderr.trim()));
        }
        out.stdout
            .lines()
            .find_map(|l| l.strip_prefix("Pages:"))
            .and_then(|v| v.trim().parse().ok())
            .ok_or_else(|| "pdfinfo не сообщил число страниц PDF".to_string())
    }

    pub fn render_pdf(
        input: PdfInput,
        scale: u32,
        page_number: u32,
        base_dpi: u32,
        spacing: u32,
    ) -> Result<tiny_skia::Pixmap, String> {
        let (temp_dir, path) = prepare(input)?;
        let path = path.as_path();
        let dpi = super::effective_dpi(base_dpi, scale);

        // Предварительная проверка по pdfinfo (тот же пакет poppler-utils):
        // число страниц и итоговый размер картинки — до растеризации.
        // Если pdfinfo недоступен или его вывод не разобран, проверка
        // выполняется только по факту рендера.
        {
            if let Some(info) = pdf_info(path, temp_dir.path()) {
                if info.pages == 0 {
                    return Err("PDF-документ не содержит страниц".to_string());
                }
                if page_number > info.pages {
                    return Err(format!(
                        "запрошена страница {page_number}, но в документе всего {} стр.",
                        info.pages
                    ));
                }
                let selected: Vec<(f64, f64)> = if page_number == 0 {
                    info.sizes
                } else {
                    vec![info.sizes[page_number as usize - 1]]
                };
                // оценка снизу (pdftoppm округляет вверх) — отказ только
                // там, где картинка заведомо не помещается
                let px = |pt: f64| (pt * dpi as f64 / 72.0).floor() as u64;
                let estimate: Vec<(u64, u64)> =
                    selected.iter().map(|&(w, h)| (px(w), px(h))).collect();
                super::check_stacked_size(&estimate, spacing)?;
            }
        }

        // PPM вместо PNG: без сжатия и распаковки промежуточных файлов
        let mut cmd = Command::new("pdftoppm");
        cmd.arg("-r").arg(format!("{dpi}"));
        if page_number > 0 {
            cmd.arg("-f")
                .arg(page_number.to_string())
                .arg("-l")
                .arg(page_number.to_string());
        }
        cmd.arg(path).arg(temp_dir.path().join("page"));
        let output = run_tool(cmd, "pdftoppm", temp_dir.path())?;
        if !output.status.success() {
            return Err(format!(
                "ошибка выполнения pdftoppm: {}",
                output.stderr.trim()
            ));
        }

        // имена вида page-1.ppm / page-01.ppm (ширина номера зависит от
        // числа страниц)
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(temp_dir.path())
            .map_err(|e| format!("не удалось прочитать временный каталог: {e}"))?
        {
            let entry = entry.map_err(|e| format!("ошибка чтения временного каталога: {e}"))?;
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if let Some(num) = name
                .strip_prefix("page-")
                .and_then(|n| n.strip_suffix(".ppm"))
                .and_then(|n| n.parse::<u32>().ok())
            {
                entries.push((num, entry.path()));
            }
        }
        if entries.is_empty() {
            return Err("рендеринг PDF не создал ни одной страницы".to_string());
        }
        entries.sort_by_key(|k| k.0);

        let mut rendered = Vec::with_capacity(entries.len());
        for (_, file_path) in &entries {
            let img = image::open(file_path)
                .map_err(|e| format!("ошибка чтения временного изображения страницы: {e}"))?;
            rendered.push(super::rgba_to_opaque_pixmap(&img.to_rgba8())?);
            // освобождаем место во временном каталоге по ходу чтения
            let _ = std::fs::remove_file(file_path);
        }
        super::stack_pages(&rendered, spacing)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_pdfinfo_with_crop_box_and_rotation() {
            let text = "Pages:           2\n\
                Page    1 size:  595 x 842 pts (A4)\n\
                Page    1 rot:   0\n\
                Page    2 size:  595 x 842 pts (A4)\n\
                Page    2 rot:   90\n\
                Page    1 MediaBox:      0.00     0.00   595.00   842.00\n\
                Page    1 CropBox:      10.00    20.00   510.00   720.00\n\
                Page    2 MediaBox:      0.00     0.00   595.00   842.00\n";
            let info = parse_pdfinfo(text).unwrap();
            assert_eq!(info.pages, 2);
            assert_eq!(info.sizes, vec![(500.0, 700.0), (842.0, 595.0)]);
        }

        #[test]
        fn pdfinfo_without_page_sizes_is_ignored() {
            assert!(parse_pdfinfo("Pages: 3\n").is_none());
            assert!(parse_pdfinfo("garbage").is_none());
        }
    }
}
