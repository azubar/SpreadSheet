//! Внешняя компонента 1С «SpreadSheet» (Native API).
//!
//! Конвертирует табличный документ 1С 8.х (*.mxl) или *.pdf в JPEG или PNG:
//! * `СохранитьКартинку(ПутьКФайлу, ПутьКартинки, ТипКартинки)` — в файл;
//! * `ПолучитьКартинку(ПутьКФайлу, ТипКартинки)` — в значение ДвоичныеДанные;
//! * `ПолучитьКартинкуВБуферОбмена(ПутьКФайлу, ТипКартинки)` — в буфер обмена;
//! * те же методы с суффиксом `ИзДанных` принимают документ значением
//!   ДвоичныеДанные вместо пути к файлу;
//! * `КоличествоСтраниц(ПутьКФайлу)` / `КоличествоСтраницИзДанных(Данные)` —
//!   число страниц (для выбора `НомерСтраницы`).
//!
//! Свойства (`Качество`, `Масштаб` и др.) применяются в момент вызова методов
//! и проверяются до начала конвертации; `Версия` — версия компоненты.
//! Ошибка вызова по умолчанию вызывает исключение в 1С; при
//! `ВызыватьИсключения = Ложь` метод возвращает `Ложь` (или пустые
//! ДвоичныеДанные), а текст ошибки доступен в свойстве `ОписаниеОшибки`.

pub mod converter;
pub mod mxl_parser;
pub mod pdf_renderer;

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::Arc;

use native_api_1c::{
    native_api_1c_core::ffi::connection::{Connection, MessageCode},
    native_api_1c_macro::{extern_functions, AddIn},
};

use converter::{ImageKind, PngCompression};

const COMPONENT_NAME: &str = "SpreadSheet";

/// Имя структуры задаёт имя класса компоненты в 1С:
/// `register_extension_as()` (макрос `#[derive(AddIn)]`) возвращает имя
/// структуры, поэтому объект создаётся как `Новый("AddIn.<ИмяПодключения>.SpreadSheet")`.
/// Переименование структуры сломает идентификатор `AddIn.….SpreadSheet`.
// типы полей-методов — сигнатуры `fn(...)`, которые разбирает макрос AddIn
#[allow(clippy::type_complexity)]
#[derive(AddIn)]
pub struct SpreadSheet {
    /// Соединение с платформой 1С (для передачи сообщений об ошибках)
    #[add_in_con]
    connection: Arc<Option<&'static Connection>>,

    /// Версия компоненты (из Cargo.toml)
    #[add_in_prop(ty = Str, name = "Version", name_ru = "Версия", readable)]
    pub version: String,

    /// Качество сжатия JPEG, 1–100. Игнорируется для PNG.
    #[add_in_prop(ty = Int, name = "Quality", name_ru = "Качество", readable, writable)]
    pub quality: i32,

    /// Масштаб рендера в процентах от исходного размера листа (100 = как в 1С)
    #[add_in_prop(ty = Int, name = "Scale", name_ru = "Масштаб", readable, writable)]
    pub scale: i32,

    /// Номер страницы PDF (0 = все страницы склеенные, 1..N = конкретная страница)
    #[add_in_prop(ty = Int, name = "PageNumber", name_ru = "НомерСтраницы", readable, writable)]
    pub page_number: i32,

    /// Разрешение PDF при рендеринге в DPI (50..600)
    #[add_in_prop(ty = Int, name = "PdfDpi", name_ru = "РазрешениеPDF", readable, writable)]
    pub pdf_dpi: i32,

    /// Интервал между страницами PDF при склеивании в пикселях (0..200)
    #[add_in_prop(ty = Int, name = "PdfPageSpacing", name_ru = "ИнтервалСтраницPDF", readable, writable)]
    pub pdf_page_spacing: i32,

    /// Внешние поля картинки в пикселях (0..500)
    #[add_in_prop(ty = Int, name = "ImageMargins", name_ru = "ПоляКартинки", readable, writable)]
    pub image_margins: i32,

    /// Максимальный размер файла картинки в байтах (по умолчанию 50 000 000)
    #[add_in_prop(ty = Int, name = "MaxImageBytes", name_ru = "МаксимальныйРазмерКартинки", readable, writable)]
    pub max_image_bytes: i32,

    /// Степень сжатия PNG: 0 — быстро, 1 — обычно (по умолчанию), 2 — максимально
    #[add_in_prop(ty = Int, name = "PngCompression", name_ru = "СжатиеPNG", readable, writable)]
    pub png_compression: i32,

    /// Вызывать исключение при ошибке (по умолчанию Истина). При Ложь метод
    /// возвращает Ложь / пустые ДвоичныеДанные, текст — в ОписаниеОшибки.
    #[add_in_prop(ty = Bool, name = "ThrowErrors", name_ru = "ВызыватьИсключения", readable, writable)]
    pub throw_errors: bool,

    /// Текст ошибки последнего вызова метода (пустая строка — успех)
    #[add_in_prop(ty = Str, name = "LastError", name_ru = "ОписаниеОшибки", readable)]
    pub last_error: String,

    /// СохранитьКартинку(ПутьКФайлу, ПутьКартинки, ТипКартинки) -> Булево
    #[add_in_func(name = "SavePicture", name_ru = "СохранитьКартинку")]
    #[arg(ty = Str)]
    #[arg(ty = Str)]
    #[arg(ty = Str)]
    #[returns(ty = Bool, result)]
    pub save_picture: fn(&mut Self, String, String, String) -> Result<bool, ()>,

    /// ПолучитьКартинку(ПутьКФайлу, ТипКартинки) -> ДвоичныеДанные
    #[add_in_func(name = "GetPicture", name_ru = "ПолучитьКартинку")]
    #[arg(ty = Str)]
    #[arg(ty = Str)]
    #[returns(ty = Blob, result)]
    pub get_picture: fn(&mut Self, String, String) -> Result<Vec<u8>, ()>,

    /// ПолучитьКартинкуВБуферОбмена(ПутьКФайлу, ТипКартинки) -> Булево
    #[add_in_func(
        name = "GetPictureToClipboard",
        name_ru = "ПолучитьКартинкуВБуферОбмена"
    )]
    #[arg(ty = Str)]
    #[arg(ty = Str)]
    #[returns(ty = Bool, result)]
    pub get_picture_to_clipboard: fn(&mut Self, String, String) -> Result<bool, ()>,

    /// СохранитьКартинкуИзДанных(Данные, ПутьКартинки, ТипКартинки) -> Булево
    #[add_in_func(name = "SavePictureFromData", name_ru = "СохранитьКартинкуИзДанных")]
    #[arg(ty = Blob)]
    #[arg(ty = Str)]
    #[arg(ty = Str)]
    #[returns(ty = Bool, result)]
    pub save_picture_from_data: fn(&mut Self, Vec<u8>, String, String) -> Result<bool, ()>,

    /// ПолучитьКартинкуИзДанных(Данные, ТипКартинки) -> ДвоичныеДанные
    #[add_in_func(name = "GetPictureFromData", name_ru = "ПолучитьКартинкуИзДанных")]
    #[arg(ty = Blob)]
    #[arg(ty = Str)]
    #[returns(ty = Blob, result)]
    pub get_picture_from_data: fn(&mut Self, Vec<u8>, String) -> Result<Vec<u8>, ()>,

    /// ПолучитьКартинкуИзДанныхВБуферОбмена(Данные, ТипКартинки) -> Булево
    #[add_in_func(
        name = "GetPictureFromDataToClipboard",
        name_ru = "ПолучитьКартинкуИзДанныхВБуферОбмена"
    )]
    #[arg(ty = Blob)]
    #[arg(ty = Str)]
    #[returns(ty = Bool, result)]
    pub get_picture_from_data_to_clipboard: fn(&mut Self, Vec<u8>, String) -> Result<bool, ()>,

    /// КоличествоСтраниц(ПутьКФайлу) -> Число
    #[add_in_func(name = "PageCount", name_ru = "КоличествоСтраниц")]
    #[arg(ty = Str)]
    #[returns(ty = Int, result)]
    pub page_count: fn(&mut Self, String) -> Result<i32, ()>,

    /// КоличествоСтраницИзДанных(Данные) -> Число
    #[add_in_func(name = "PageCountFromData", name_ru = "КоличествоСтраницИзДанных")]
    #[arg(ty = Blob)]
    #[returns(ty = Int, result)]
    pub page_count_from_data: fn(&mut Self, Vec<u8>) -> Result<i32, ()>,

    /// Буфер обмена держится открытым, пока жив объект компоненты: в Linux
    /// (X11/Wayland) содержимое буфера обмена отдаёт сам процесс-владелец, и
    /// при закрытии объекта `arboard::Clipboard` сразу после вызова картинка
    /// могла пропасть до вставки.
    clipboard: Option<arboard::Clipboard>,
}

impl Default for SpreadSheet {
    fn default() -> Self {
        Self {
            connection: Arc::new(None),
            version: env!("CARGO_PKG_VERSION").to_string(),
            quality: 90,
            scale: 100,
            page_number: 0,
            pdf_dpi: 96,
            pdf_page_spacing: 10,
            image_margins: 0,
            max_image_bytes: converter::DEFAULT_MAX_IMAGE_BYTES as i32,
            png_compression: 1,
            throw_errors: true,
            last_error: String::new(),
            save_picture: Self::save_picture_inner,
            get_picture: Self::get_picture_inner,
            get_picture_to_clipboard: Self::get_picture_to_clipboard_inner,
            save_picture_from_data: Self::save_picture_from_data_inner,
            get_picture_from_data: Self::get_picture_from_data_inner,
            get_picture_from_data_to_clipboard: Self::get_picture_from_data_to_clipboard_inner,
            page_count: Self::page_count_inner,
            page_count_from_data: Self::page_count_from_data_inner,
            clipboard: None,
        }
    }
}

/// Источник документа: путь к файлу или данные в памяти.
#[derive(Clone, Copy)]
enum Source<'a> {
    File(&'a str),
    Data(&'a [u8]),
}

impl<'a> From<&'a str> for Source<'a> {
    fn from(path: &'a str) -> Self {
        Source::File(path)
    }
}

/// Загруженный документ: PDF (рендерится средствами ОС) или разобранный MXL.
enum Loaded<'a> {
    Pdf(pdf_renderer::PdfInput<'a>),
    Mxl(Box<mxl_parser::Document>),
}

/// Проверенные значения общих свойств.
#[derive(Debug)]
struct Settings {
    kind: ImageKind,
    scale: u32,
    max_bytes: usize,
    margins: u32,
}

/// Выполняет `f`, превращая panic в ошибку: panic не должен выйти за
/// границу FFI в процесс 1С.
fn guarded<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err("внутренняя ошибка конвертации (panic)".to_string()))
}

impl SpreadSheet {
    fn save_picture_inner(
        &mut self,
        path: String,
        image_path: String,
        image_type: String,
    ) -> Result<bool, ()> {
        self.save_picture_from(Source::File(&path), &image_path, &image_type)
    }

    fn get_picture_inner(&mut self, path: String, image_type: String) -> Result<Vec<u8>, ()> {
        self.get_picture_from(Source::File(&path), &image_type)
    }

    fn get_picture_to_clipboard_inner(
        &mut self,
        path: String,
        image_type: String,
    ) -> Result<bool, ()> {
        self.get_picture_to_clipboard_from(Source::File(&path), &image_type)
    }

    fn save_picture_from_data_inner(
        &mut self,
        data: Vec<u8>,
        image_path: String,
        image_type: String,
    ) -> Result<bool, ()> {
        self.save_picture_from(Source::Data(&data), &image_path, &image_type)
    }

    fn get_picture_from_data_inner(
        &mut self,
        data: Vec<u8>,
        image_type: String,
    ) -> Result<Vec<u8>, ()> {
        self.get_picture_from(Source::Data(&data), &image_type)
    }

    fn get_picture_from_data_to_clipboard_inner(
        &mut self,
        data: Vec<u8>,
        image_type: String,
    ) -> Result<bool, ()> {
        self.get_picture_to_clipboard_from(Source::Data(&data), &image_type)
    }

    fn page_count_inner(&mut self, path: String) -> Result<i32, ()> {
        let result = guarded(|| self.count_pages(Source::File(&path)));
        self.finish(result, 0)
    }

    fn page_count_from_data_inner(&mut self, data: Vec<u8>) -> Result<i32, ()> {
        let result = guarded(|| self.count_pages(Source::Data(&data)));
        self.finish(result, 0)
    }

    fn save_picture_from(
        &mut self,
        source: Source,
        image_path: &str,
        image_type: &str,
    ) -> Result<bool, ()> {
        let result = guarded(|| {
            let (pixmap, settings) = self.prepare_pixmap(source, image_type)?;
            converter::encode_to_file(
                &pixmap,
                Path::new(image_path),
                &settings.kind,
                settings.max_bytes,
            )
        });
        self.finish(result.map(|()| true), false)
    }

    fn get_picture_from(&mut self, source: Source, image_type: &str) -> Result<Vec<u8>, ()> {
        let result = guarded(|| {
            let (pixmap, settings) = self.prepare_pixmap(source, image_type)?;
            converter::encode_to_vec(&pixmap, &settings.kind, settings.max_bytes)
        });
        self.finish(result, Vec::new())
    }

    fn get_picture_to_clipboard_from(
        &mut self,
        source: Source,
        image_type: &str,
    ) -> Result<bool, ()> {
        let result = guarded(|| {
            let (pixmap, _settings) = self.prepare_pixmap(source, image_type)?;
            self.set_clipboard_image(&pixmap)
        });
        self.finish(result.map(|()| true), false)
    }

    fn set_clipboard_image(&mut self, pixmap: &tiny_skia::Pixmap) -> Result<(), String> {
        // Все этапы рендера дают непрозрачные пиксели (альфа 255), а для
        // таких пикселей премультиплицированный RGBA tiny-skia совпадает
        // с обычным RGBA, который ожидает arboard, — буфер передаётся
        // без копирования.
        let img_data = arboard::ImageData {
            width: pixmap.width() as usize,
            height: pixmap.height() as usize,
            bytes: std::borrow::Cow::Borrowed(pixmap.data()),
        };
        let clipboard = match &mut self.clipboard {
            Some(clipboard) => clipboard,
            empty => empty.insert(
                arboard::Clipboard::new()
                    .map_err(|e| format!("нет доступа к буферу обмена: {e}"))?,
            ),
        };
        let result = clipboard
            .set_image(img_data)
            .map_err(|e| format!("не удалось поместить картинку в буфер обмена: {e}"));
        if result.is_err() {
            // соединение с буфером обмена могло устареть — при следующем
            // вызове оно будет открыто заново
            self.clipboard = None;
        }
        result
    }

    /// Завершает вызов метода: запоминает текст ошибки в `ОписаниеОшибки` и
    /// либо вызывает исключение в 1С (`ВызыватьИсключения = Истина`), либо
    /// возвращает `fallback`.
    fn finish<T>(&mut self, result: Result<T, String>, fallback: T) -> Result<T, ()> {
        match result {
            Ok(value) => {
                self.last_error.clear();
                Ok(value)
            }
            Err(message) => {
                self.last_error = message;
                if self.throw_errors {
                    self.report_error(&self.last_error);
                    Err(())
                } else {
                    Ok(fallback)
                }
            }
        }
    }

    /// Проверяет общие свойства и тип картинки — до чтения файла и рендера.
    fn settings(&self, image_type: &str) -> Result<Settings, String> {
        let kind = match image_type.trim().to_uppercase().as_str() {
            "JPEG" | "JPG" => {
                if !(1..=100).contains(&self.quality) {
                    return Err(format!(
                        "недопустимое Качество {}: ожидается от 1 до 100",
                        self.quality
                    ));
                }
                ImageKind::Jpeg {
                    quality: self.quality as u8,
                }
            }
            "PNG" => ImageKind::Png {
                compression: match self.png_compression {
                    0 => PngCompression::Fast,
                    1 => PngCompression::Default,
                    2 => PngCompression::Best,
                    other => {
                        return Err(format!(
                            "недопустимое СжатиеPNG {other}: ожидается 0 (быстро), \
                             1 (обычно) или 2 (максимально)"
                        ))
                    }
                },
            },
            other => {
                return Err(format!(
                    "недопустимый ТипКартинки \"{other}\": ожидается \"JPEG\" или \"PNG\""
                ))
            }
        };
        if !(1..=1000).contains(&self.scale) {
            return Err(format!(
                "недопустимый Масштаб {}: ожидается от 1 до 1000 процентов",
                self.scale
            ));
        }
        if !(1_000_000..=500_000_000).contains(&self.max_image_bytes) {
            return Err(format!(
                "недопустимый МаксимальныйРазмерКартинки {}: ожидается от 1 000 000 до 500 000 000 байт",
                self.max_image_bytes
            ));
        }
        if !(0..=500).contains(&self.image_margins) {
            return Err(format!(
                "недопустимые ПоляКартинки {}: ожидается от 0 до 500 пикселей",
                self.image_margins
            ));
        }
        Ok(Settings {
            kind,
            scale: self.scale as u32,
            max_bytes: self.max_image_bytes as usize,
            margins: self.image_margins as u32,
        })
    }

    /// Проверяет `НомерСтраницы` (для MXL и PDF).
    fn page_number(&self) -> Result<u32, String> {
        u32::try_from(self.page_number).map_err(|_| {
            format!(
                "недопустимый НомерСтраницы {}: ожидается 0 или больше",
                self.page_number
            )
        })
    }

    /// Проверяет свойства, относящиеся только к PDF.
    fn pdf_settings(&self) -> Result<(u32, u32, u32), String> {
        let page_number = self.page_number()?;
        if !(50..=600).contains(&self.pdf_dpi) {
            return Err(format!(
                "недопустимое РазрешениеPDF {}: ожидается от 50 до 600 DPI",
                self.pdf_dpi
            ));
        }
        if !(0..=200).contains(&self.pdf_page_spacing) {
            return Err(format!(
                "недопустимый ИнтервалСтраницPDF {}: ожидается от 0 до 200 пикселей",
                self.pdf_page_spacing
            ));
        }
        Ok((
            page_number,
            self.pdf_dpi as u32,
            self.pdf_page_spacing as u32,
        ))
    }

    /// Определяет тип документа и разбирает MXL.
    fn load<'a>(&self, source: Source<'a>) -> Result<Loaded<'a>, String> {
        match source {
            Source::File(path_str) => {
                let path = Path::new(path_str);
                if !path.is_file() {
                    return Err(format!("файл не найден: {path_str}"));
                }
                if detect_pdf(path)? {
                    Ok(Loaded::Pdf(pdf_renderer::PdfInput::File(path)))
                } else {
                    let doc = mxl_parser::parse_file(path).map_err(|e| e.to_string())?;
                    Ok(Loaded::Mxl(Box::new(doc)))
                }
            }
            Source::Data(data) => {
                if data.is_empty() {
                    return Err("переданы пустые ДвоичныеДанные".to_string());
                }
                if has_pdf_signature(data) {
                    Ok(Loaded::Pdf(pdf_renderer::PdfInput::Bytes(data)))
                } else {
                    if data.len() as u64 > mxl_parser::MAX_FILE_SIZE {
                        return Err(format!(
                            "слишком большой документ ({} МБ, предел {} МБ)",
                            data.len() / 1024 / 1024,
                            mxl_parser::MAX_FILE_SIZE / 1024 / 1024
                        ));
                    }
                    let doc = mxl_parser::parse_bytes(data).map_err(|e| e.to_string())?;
                    Ok(Loaded::Mxl(Box::new(doc)))
                }
            }
        }
    }

    /// Общая часть методов: проверка свойств и типа, разбор и рендер.
    fn prepare_pixmap<'a>(
        &self,
        source: impl Into<Source<'a>>,
        image_type: &str,
    ) -> Result<(tiny_skia::Pixmap, Settings), String> {
        let settings = self.settings(image_type)?;
        let pixmap = match self.load(source.into())? {
            Loaded::Pdf(input) => {
                let (page_number, dpi, spacing) = self.pdf_settings()?;
                pdf_renderer::render_pdf(input, settings.scale, page_number, dpi, spacing)?
            }
            Loaded::Mxl(doc) => match self.page_number()? {
                0 => converter::render(&doc, settings.scale)?,
                page => converter::render_page(&doc, settings.scale, page)?,
            },
        };
        let pixmap = apply_margins(pixmap, settings.margins)?;
        Ok((pixmap, settings))
    }

    /// Число страниц документа (MXL — по области печати и разрывам).
    fn count_pages(&self, source: Source) -> Result<i32, String> {
        let count = match self.load(source)? {
            Loaded::Pdf(input) => pdf_renderer::page_count(input)? as usize,
            Loaded::Mxl(doc) => converter::pages(&doc).len(),
        };
        Ok(count.min(i32::MAX as usize) as i32)
    }

    fn report_error(&self, message: &str) {
        if let Some(connection) = self.connection.as_ref() {
            connection.add_error(
                MessageCode::VeryImportant,
                COMPONENT_NAME,
                &format!("{COMPONENT_NAME}: {message}"),
            );
        }
    }
}

/// Добавляет белые поля `margins` px вокруг картинки.
fn apply_margins(pixmap: tiny_skia::Pixmap, margins: u32) -> Result<tiny_skia::Pixmap, String> {
    if margins == 0 {
        return Ok(pixmap);
    }
    let w = pixmap.width() as u64 + 2 * margins as u64;
    let h = pixmap.height() as u64 + 2 * margins as u64;
    converter::check_image_size(w, h, "уменьшите ПоляКартинки или Масштаб")?;
    let mut result = tiny_skia::Pixmap::new(w as u32, h as u32)
        .ok_or_else(|| "не удалось создать буфер изображения для полей".to_string())?;
    result.fill(tiny_skia::Color::WHITE);
    converter::copy_pixmap(&mut result, &pixmap, margins, margins);
    Ok(result)
}

fn detect_pdf(path: &Path) -> Result<bool, String> {
    if let Some(ext) = path.extension() {
        if ext.to_string_lossy().to_lowercase() == "pdf" {
            return Ok(true);
        }
    }
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("не удалось открыть файл: {e}"))?;
    use std::io::Read;
    let mut buffer = [0; 1024];
    let n = file
        .read(&mut buffer)
        .map_err(|e| format!("ошибка чтения файла: {e}"))?;
    Ok(has_pdf_signature(&buffer[..n]))
}

/// Признак PDF: сигнатура `%PDF-` в первом килобайте (допускается мусор
/// перед заголовком, как и в программах просмотра PDF).
fn has_pdf_signature(data: &[u8]) -> bool {
    data[..data.len().min(1024)]
        .windows(5)
        .any(|window| window == b"%PDF-")
}

extern_functions! {
    SpreadSheet::default(),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Временный файл с заданным расширением и содержимым; удаляется при
    /// выходе из области видимости.
    struct TempFile(PathBuf);

    impl TempFile {
        fn new(name: &str, content: &[u8]) -> TempFile {
            let path = std::env::temp_dir()
                .join(format!("spreadsheet_test_{}_{name}", std::process::id()));
            std::fs::write(&path, content).unwrap();
            TempFile(path)
        }

        fn path(&self) -> &str {
            self.0.to_str().unwrap()
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn demo_mxl() -> String {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../mxl-demo-collection/ДемонстрацияРазмещенияТекста.mxl")
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn default_property_values() {
        let addin = SpreadSheet::default();
        assert_eq!(addin.quality, 90);
        assert_eq!(addin.scale, 100);
        assert_eq!(addin.page_number, 0);
        assert_eq!(addin.pdf_dpi, 96);
        assert_eq!(addin.pdf_page_spacing, 10);
        assert_eq!(addin.image_margins, 0);
        assert_eq!(addin.max_image_bytes, 50_000_000);
        assert_eq!(addin.png_compression, 1);
        assert!(addin.throw_errors);
        assert!(addin.last_error.is_empty());
        assert_eq!(addin.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn rejects_unknown_image_type() {
        let addin = SpreadSheet::default();
        let e = addin.prepare_pixmap("in.mxl", "BMP").unwrap_err();
        assert!(e.contains("ТипКартинки"), "{e}");
        for t in ["jpeg", "Jpg", "png", " PNG "] {
            let e = addin.prepare_pixmap("no-such-file.mxl", t).unwrap_err();
            assert!(e.contains("файл не найден"), "{t}: {e}");
        }
    }

    #[test]
    fn rejects_bad_common_properties_before_reading_file() {
        let cases: [(SpreadSheet, &str, &str); 5] = [
            (
                SpreadSheet {
                    quality: 0,
                    ..Default::default()
                },
                "JPEG",
                "Качество",
            ),
            (
                SpreadSheet {
                    scale: 0,
                    ..Default::default()
                },
                "JPEG",
                "Масштаб",
            ),
            (
                SpreadSheet {
                    max_image_bytes: 500,
                    ..Default::default()
                },
                "JPEG",
                "МаксимальныйРазмерКартинки",
            ),
            (
                SpreadSheet {
                    image_margins: -1,
                    ..Default::default()
                },
                "JPEG",
                "ПоляКартинки",
            ),
            (
                SpreadSheet {
                    png_compression: 3,
                    ..Default::default()
                },
                "PNG",
                "СжатиеPNG",
            ),
        ];
        for (addin, image_type, property) in cases {
            // файл не существует: ошибка свойства должна быть раньше
            let e = addin.prepare_pixmap("x.mxl", image_type).unwrap_err();
            assert!(e.contains(property), "{property}: {e}");
        }
    }

    #[test]
    fn rejects_bad_pdf_properties_before_rendering() {
        // содержимое — заглушка: проверка свойств идёт до чтения PDF
        let pdf = TempFile::new("props.pdf", b"%PDF-1.4\n%%EOF\n");
        let cases: [(SpreadSheet, &str); 4] = [
            (
                SpreadSheet {
                    pdf_dpi: 10,
                    ..Default::default()
                },
                "РазрешениеPDF",
            ),
            (
                SpreadSheet {
                    pdf_dpi: 601,
                    ..Default::default()
                },
                "РазрешениеPDF",
            ),
            (
                SpreadSheet {
                    page_number: -1,
                    ..Default::default()
                },
                "НомерСтраницы",
            ),
            (
                SpreadSheet {
                    pdf_page_spacing: 201,
                    ..Default::default()
                },
                "ИнтервалСтраницPDF",
            ),
        ];
        for (addin, property) in cases {
            let e = addin.prepare_pixmap(pdf.path(), "PNG").unwrap_err();
            assert!(e.contains(property), "{property}: {e}");
        }
    }

    #[test]
    fn margins_add_white_border() {
        let mut p = tiny_skia::Pixmap::new(10, 5).unwrap();
        p.fill(tiny_skia::Color::BLACK);
        let r = apply_margins(p, 3).unwrap();
        assert_eq!((r.width(), r.height()), (16, 11));
        let px = |x: u32, y: u32| r.data()[((y * r.width() + x) * 4) as usize];
        assert_eq!(px(2, 2), 255);
        assert_eq!(px(3, 3), 0);
        assert_eq!(px(12, 7), 0);
        assert_eq!(px(13, 8), 255);
    }

    #[test]
    fn margins_respect_size_limit() {
        let p = tiny_skia::Pixmap::new(65_400, 1).unwrap();
        let e = apply_margins(p, 100).unwrap_err();
        assert!(e.contains("ПоляКартинки"), "{e}");
    }

    #[test]
    fn errors_without_exceptions_go_to_last_error() {
        let mut addin = SpreadSheet {
            throw_errors: false,
            ..Default::default()
        };
        assert_eq!(
            (addin.save_picture)(
                &mut addin,
                "no-such.mxl".into(),
                "out.png".into(),
                "PNG".into()
            ),
            Ok(false)
        );
        assert!(
            addin.last_error.contains("файл не найден"),
            "{}",
            addin.last_error
        );
        assert_eq!(
            (addin.get_picture)(&mut addin, "no-such.mxl".into(), "BMP".into()),
            Ok(Vec::new())
        );
        assert!(
            addin.last_error.contains("ТипКартинки"),
            "{}",
            addin.last_error
        );

        // успешный вызов очищает ОписаниеОшибки
        let bytes = (addin.get_picture)(&mut addin, demo_mxl(), "PNG".into()).unwrap();
        assert!(bytes.starts_with(b"\x89PNG"));
        assert!(addin.last_error.is_empty(), "{}", addin.last_error);
    }

    #[test]
    fn errors_with_exceptions_fail_the_call() {
        let mut addin = SpreadSheet::default();
        assert_eq!(
            (addin.get_picture)(&mut addin, "no-such.mxl".into(), "PNG".into()),
            Err(())
        );
        assert!(addin.last_error.contains("файл не найден"));
    }

    #[test]
    fn png_compression_levels_produce_valid_files() {
        let mut sizes = Vec::new();
        for level in 0..=2 {
            let mut addin = SpreadSheet {
                png_compression: level,
                ..Default::default()
            };
            let bytes = (addin.get_picture)(&mut addin, demo_mxl(), "PNG".into()).unwrap();
            let img = image::load_from_memory(&bytes).unwrap();
            assert!(img.width() > 100);
            sizes.push(bytes.len());
        }
        assert!(
            sizes[2] <= sizes[0],
            "максимальное сжатие крупнее быстрого: {sizes:?}"
        );
    }

    fn printing_demo_path() -> String {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../mxl-demo-collection/ДемонстрацияПечатиРазрывов_A4_Книжная.mxl")
            .to_string_lossy()
            .into_owned()
    }

    /// Минимальный корректный PDF из пустых страниц заданных размеров (pt).
    fn minimal_pdf(pages: &[(u32, u32)]) -> Vec<u8> {
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            format!(
                "<< /Type /Pages /Kids [{}] /Count {} >>",
                (0..pages.len())
                    .map(|i| format!("{} 0 R", i + 3))
                    .collect::<Vec<_>>()
                    .join(" "),
                pages.len()
            ),
        ];
        for (w, h) in pages {
            objects.push(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] >>"
            ));
        }
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    /// PDF-тесты на Linux требуют poppler-utils; без них тест пропускается.
    fn pdf_tools_available() -> bool {
        if cfg!(windows) {
            return true;
        }
        let ok = std::process::Command::new("pdftoppm")
            .arg("-v")
            .output()
            .is_ok();
        if !ok {
            eprintln!("pdftoppm не найден — PDF-тест пропущен");
        }
        ok
    }

    #[test]
    fn picture_from_data_matches_picture_from_file() {
        let path = demo_mxl();
        let data = std::fs::read(&path).unwrap();
        let mut addin = SpreadSheet::default();
        let from_file = (addin.get_picture)(&mut addin, path, "PNG".into()).unwrap();
        let from_data =
            (addin.get_picture_from_data)(&mut addin, data.clone(), "PNG".into()).unwrap();
        assert_eq!(from_file, from_data);

        let out = TempFile::new("from_data.jpg", b"");
        assert_eq!(
            (addin.save_picture_from_data)(&mut addin, data, out.path().into(), "JPEG".into()),
            Ok(true)
        );
        assert!(std::fs::read(&out.0).unwrap().starts_with(&[0xFF, 0xD8]));
    }

    #[test]
    fn bad_data_is_reported() {
        let mut addin = SpreadSheet {
            throw_errors: false,
            ..Default::default()
        };
        let r = (addin.get_picture_from_data)(&mut addin, Vec::new(), "PNG".into());
        assert_eq!(r, Ok(Vec::new()));
        assert!(addin.last_error.contains("пустые"), "{}", addin.last_error);
        let r = (addin.get_picture_from_data)(&mut addin, b"garbage".to_vec(), "PNG".into());
        assert_eq!(r, Ok(Vec::new()));
        assert!(addin.last_error.contains("MOXCEL"), "{}", addin.last_error);
        assert_eq!((addin.page_count_from_data)(&mut addin, Vec::new()), Ok(0));
    }

    #[test]
    fn mxl_page_count_and_page_selection() {
        let path = printing_demo_path();
        let mut addin = SpreadSheet::default();
        assert_eq!((addin.page_count)(&mut addin, path.clone()), Ok(3));
        let data = std::fs::read(&path).unwrap();
        assert_eq!((addin.page_count_from_data)(&mut addin, data), Ok(3));

        let size = |addin: &mut SpreadSheet| {
            let bytes = (addin.get_picture)(addin, path.clone(), "PNG".into()).unwrap();
            let img = image::load_from_memory(&bytes).unwrap();
            (img.width(), img.height())
        };
        let (full_w, full_h) = size(&mut addin);
        addin.page_number = 2;
        let (w, h) = size(&mut addin);
        assert_eq!(w, full_w);
        assert!(h < full_h / 2, "{h} vs {full_h}");

        addin.page_number = 4;
        addin.throw_errors = false;
        let r = (addin.get_picture)(&mut addin, path, "PNG".into());
        assert_eq!(r, Ok(Vec::new()));
        assert!(
            addin.last_error.contains("всего 3 стр."),
            "{}",
            addin.last_error
        );
    }

    #[test]
    fn pdf_from_data_and_page_count() {
        if !pdf_tools_available() {
            return;
        }
        let pdf = minimal_pdf(&[(595, 842), (842, 595), (300, 300)]);
        let mut addin = SpreadSheet::default();
        assert_eq!((addin.page_count_from_data)(&mut addin, pdf.clone()), Ok(3));
        let file = TempFile::new("pages.pdf", &pdf);
        assert_eq!((addin.page_count)(&mut addin, file.path().into()), Ok(3));

        addin.page_number = 2;
        let bytes = (addin.get_picture_from_data)(&mut addin, pdf.clone(), "PNG".into()).unwrap();
        let img = image::load_from_memory(&bytes).unwrap();
        // 842 x 595 pt при 96 DPI ≈ 1123 x 794 px (альбомная страница)
        assert!(
            img.width() > img.height(),
            "{}x{}",
            img.width(),
            img.height()
        );
        let from_file = (addin.get_picture)(&mut addin, file.path().into(), "PNG".into()).unwrap();
        assert_eq!(bytes, from_file);
    }
}
