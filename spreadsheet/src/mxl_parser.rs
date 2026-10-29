//! Парсер табличных документов 1С 8.х в формате MXL (MXL8).
//!
//! Формат: 13-байтовый заголовок `MOXCEL\0` + версия, BOM UTF-8, далее —
//! текстовая скобочная сериализация значений 1С («V81SSerial»).
//! Схема документа восстановлена по референсному парсеру Yoksel
//! (`yoksel/SpreadSheet/Converters/mxl8Import.ml`) и по байт-в-байт разбору
//! реальных файлов 1С 8.3 (v8.1.9 / v8.1.12), см.
//! `docs/Формат_MOXCEL_MXL_полное_описание.md`, Часть 3.
//!
//! Ключевые факты схемы, установленные разбором реальных файлов:
//! * блок строк начинается с 16-го поля корневого объекта (индекс 15);
//! * ячейка: `{маркер, индекс_формата, ...}`; маркер — битовая маска:
//!   2 = типизированное значение `{"S",текст}`, 4 = ссылка-расшифровка,
//!   16 = локализованный текст `{n,есть_текст,{язык,текст}}`;
//! * индекс формата 1-базный (0 = «формата нет»), индекс шрифта 0-базный;
//! * запись формата: `{флаги, значение_на_каждый_установленный_бит...}`;
//!   бит 0 — шрифт, биты 1–4 — рамки Л/В/П/Н, бит 6 — высота строки
//!   (1/288 дюйма), бит 7 — ширина колонки (1/8 ширины символа),
//!   бит 8 — гор. выравнивание (0/2/4/6), бит 9 — верт. (0/8/24),
//!   бит 14 — контроль текста (0 авто/1 обрезать/2 забивать/3 переносить);
//! * после блока строк: группа колонок по умолчанию, число строк,
//!   дополнительные группы колонок, карта строка→группа, затем (среди
//!   служебных полей) объединения ячеек, таблица форматов и таблица шрифтов.
//!
//! Неизвестные/нерасшифрованные поля обрабатываются терпимо (graceful):
//! обязателен только блок строк, остальное ищется по характерной форме.

use std::fmt;
use std::path::Path;

// ---------------------------------------------------------------------------
// Ошибки
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct MxlError(pub String);

impl fmt::Display for MxlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MxlError {}

fn err<T>(msg: impl Into<String>) -> Result<T, MxlError> {
    Err(MxlError(msg.into()))
}

// ---------------------------------------------------------------------------
// Обобщённое дерево значений скобочной сериализации
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Десятичное число (возможно со знаком)
    Num(i64),
    /// Строка в кавычках (кавычка внутри экранируется удвоением)
    Str(String),
    /// «Голый» токен: GUID или иной литерал-ссылка
    Atom(String),
    /// Объект `{...}`
    List(Vec<Value>),
}

impl Value {
    fn as_num(&self) -> Option<i64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }

    fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(items) => Some(items),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
}

/// Разбирает текстовое тело файла (после BOM) в последовательность значений
/// верхнего уровня. Без рекурсии — вложенность произвольная.
fn parse_body(text: &str) -> Result<Vec<Value>, MxlError> {
    let mut stack: Vec<Vec<Value>> = vec![Vec::new()];
    let bytes = text.as_bytes();
    let mut i = 0usize;
    let n = bytes.len();
    while i < n {
        match bytes[i] {
            b' ' | b'\t' | b'\r' | b'\n' | b',' => i += 1,
            b'{' => {
                stack.push(Vec::new());
                i += 1;
            }
            b'}' => {
                let done = match stack.pop() {
                    Some(v) => v,
                    None => return err("MXL: лишняя закрывающая скобка"),
                };
                match stack.last_mut() {
                    Some(top) => top.push(Value::List(done)),
                    None => return err("MXL: несбалансированные скобки"),
                }
                i += 1;
            }
            b'"' => {
                let mut s = String::new();
                i += 1;
                loop {
                    if i >= n {
                        return err("MXL: незакрытая строка в кавычках");
                    }
                    if bytes[i] == b'"' {
                        if i + 1 < n && bytes[i + 1] == b'"' {
                            s.push('"');
                            i += 2;
                        } else {
                            i += 1;
                            break;
                        }
                    } else {
                        // копируем целый UTF-8 символ
                        let ch_len = utf8_len(bytes[i]);
                        s.push_str(&text[i..i + ch_len]);
                        i += ch_len;
                    }
                }
                stack.last_mut().unwrap().push(Value::Str(s));
            }
            _ => {
                let start = i;
                while i < n
                    && !matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n' | b',' | b'{' | b'}')
                {
                    i += 1;
                }
                let tok = &text[start..i];
                let v = match tok.parse::<i64>() {
                    Ok(num) => Value::Num(num),
                    Err(_) => Value::Atom(tok.to_string()),
                };
                stack.last_mut().unwrap().push(v);
            }
        }
    }
    if stack.len() != 1 {
        return err("MXL: не хватает закрывающих скобок");
    }
    Ok(stack.pop().unwrap())
}

fn utf8_len(first_byte: u8) -> usize {
    match first_byte {
        b if b < 0x80 => 1,
        b if b < 0xE0 => 2,
        b if b < 0xF0 => 3,
        _ => 4,
    }
}

// ---------------------------------------------------------------------------
// Модель документа
// ---------------------------------------------------------------------------

/// Формат ячейки/строки/колонки. `None` — свойство не задано (наследуется
/// умолчание). Значения — как в файле, без пересчёта единиц.
#[derive(Debug, Default, Clone)]
pub struct CellFormat {
    /// Индекс шрифта в таблице шрифтов (0-базный)
    pub font: Option<u32>,
    /// Рамки — индекс в таблице линий документа (`Document::lines`, 0-базный).
    /// Сам стиль/толщина линии берётся из этой таблицы, а не из значения
    /// индекса: у каждого документа своя таблица линий, наполняемая в
    /// порядке первого использования. Установленный бит рамки означает
    /// «линия есть» (даже если индекс 0).
    pub border_left: Option<u32>,
    pub border_top: Option<u32>,
    pub border_right: Option<u32>,
    pub border_bottom: Option<u32>,
    /// Цвет рамок — индекс в таблице цветов документа (0-базный)
    pub borders_color: Option<u32>,
    /// Высота строки, 1/288 дюйма (для формата строки)
    pub height: Option<u32>,
    /// Ширина колонки, 1/8 ширины стандартного символа (для формата колонки)
    pub width: Option<u32>,
    /// 0 = лево, 2 = право, 4 = по ширине, 6 = центр
    pub h_align: Option<u32>,
    /// 0 = верх, 8 = низ, 24 = центр
    pub v_align: Option<u32>,
    /// Цвет текста — индекс в таблице цветов документа (0-базный)
    pub text_color: Option<u32>,
    /// Цвет фона — индекс в таблице цветов документа (0-базный)
    pub bg_color: Option<u32>,
    /// Тип узора заливки: 255 = без узора, 0 = сплошной (цветом узора),
    /// 1–17 = штриховки
    pub pattern: Option<u32>,
    /// Цвет узора — индекс в таблице цветов документа (0-базный)
    pub pattern_color: Option<u32>,
    /// 0 = авто (выпускать на соседние), 1 = обрезать, 2 = забивать, 3 = переносить
    pub text_control: Option<u32>,
    /// Ориентация текста в десятых долях градуса (900 = вертикально, 90°)
    pub orientation: Option<u32>,
}

/// Семейство шрифта для рендера: имя шрифта из документа сводится к одному
/// из встроенных метрически совместимых семейств Liberation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontFamily {
    /// Без засечек: Arial, Tahoma, Verdana, Calibri и др. (Liberation Sans)
    #[default]
    Sans,
    /// С засечками: Times New Roman, Georgia и др. (Liberation Serif)
    Serif,
    /// Моноширинный: Courier New, Consolas и др. (Liberation Mono)
    Mono,
}

impl FontFamily {
    /// Семейство по имени шрифта из документа.
    pub fn from_name(name: &str) -> FontFamily {
        let n = name.to_lowercase();
        const MONO: [&str; 10] = [
            "courier",
            "consolas",
            "mono",
            "lucida console",
            "fixedsys",
            "terminal",
            "monaco",
            "menlo",
            "inconsolata",
            "source code",
        ];
        const SERIF: [&str; 12] = [
            "times",
            "georgia",
            "cambria",
            "garamond",
            "antiqua",
            "palatino",
            "bookman",
            "century",
            "constantia",
            "baskerville",
            "tinos",
            "pt serif",
        ];
        if MONO.iter().any(|m| n.contains(m)) {
            FontFamily::Mono
        } else if SERIF.iter().any(|m| n.contains(m))
            || (n.contains("serif") && !n.contains("sans"))
        {
            FontFamily::Serif
        } else {
            FontFamily::Sans
        }
    }
}

#[derive(Debug, Clone)]
pub struct FontSpec {
    /// Семейство (по имени шрифта)
    pub family: FontFamily,
    /// Размер в десятых долях пункта (80 = 8 pt)
    pub size_tenth_pt: u32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikeout: bool,
}

impl Default for FontSpec {
    fn default() -> Self {
        FontSpec {
            family: FontFamily::Sans,
            size_tenth_pt: 80,
            bold: false,
            italic: false,
            underline: false,
            strikeout: false,
        }
    }
}

/// Линия рамки из таблицы линий документа.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineSpec {
    /// Тип линии: 0 — служебная «нет линии/авто» (1С не печатает такие рамки),
    /// 1 — сплошная, 2 — точечная, 3 — двойная, 4 — редкий пунктир,
    /// 5 — частый пунктир, 6 — большой пунктир.
    pub kind: u32,
    /// Толщина линии (обычно 1–3); 0 — линия невидима.
    pub thickness: u32,
}

/// Цвет из таблицы цветов документа.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpec {
    /// Абсолютный цвет RGB
    Rgb(u8, u8, u8),
    /// Автоматический/стилевой цвет (в файле — служебная ссылка)
    Auto,
}

#[derive(Debug, Clone)]
pub struct Cell {
    /// Индекс формата, 1-базный; 0 = формата нет
    pub format: u32,
    pub text: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub index: u32,
    /// Индекс формата строки, 1-базный; 0 = формата нет
    pub format: u32,
    /// (номер колонки, ячейка), по возрастанию номера
    pub cells: Vec<(u32, Cell)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Merge {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

/// Группа колонок: у разных диапазонов строк могут быть разные ширины.
#[derive(Debug, Default, Clone)]
pub struct ColumnGroup {
    /// (номер колонки, индекс формата колонки, 1-базный)
    pub columns: Vec<(u32, u32)>,
}

/// Вид графического объекта табличного документа.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawingKind {
    /// Прямоугольник (тип 2)
    Rectangle,
    /// Текстовая надпись (тип 3)
    Text,
    /// Растровая картинка (тип 5)
    Picture,
    /// Прочее (диаграмма, OLE и т.п.) — не отрисовывается
    Other,
}

/// Графический объект (рисунок) листа.
#[derive(Debug, Clone)]
pub struct Drawing {
    pub kind: DrawingKind,
    /// Ячейка-якорь начала: (колонка, строка)
    pub col_start: u32,
    pub row_start: u32,
    /// Ячейка-якорь конца (включительно, как в классическом формате)
    pub col_end: u32,
    pub row_end: u32,
    /// Смещения от якорей: (слева, сверху, справа, снизу) — в единицах
    /// ширины (1/8 символа) по X и высоты (1/288") по Y
    pub offsets: (u32, u32, u32, u32),
    /// Индекс формата, 1-базный (рамки/шрифт объекта)
    pub format: u32,
    /// Текст надписи (для текстовых объектов)
    pub text: Option<String>,
    /// 1-базный индекс в таблице картинок (для типа «картинка»)
    pub picture: Option<u32>,
}

/// Область печати: границы включительно, 0-базные; `None` — граница не
/// ограничена (область вида «строки» или «колонки»).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PrintArea {
    pub left: Option<u32>,
    pub top: Option<u32>,
    pub right: Option<u32>,
    pub bottom: Option<u32>,
}

/// Признак разрыва страницы в формате строки/колонки (значение поля
/// «другого измерения»: ширины для строки, высоты для колонки).
const PAGE_BREAK_FLAG: u32 = 4;

#[derive(Debug, Default)]
pub struct Document {
    /// Область печати документа
    pub print_area: Option<PrintArea>,
    pub rows: Vec<Row>,
    /// Таблица форматов; индексы из ячеек 1-базные: ячейка с format = i
    /// ссылается на formats[i-1]
    pub formats: Vec<CellFormat>,
    pub fonts: Vec<FontSpec>,
    pub default_group: ColumnGroup,
    pub extra_groups: Vec<ColumnGroup>,
    /// строка → индекс в extra_groups
    pub row_groups: Vec<(u32, u32)>,
    pub merges: Vec<Merge>,
    /// Таблица цветов документа; индексы из форматов 0-базные
    pub colors: Vec<ColorSpec>,
    /// Таблица линий рамок документа; индексы из форматов 0-базные
    pub lines: Vec<LineSpec>,
    /// Графические объекты листа
    pub drawings: Vec<Drawing>,
    /// Таблица картинок: декодированные данные (PNG/JPEG и т.п.);
    /// ссылки из рисунков 1-базные
    pub pictures: Vec<Vec<u8>>,
}

impl Document {
    /// Формат по 1-базному индексу из файла (0 → None).
    pub fn format(&self, index_1based: u32) -> Option<&CellFormat> {
        if index_1based == 0 {
            None
        } else {
            self.formats.get(index_1based as usize - 1)
        }
    }

    /// Линия рамки по 0-базному индексу таблицы линий.
    pub fn line(&self, index: u32) -> Option<LineSpec> {
        self.lines.get(index as usize).copied()
    }

    /// RGB-цвет по индексу таблицы цветов; `None` для авто/неизвестных.
    pub fn color_rgb(&self, index: u32) -> Option<(u8, u8, u8)> {
        match self.colors.get(index as usize) {
            Some(ColorSpec::Rgb(r, g, b)) => Some((*r, *g, *b)),
            _ => None,
        }
    }

    /// Горизонтальные разрывы страниц: номера строк (0-базные), после
    /// которых начинается новая страница. В файле разрыв хранится в формате
    /// строки как значение поля ширины с битом 4.
    pub fn row_breaks(&self) -> Vec<u32> {
        let mut breaks: Vec<u32> = self
            .rows
            .iter()
            .filter(|r| {
                self.format(r.format)
                    .and_then(|f| f.width)
                    .is_some_and(|w| w & PAGE_BREAK_FLAG != 0)
            })
            .map(|r| r.index)
            .collect();
        breaks.sort_unstable();
        breaks.dedup();
        breaks
    }

    /// Вертикальные разрывы страниц: номера колонок (0-базные), после
    /// которых начинается новая страница (поле высоты в формате колонки
    /// группы по умолчанию с битом 4).
    pub fn column_breaks(&self) -> Vec<u32> {
        let mut breaks: Vec<u32> = self
            .default_group
            .columns
            .iter()
            .filter(|(_, fmt)| {
                self.format(*fmt)
                    .and_then(|f| f.height)
                    .is_some_and(|h| h & PAGE_BREAK_FLAG != 0)
            })
            .map(|(col, _)| *col)
            .collect();
        breaks.sort_unstable();
        breaks.dedup();
        breaks
    }

    /// Группа колонок, действующая для строки.
    pub fn group_for_row(&self, row: u32) -> &ColumnGroup {
        for &(r, g) in &self.row_groups {
            if r == row {
                if let Some(group) = self.extra_groups.get(g as usize) {
                    return group;
                }
            }
        }
        &self.default_group
    }
}

// ---------------------------------------------------------------------------
// Заголовок файла
// ---------------------------------------------------------------------------

const SIGNATURE: &[u8] = b"MOXCEL";
const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// Проверяет заголовок, возвращает текстовое тело (после BOM).
fn check_header(data: &[u8]) -> Result<&[u8], MxlError> {
    if data.len() < 16 || &data[0..6] != SIGNATURE {
        return err("файл не является табличным документом MXL (нет сигнатуры MOXCEL)");
    }
    // Классический бинарный MOXCEL (1С 7.7): байты 6..11 — нули,
    // байт 11 — версия 6 или 7.
    if data[6..11] == [0, 0, 0, 0, 0] && (data[11] == 6 || data[11] == 7) {
        return err("файл сохранён в формате 1С 7.7 (бинарный MOXCEL) — \
             формат 1С 7.7 не поддерживается");
    }
    let word = |off: usize| u16::from_le_bytes([data[off], data[off + 1]]);
    if data[6] != 0 || word(7) != 8 || word(9) != 1 {
        return err("неизвестный вариант формата MXL (ожидался MXL8 версии 8.1.x)");
    }
    if &data[13..16] != BOM {
        return err("повреждённый файл MXL8: не найден BOM UTF-8 после заголовка");
    }
    Ok(&data[16..])
}

// ---------------------------------------------------------------------------
// Схема MXL8
// ---------------------------------------------------------------------------

/// Индекс поля корневого объекта, с которого начинается блок строк
/// (подтверждено на всех проверенных файлах v8.1.9 и v8.1.12).
const ROWS_START_INDEX: usize = 15;

/// Извлекает текст ячейки из объекта-ячейки `{маркер, формат, ...}`.
fn cell_from_value(fields: &[Value]) -> Option<Cell> {
    if fields.len() < 2 {
        return None;
    }
    let marker = fields[0].as_num()?;
    let format = fields[1].as_num().unwrap_or(0).max(0) as u32;
    let mut text = None;
    // после формата: [ссылка если маркер&4], [текст если маркер&16]
    let mut next = 2usize;
    if marker & 2 != 0 {
        // типизированное значение: {"S", "текст"}
        if let Some(tv) = fields.get(next).and_then(Value::as_list) {
            if tv.len() >= 2 && tv[0].as_str() == Some("S") {
                text = tv[1].as_str().map(str::to_string);
            }
            next += 1;
        }
    }
    if marker & 4 != 0 {
        next += 1; // ссылка-расшифровка, сам текст берём из кэша ниже
    }
    if marker & 16 != 0 {
        // локализованная строка: {n, есть_текст, {язык, текст}}
        if let Some(loc) = fields.get(next).and_then(Value::as_list) {
            if loc.len() >= 3 && loc[1].as_num() == Some(1) {
                if let Some(pair) = loc[2].as_list() {
                    if pair.len() >= 2 {
                        text = pair[1].as_str().map(str::to_string);
                    }
                }
            }
        }
    }
    Some(Cell { format, text })
}

/// Строгий разбор блока строк начиная с поля `start`.
/// Возвращает (строки, индекс первого поля после блока).
fn parse_rows(root: &[Value], start: usize) -> Result<(Vec<Row>, usize), MxlError> {
    let mut i = start;
    let num_at = |i: usize| -> Result<i64, MxlError> {
        root.get(i)
            .and_then(Value::as_num)
            .ok_or_else(|| MxlError(format!("MXL8: ожидалось число в поле {i} блока строк")))
    };
    let nrows = num_at(i)?;
    if !(0..=1_000_000).contains(&nrows) {
        return err(format!("MXL8: неправдоподобное число строк {nrows}"));
    }
    i += 1;
    let mut rows = Vec::with_capacity(nrows as usize);
    let mut prev: i64 = -1;
    for _ in 0..nrows {
        let index = num_at(i)?;
        let format = num_at(i + 1)?;
        let ncells = num_at(i + 2)?;
        if index <= prev || index < 0 {
            return err("MXL8: номера строк не возрастают — файл повреждён");
        }
        if index > u32::MAX as i64 {
            return err(format!("MXL8: недопустимый номер строки {index}"));
        }
        if !(0..=100_000).contains(&ncells) {
            return err(format!("MXL8: неправдоподобное число ячеек {ncells}"));
        }
        prev = index;
        i += 3;
        let mut cells = Vec::with_capacity(ncells as usize);
        for _ in 0..ncells {
            let col = num_at(i)?;
            let cell_fields = root
                .get(i + 1)
                .and_then(Value::as_list)
                .ok_or_else(|| MxlError("MXL8: ожидался объект ячейки".to_string()))?;
            if !(0..=u32::MAX as i64).contains(&col) {
                return err(format!("MXL8: недопустимый номер колонки {col}"));
            }
            let cell = cell_from_value(cell_fields).unwrap_or(Cell {
                format: 0,
                text: None,
            });
            cells.push((col as u32, cell));
            i += 2;
        }
        rows.push(Row {
            index: index as u32,
            format: format.max(0) as u32,
            cells,
        });
    }
    Ok((rows, i))
}

/// Группа колонок: `{a, b, GUID, n, (колонка, формат) × n}`.
fn column_group_from_value(v: &Value) -> Option<ColumnGroup> {
    let fields = v.as_list()?;
    if fields.len() < 4 {
        return None;
    }
    fields[0].as_num()?;
    let n = fields[3].as_num()?;
    if n < 0 || fields.len() != 4 + 2 * n as usize {
        return None;
    }
    let mut columns = Vec::with_capacity(n as usize);
    for k in 0..n as usize {
        let col = fields[4 + 2 * k].as_num()?;
        let fmt = fields[5 + 2 * k].as_num()?;
        if !(0..=u32::MAX as i64).contains(&col) || !(0..=u32::MAX as i64).contains(&fmt) {
            return None;
        }
        columns.push((col as u32, fmt as u32));
    }
    Some(ColumnGroup { columns })
}

/// Запись формата: `{флаги, по одному значению на установленный бит}`.
/// Используются биты 0–18; неизвестные старшие биты тоже занимают по одному
/// значению каждый, поэтому младшие поля читаются корректно и при их наличии.
fn format_from_value(fields: &[Value]) -> CellFormat {
    let mut fmt = CellFormat::default();
    let flags = match fields.first().and_then(Value::as_num) {
        Some(f) if f >= 0 => f as u64,
        _ => return fmt,
    };
    let mut vi = 1usize; // индекс очередного значения
    for bit in 0..40u32 {
        if flags >> bit & 1 == 0 {
            continue;
        }
        let value = fields.get(vi).and_then(Value::as_num);
        vi += 1;
        let Some(v) = value else { continue };
        if v < 0 {
            continue;
        }
        let v = v as u32;
        match bit {
            0 => fmt.font = Some(v),
            1 => fmt.border_left = Some(v),
            2 => fmt.border_top = Some(v),
            3 => fmt.border_right = Some(v),
            4 => fmt.border_bottom = Some(v),
            5 => fmt.borders_color = Some(v),
            6 => fmt.height = Some(v),
            7 => fmt.width = Some(v),
            8 => fmt.h_align = Some(v),
            9 => fmt.v_align = Some(v),
            10 => fmt.text_color = Some(v),
            11 => fmt.bg_color = Some(v),
            12 => fmt.pattern = Some(v),
            13 => fmt.pattern_color = Some(v),
            14 => fmt.text_control = Some(v),
            18 => fmt.orientation = Some(v),
            _ => {}
        }
    }
    fmt
}

/// Флаги переопределений шрифта (как в референсном парсере Yoksel).
mod font_flags {
    pub const FAMILY: i64 = 1;
    pub const HEIGHT: i64 = 2;
    pub const WEIGHT: i64 = 4;
    pub const ITALIC: i64 = 8;
    pub const UNDERLINE: i64 = 16;
    pub const STRIKEOUT: i64 = 32;
}

/// Запись шрифта: `{7, вид, ...}`. Вид 0 — полная фиксированная запись,
/// виды 1–3 — переопределение по маске флагов (после флагов может идти
/// вложенный объект-«база», его пропускаем).
fn font_from_value(fields: &[Value]) -> FontSpec {
    let mut font = FontSpec::default();
    if fields.len() < 3 || !matches!(fields[0].as_num(), Some(6) | Some(7)) {
        return font;
    }
    let kind = fields[1].as_num().unwrap_or(0);
    if kind == 0 {
        // {7,0, флаги, высота, 0,0,0, насыщенность, курсив, подчёркивание, ...}
        let num = |i: usize| fields.get(i).and_then(Value::as_num).unwrap_or(0);
        if num(3) > 0 {
            font.size_tenth_pt = num(3) as u32;
        }
        font.bold = num(7) >= 600;
        font.italic = num(8) != 0;
        font.underline = num(9) != 0;
        font.strikeout = num(10) != 0;
        // имя шрифта — единственная строка записи (после служебных чисел)
        if let Some(name) = fields.iter().skip(3).find_map(Value::as_str) {
            font.family = FontFamily::from_name(name);
        }
        return font;
    }
    // виды 1-3: {7, вид, флаги, [{база}], значения по флагам...}
    let flags = fields[2].as_num().unwrap_or(0);
    let mut vi = 3usize;
    if matches!(fields.get(vi), Some(Value::List(_))) {
        vi += 1;
    }
    let mut take = |present: bool| -> Option<&Value> {
        if !present {
            return None;
        }
        let v = fields.get(vi);
        vi += 1;
        v
    };
    if let Some(name) = take(flags & font_flags::FAMILY != 0).and_then(Value::as_str) {
        font.family = FontFamily::from_name(name);
    }
    let mut take_num = |present: bool| take(present).and_then(Value::as_num);
    if let Some(h) = take_num(flags & font_flags::HEIGHT != 0) {
        if h > 0 {
            font.size_tenth_pt = h as u32;
        }
    }
    if let Some(w) = take_num(flags & font_flags::WEIGHT != 0) {
        font.bold = w >= 600;
    }
    if let Some(v) = take_num(flags & font_flags::ITALIC != 0) {
        font.italic = v != 0;
    }
    if let Some(v) = take_num(flags & font_flags::UNDERLINE != 0) {
        font.underline = v != 0;
    }
    if let Some(v) = take_num(flags & font_flags::STRIKEOUT != 0) {
        font.strikeout = v != 0;
    }
    font
}

/// Запись таблицы цветов: `{3, вид, {значение}}`. Вид 0 — абсолютный цвет,
/// значение — COLORREF (R | G<<8 | B<<16); прочие виды — авто/стилевые.
fn color_from_value(v: &Value) -> Option<ColorSpec> {
    let fields = v.as_list()?;
    if fields.len() < 3 || fields[0].as_num() != Some(3) {
        return None;
    }
    let kind = fields[1].as_num()?;
    let value = fields[2].as_list()?.first()?.as_num()?;
    if kind == 0 && (0..=0xFF_FF_FF).contains(&value) {
        let v = value as u32;
        Some(ColorSpec::Rgb(
            (v & 0xFF) as u8,
            (v >> 8 & 0xFF) as u8,
            (v >> 16 & 0xFF) as u8,
        ))
    } else {
        Some(ColorSpec::Auto)
    }
}

/// Ищет таблицу цветов: `N, {3, вид, {значение}} × N` (идёт после таблицы
/// шрифтов среди служебных полей хвоста). Возвращает также индекс первого
/// поля после таблицы (оттуда ищется таблица картинок).
fn find_color_table(root: &[Value], start: usize) -> Option<(Vec<ColorSpec>, usize)> {
    let n_fields = root.len();
    let mut i = start;
    while i + 1 < n_fields {
        if let Some(n) = root[i].as_num() {
            let n = n as usize;
            if n > 0 && n <= 10_000 && i + n < n_fields {
                let colors: Vec<ColorSpec> = root[i + 1..i + 1 + n]
                    .iter()
                    .map_while(color_from_value)
                    .collect();
                if colors.len() == n {
                    return Some((colors, i + 1 + n));
                }
            }
        }
        i += 1;
    }
    None
}

/// Запись линии из таблицы линий: `{4, 0, {..}, тип, толщина, ...}`.
fn line_from_record(rec: &[Value]) -> Option<LineSpec> {
    if rec.len() < 5 || rec[0].as_num() != Some(4) || rec[1].as_num() != Some(0) {
        return None;
    }
    let kind = rec[3].as_num()?.max(0) as u32;
    let thickness = rec[4].as_num()?.max(0) as u32;
    Some(LineSpec { kind, thickness })
}

/// Ищет таблицу линий в преамбуле документа. Форма поля:
/// `{N, (1, {запись-линии}, 0) × N}`, где N — число линий, а каждая запись —
/// объект с тегом 4 (см. [`line_from_record`]). Индексы рамок в форматах —
/// 0-базные номера линий этой таблицы.
fn find_line_table(root: &[Value]) -> Vec<LineSpec> {
    for f in root {
        let Value::List(items) = f else { continue };
        let Some(n) = items.first().and_then(Value::as_num) else {
            continue;
        };
        if !(1..=100_000).contains(&n) || items.len() != 1 + 3 * n as usize {
            continue;
        }
        let mut lines = Vec::with_capacity(n as usize);
        let mut ok = true;
        for k in 0..n as usize {
            let rec = items.get(2 + 3 * k).and_then(Value::as_list);
            match rec.and_then(line_from_record) {
                Some(l) => lines.push(l),
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok && !lines.is_empty() {
            return lines;
        }
    }
    Vec::new()
}

/// Графический объект: `{ячейка-формат, тип, колН, стрН, смЛ, смВ, колК,
/// стрК, смП, смН, z-номер, [доп. поля по типу]}`. Для картинки (тип 5)
/// первое доп. поле — 1-базный индекс в таблице картинок.
fn drawing_from_value(v: &Value) -> Option<Drawing> {
    let fields = v.as_list()?;
    if fields.len() < 11 {
        return None;
    }
    let cell_like = fields[0].as_list()?;
    let cell = cell_from_value(cell_like)?;
    let kind_num = fields[1].as_num()?;
    let kind = match kind_num {
        2 => DrawingKind::Rectangle,
        3 => DrawingKind::Text,
        5 => DrawingKind::Picture,
        _ => DrawingKind::Other,
    };
    let num = |i: usize| -> Option<u32> {
        fields
            .get(i)
            .and_then(Value::as_num)
            .map(|n| n.max(0) as u32)
    };
    Some(Drawing {
        kind,
        col_start: num(2)?,
        row_start: num(3)?,
        offsets: (num(4)?, num(5)?, num(8)?, num(9)?),
        col_end: num(6)?,
        row_end: num(7)?,
        format: cell.format,
        text: cell.text,
        picture: if kind == DrawingKind::Picture {
            num(11)
        } else {
            None
        },
    })
}

/// Декодер base64 (стандартный алфавит, `=`-паддинг; переносы строк
/// допустимы). Небольшой собственный, чтобы не тянуть зависимость.
fn decode_base64(input: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for &b in input.as_bytes() {
        if b == b'\r' || b == b'\n' || b == b'=' || b == b' ' {
            continue;
        }
        acc = (acc << 6) | val(b)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Извлекает данные картинки из записи таблицы картинок: внутри записи —
/// вложенный объект, все элементы которого — «голые» токены, первый
/// начинается с `#base64:`.
fn picture_from_value(v: &Value) -> Option<Vec<u8>> {
    let fields = v.as_list()?;
    for f in fields {
        if let Value::List(items) = f {
            // сам блок данных может быть обёрнут ещё одним уровнем
            let atoms: Option<Vec<&str>> = items
                .iter()
                .map(|x| match x {
                    Value::Atom(s) => Some(s.as_str()),
                    _ => None,
                })
                .collect();
            if let Some(atoms) = atoms {
                if let Some(first) = atoms.first() {
                    if let Some(stripped) = first.strip_prefix("#base64:") {
                        let mut joined = String::from(stripped);
                        for a in &atoms[1..] {
                            joined.push_str(a);
                        }
                        return decode_base64(&joined);
                    }
                }
            }
            if let Some(data) = picture_from_value(f) {
                return Some(data);
            }
        }
    }
    None
}

/// Ищет таблицу картинок после таблицы цветов: `N, запись × N`, где хотя бы
/// одна запись содержит блок `#base64:`.
fn find_picture_table(root: &[Value], start: usize) -> Option<Vec<Vec<u8>>> {
    let n_fields = root.len();
    let mut i = start;
    while i + 1 < n_fields {
        if let Some(n) = root[i].as_num() {
            let n = n as usize;
            if n > 0 && n <= 1000 && i + n < n_fields {
                let entries = &root[i + 1..i + 1 + n];
                if entries.iter().all(|v| matches!(v, Value::List(_)))
                    && entries.iter().any(|v| picture_from_value(v).is_some())
                {
                    return Some(
                        entries
                            .iter()
                            .map(|v| picture_from_value(v).unwrap_or_default())
                            .collect(),
                    );
                }
            }
        }
        i += 1;
    }
    None
}

/// Область печати: `{тип, лево, верх, право, низ, GUID}`; тип 0 — область не
/// задана, отрицательная граница — граница не ограничена. Возвращает `None`,
/// если объект не той формы, и `Some(None)` для незаданной области.
fn print_area_from_value(v: &Value) -> Option<Option<PrintArea>> {
    let fields = v.as_list()?;
    if fields.len() != 6 {
        return None;
    }
    let guid = match &fields[5] {
        Value::Atom(a) => a,
        _ => return None,
    };
    if guid.len() != 36 || guid.matches('-').count() != 4 {
        return None;
    }
    let kind = fields[0].as_num()?;
    let mut nums = [0i64; 4];
    for (dst, src) in nums.iter_mut().zip(&fields[1..5]) {
        *dst = src.as_num()?;
    }
    if !(0..=3).contains(&kind) {
        return None;
    }
    if kind == 0 {
        return Some(None);
    }
    let bound = |n: i64| u32::try_from(n).ok();
    let [l, t, r, b] = nums.map(bound);
    if matches!((l, r), (Some(l), Some(r)) if r < l)
        || matches!((t, b), (Some(t), Some(b)) if b < t)
    {
        return Some(None);
    }
    Some(Some(PrintArea {
        left: l,
        top: t,
        right: r,
        bottom: b,
    }))
}

/// Объединения: объект вида `{n, {л,в,п,н,флаг} × n}`; берём записи с флагом 0.
fn merges_from_value(v: &Value) -> Option<Vec<Merge>> {
    let fields = v.as_list()?;
    let n = fields.first()?.as_num()?;
    if n <= 0 || fields.len() != n as usize + 1 {
        return None;
    }
    let mut merges = Vec::new();
    for item in &fields[1..] {
        let rect = item.as_list()?;
        if rect.len() != 5 {
            return None;
        }
        let mut nums = [0i64; 5];
        for (dst, src) in nums.iter_mut().zip(rect) {
            *dst = src.as_num()?;
        }
        let [l, t, r, b, flag] = nums;
        // r и b не меньше l и t, поэтому достаточно проверить их предел
        if flag != 0
            || l < 0
            || t < 0
            || r < l
            || b < t
            || r > u32::MAX as i64
            || b > u32::MAX as i64
        {
            continue;
        }
        merges.push(Merge {
            left: l as u32,
            top: t as u32,
            right: r as u32,
            bottom: b as u32,
        });
    }
    Some(merges)
}

/// Ищет в хвосте документа таблицу форматов и сразу за ней таблицу шрифтов:
/// `N, {формат} × N, M, {7|6, ...} × M`. Возвращает также индекс первого
/// поля после таблицы шрифтов (оттуда ищется таблица цветов).
fn find_format_and_font_tables(
    root: &[Value],
    start: usize,
) -> Option<(Vec<CellFormat>, Vec<FontSpec>, usize)> {
    let n_fields = root.len();
    let mut i = start;
    while i + 1 < n_fields {
        let Some(n) = root[i].as_num() else {
            i += 1;
            continue;
        };
        let n = n as usize;
        if n == 0 || n > 10_000 || i + n + 1 >= n_fields {
            i += 1;
            continue;
        }
        let entries_ok = root[i + 1..i + 1 + n].iter().all(
            |v| matches!(v, Value::List(items) if items.first().and_then(Value::as_num).is_some()),
        );
        if entries_ok {
            let j = i + n + 1;
            if let Some(m) = root[j].as_num() {
                let m = m as usize;
                if m > 0 && m <= 1000 && j + m < n_fields {
                    let fonts_ok = root[j + 1..j + 1 + m].iter().all(|v| {
                        matches!(v, Value::List(items)
                            if matches!(items.first().and_then(Value::as_num), Some(6) | Some(7)))
                    });
                    if fonts_ok {
                        let formats = root[i + 1..i + 1 + n]
                            .iter()
                            .map(|v| format_from_value(v.as_list().unwrap_or(&[])))
                            .collect();
                        let fonts = root[j + 1..j + 1 + m]
                            .iter()
                            .map(|v| font_from_value(v.as_list().unwrap_or(&[])))
                            .collect();
                        return Some((formats, fonts, j + 1 + m));
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// Разбор хвоста после блока строк: группы колонок, карта строка→группа,
/// объединения. Всё — терпимо к отклонениям (при несовпадении формы блок
/// просто пропускается).
fn parse_tail(root: &[Value], rows_end: usize, doc: &mut Document) {
    let mut i = rows_end;
    // группа колонок по умолчанию
    if let Some(group) = root.get(i).and_then(column_group_from_value) {
        doc.default_group = group;
        i += 1;
        // число строк (дублирует блок строк)
        if root.get(i).and_then(Value::as_num).is_some() {
            i += 1;
        }
        // дополнительные группы
        if let Some(extra) = root.get(i).and_then(Value::as_num) {
            if (0..=1000).contains(&extra) {
                let mut groups = Vec::new();
                let mut ok = true;
                for k in 0..extra as usize {
                    match root.get(i + 1 + k).and_then(column_group_from_value) {
                        Some(g) => groups.push(g),
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    doc.extra_groups = groups;
                    i += 1 + extra as usize;
                    // карта строка → группа
                    if let Some(npairs) = root.get(i).and_then(Value::as_num) {
                        if (0..=1_000_000).contains(&npairs) {
                            let need = 2 * npairs as usize;
                            let pairs: Vec<i64> = root[i + 1..]
                                .iter()
                                .take(need)
                                .filter_map(Value::as_num)
                                .collect();
                            if pairs.len() == need {
                                doc.row_groups = pairs
                                    .chunks(2)
                                    .filter(|p| p[0] >= 0 && p[1] >= 0)
                                    .map(|p| (p[0] as u32, p[1] as u32))
                                    .collect();
                                i += 1 + need;
                            }
                        }
                    }
                }
            }
        }
    }
    // графические объекты: `Число, Число N, N записей` сразу после карты
    // групп (перед объединениями); форма записи проверяется строго
    if root.get(i).and_then(Value::as_num).is_some() {
        if let Some(n) = root.get(i + 1).and_then(Value::as_num) {
            if (1..=10_000).contains(&n) {
                let n = n as usize;
                let entries: Vec<Drawing> = root
                    .iter()
                    .skip(i + 2)
                    .take(n)
                    .map_while(drawing_from_value)
                    .collect();
                if entries.len() == n {
                    doc.drawings = entries;
                    i += 2 + n;
                }
            }
        }
    }
    // объединения: первый подходящий по форме объект в ближайших полях
    for value in root.iter().skip(i) {
        if let Some(merges) = merges_from_value(value) {
            doc.merges = merges;
            break;
        }
    }
    // область печати: первый объект вида {тип, л, в, п, н, GUID} в хвосте
    doc.print_area = root
        .iter()
        .skip(i)
        .find_map(print_area_from_value)
        .flatten();
    if let Some((formats, fonts, fonts_end)) = find_format_and_font_tables(root, i) {
        doc.formats = formats;
        doc.fonts = fonts;
        // таблица цветов идёт после таблицы шрифтов (через таблицу числовых
        // форматов и пару служебных полей)
        if let Some((colors, colors_end)) = find_color_table(root, fonts_end) {
            doc.colors = colors;
            // таблица картинок — после таблицы цветов
            if doc.drawings.iter().any(|d| d.kind == DrawingKind::Picture) {
                if let Some(pictures) = find_picture_table(root, colors_end) {
                    doc.pictures = pictures;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Точка входа
// ---------------------------------------------------------------------------

/// Предел размера входного файла: файл читается в память целиком, и дерево
/// значений занимает кратно больше самого файла.
pub const MAX_FILE_SIZE: u64 = 200 * 1024 * 1024;

/// Читает и разбирает файл *.mxl (формат 1С 8.х).
pub fn parse_file(path: &Path) -> Result<Document, MxlError> {
    let meta = std::fs::metadata(path)
        .map_err(|e| MxlError(format!("не удалось прочитать файл {}: {e}", path.display())))?;
    if meta.len() > MAX_FILE_SIZE {
        return err(format!(
            "файл {} слишком большой ({} МБ, предел {} МБ)",
            path.display(),
            meta.len() / 1024 / 1024,
            MAX_FILE_SIZE / 1024 / 1024
        ));
    }
    let data = std::fs::read(path)
        .map_err(|e| MxlError(format!("не удалось прочитать файл {}: {e}", path.display())))?;
    parse_bytes(&data)
}

/// Разбирает содержимое файла *.mxl.
pub fn parse_bytes(data: &[u8]) -> Result<Document, MxlError> {
    let body = check_header(data)?;
    let text = std::str::from_utf8(body)
        .map_err(|_| MxlError("повреждённый файл MXL8: тело не является UTF-8".to_string()))?;
    let top = parse_body(text)?;
    let root = top
        .first()
        .and_then(Value::as_list)
        .ok_or_else(|| MxlError("MXL8: не найден корневой объект документа".to_string()))?;
    if root.len() < ROWS_START_INDEX + 1 {
        return err("MXL8: корневой объект слишком короткий");
    }
    // блок строк: штатно — с поля 15; на случай иной версии преамбулы
    // пробуем соседние позиции (принимаем только непустой блок)
    let mut parsed = parse_rows(root, ROWS_START_INDEX).ok();
    if parsed.is_none() {
        for start in 3..40usize.min(root.len()) {
            if let Ok((rows, end)) = parse_rows(root, start) {
                if !rows.is_empty() {
                    parsed = Some((rows, end));
                    break;
                }
            }
        }
    }
    let (rows, rows_end) = parsed
        .ok_or_else(|| MxlError("MXL8: не удалось найти блок строк документа".to_string()))?;
    let mut doc = Document {
        rows,
        lines: find_line_table(root),
        ..Document::default()
    };
    parse_tail(root, rows_end, &mut doc);
    Ok(doc)
}

// ---------------------------------------------------------------------------
// Тесты
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Краткий конструктор линии для тестов.
    fn ls(kind: u32, thickness: u32) -> LineSpec {
        LineSpec { kind, thickness }
    }

    fn mxl_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../mxl-files")
    }

    /// Тестовые mxl-файлы не входят в репозиторий; без них тесты на
    /// реальных данных пропускаются (например, в CI).
    fn samples_available() -> bool {
        if mxl_dir().is_dir() {
            true
        } else {
            eprintln!("mxl-files/ отсутствует — тест на реальных файлах пропущен");
            false
        }
    }

    #[test]
    fn lexer_handles_strings_and_nesting() {
        let v = parse_body(r#"{8,1,9,{"a""b",-5},00000000-0000-0000-0000-000000000000}"#).unwrap();
        let root = v[0].as_list().unwrap();
        assert_eq!(root[0], Value::Num(8));
        let inner = root[3].as_list().unwrap();
        assert_eq!(inner[0], Value::Str("a\"b".into()));
        assert_eq!(inner[1], Value::Num(-5));
        assert!(matches!(root[4], Value::Atom(_)));
    }

    #[test]
    fn rejects_v77_binary_format() {
        // классический заголовок 1С 7.7: MOXCEL + 5 нулей + версия 7
        let mut data = Vec::new();
        data.extend_from_slice(b"MOXCEL");
        data.extend_from_slice(&[0, 0, 0, 0, 0, 7, 0]);
        data.extend_from_slice(&[0u8; 16]);
        let e = parse_bytes(&data).unwrap_err();
        assert!(e.0.contains("1С 7.7"), "{}", e.0);
    }

    #[test]
    fn rejects_non_mxl() {
        let e = parse_bytes(b"hello world, definitely not a spreadsheet").unwrap_err();
        assert!(e.0.contains("MOXCEL"), "{}", e.0);
    }

    #[test]
    fn parses_real_files_without_cell_loss() {
        if !samples_available() {
            return;
        }
        // (файл, строк, минимум ячеек с текстом, фрагмент ожидаемого текста)
        let cases = [
            (
                "Акт сверки взаиморасчетов № 12 от 18 декабря 2023 г..mxl",
                24usize,
                30usize,
                "Акт звіряння",
            ),
            ("124.mxl", 62, 100, "ТОВАРНО-ТРАНСПОРТНА НАКЛАДНА"),
            ("Сетка.mxl", 18, 100, "Номенклатура"),
        ];
        for (name, nrows, min_texts, fragment) in cases {
            let path = mxl_dir().join(name);
            let doc = parse_file(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(doc.rows.len(), nrows, "{name}: число строк");
            let texts: Vec<&str> = doc
                .rows
                .iter()
                .flat_map(|r| r.cells.iter())
                .filter_map(|(_, c)| c.text.as_deref())
                .collect();
            assert!(
                texts.len() >= min_texts,
                "{name}: слишком мало текстов ({})",
                texts.len()
            );
            assert!(
                texts.iter().any(|t| t.contains(fragment)),
                "{name}: не найден текст «{fragment}»"
            );
            assert!(!doc.formats.is_empty(), "{name}: пустая таблица форматов");
            assert!(!doc.fonts.is_empty(), "{name}: пустая таблица шрифтов");
            assert!(
                !doc.default_group.columns.is_empty(),
                "{name}: нет ширин колонок"
            );
        }
    }

    #[test]
    fn parses_all_sample_files() {
        if !samples_available() {
            return;
        }
        let dir = mxl_dir();
        let mut checked = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("mxl") {
                continue;
            }
            let doc = parse_file(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert!(!doc.rows.is_empty(), "{}: нет строк", path.display());
            checked += 1;
        }
        assert!(checked >= 30, "найдено слишком мало файлов: {checked}");
    }

    #[test]
    fn decodes_demo_colors_and_styles() {
        if !samples_available() {
            return;
        }
        let path = mxl_dir().join("ДемонстрацияВозможностейТабличногоДокумента.mxl");
        if !path.is_file() {
            eprintln!("демо-файл отсутствует — тест пропущен");
            return;
        }
        let doc = parse_file(&path).unwrap();
        assert_eq!(doc.rows.len(), 30);
        assert!(doc.colors.len() >= 70, "цветов: {}", doc.colors.len());
        // титул (строка 0): формат 2 → цвет текста белый, фон (31,78,121)
        let title = doc.rows.iter().find(|r| r.index == 0).unwrap();
        let title_fmt = doc.format(title.cells[0].1.format).unwrap();
        assert_eq!(
            doc.color_rgb(title_fmt.text_color.unwrap()),
            Some((255, 255, 255))
        );
        assert_eq!(
            doc.color_rgb(title_fmt.bg_color.unwrap()),
            Some((31, 78, 121))
        );
        // первая ячейка палитры (строка 4, колонка 0): фон #FF0000
        let palette = doc.rows.iter().find(|r| r.index == 4).unwrap();
        let red_fmt = doc.format(palette.cells[0].1.format).unwrap();
        assert_eq!(doc.color_rgb(red_fmt.bg_color.unwrap()), Some((255, 0, 0)));
        // вертикальный текст (строка 27): ориентация 900
        let row27 = doc.rows.iter().find(|r| r.index == 27).unwrap();
        let vertical = row27
            .cells
            .iter()
            .filter_map(|(_, c)| doc.format(c.format))
            .find(|f| f.orientation == Some(900));
        assert!(vertical.is_some(), "не найден формат с ориентацией 900");
        // рамки (строка 30): стили 0..=6 по порядку колонок
        let row30 = doc.rows.iter().find(|r| r.index == 30).unwrap();
        let styles: Vec<u32> = row30
            .cells
            .iter()
            .filter_map(|(_, c)| doc.format(c.format))
            .filter_map(|f| f.border_left)
            .collect();
        assert_eq!(styles, vec![0, 1, 2, 3, 4, 5, 6, 3]);
        // таблица линий: индексы рамок разрешаются в (тип, толщина).
        // порядок первого использования: сплошная 1/2/3, двойная, редкий
        // пунктир, точечная, частый пунктир
        assert_eq!(doc.line(0), Some(ls(1, 1)));
        assert_eq!(doc.line(2), Some(ls(1, 3)));
        assert_eq!(doc.line(3), Some(ls(3, 1)));
        assert_eq!(doc.line(5), Some(ls(2, 1)));
        // зачёркнутый шрифт присутствует
        assert!(doc.fonts.iter().any(|f| f.strikeout));
        // высота строки 0: ВысотаСтроки=30 → 120 единиц (по 1/4 пункта)
        assert_eq!(doc.format(title.format).unwrap().height, Some(120));
    }

    #[test]
    fn decodes_patterns_and_drawings_demo() {
        if !samples_available() {
            return;
        }
        let path = mxl_dir().join("ДемонстрацияУзоровРисунковБитов15-32.mxl");
        if !path.is_file() {
            eprintln!("демо-файл узоров отсутствует — тест пропущен");
            return;
        }
        let doc = parse_file(&path).unwrap();
        // 19 узорных ячеек: БезУзора=255, Сплошной=0, Узор1..17 = 1..17
        let mut patterns = Vec::new();
        for row in &doc.rows {
            if (4..=7).contains(&row.index) {
                for (_, cell) in &row.cells {
                    let fmt = doc.format(cell.format).unwrap();
                    patterns.push(fmt.pattern.unwrap());
                    assert!(fmt.pattern_color.is_some());
                }
            }
        }
        let expected: Vec<u32> = std::iter::once(255)
            .chain(std::iter::once(0))
            .chain(1..=17)
            .collect();
        assert_eq!(patterns, expected);
        // рисунки: картинка, текст, прямоугольник, диаграмма (прочее)
        assert_eq!(doc.drawings.len(), 4);
        let kinds: Vec<DrawingKind> = doc.drawings.iter().map(|d| d.kind).collect();
        assert_eq!(
            kinds,
            vec![
                DrawingKind::Picture,
                DrawingKind::Text,
                DrawingKind::Rectangle,
                DrawingKind::Other
            ]
        );
        // встроенная PNG-картинка декодирована из base64
        assert_eq!(doc.pictures.len(), 1);
        assert_eq!(&doc.pictures[0][..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(doc.drawings[0].picture, Some(1));
        assert!(doc.drawings[1]
            .text
            .as_deref()
            .unwrap()
            .contains("Текстовый"));
    }

    #[test]
    fn decodes_akt_structure() {
        if !samples_available() {
            return;
        }
        let doc =
            parse_file(&mxl_dir().join("Акт сверки взаиморасчетов № 12 от 18 декабря 2023 г..mxl"))
                .unwrap();
        // заголовок «Акт звіряння»: формат 3 → запись 2: шрифт 1 (14pt bold),
        // выравнивание по центру
        let title_fmt = doc.format(3).unwrap();
        assert_eq!(title_fmt.h_align, Some(6));
        let title_font = &doc.fonts[title_fmt.font.unwrap() as usize];
        assert_eq!(title_font.size_tenth_pt, 140);
        assert!(title_font.bold);
        // строка 3 — с явной высотой 72 (1/288"); строка 2 — авто (высота 0)
        let row3 = doc.rows.iter().find(|r| r.index == 3).unwrap();
        assert_eq!(doc.format(row3.format).unwrap().height, Some(72));
        let row2 = doc.rows.iter().find(|r| r.index == 2).unwrap();
        assert_eq!(doc.format(row2.format).unwrap().height, Some(0));
        // колонка 1 («Дата», 8 символов) — ширина 64/8
        let group = doc.group_for_row(9);
        let col1_fmt_idx = group.columns.iter().find(|(c, _)| *c == 1).unwrap().1;
        assert_eq!(doc.format(col1_fmt_idx).unwrap().width, Some(64));
        // объединения найдены
        assert!(!doc.merges.is_empty());
    }

    fn demo(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../mxl-demo-collection")
            .join(name)
    }

    #[test]
    fn font_family_by_name() {
        for (name, family) in [
            ("Arial", FontFamily::Sans),
            ("Tahoma", FontFamily::Sans),
            ("Microsoft Sans Serif", FontFamily::Sans),
            ("Times New Roman", FontFamily::Serif),
            ("Georgia", FontFamily::Serif),
            ("PT Serif", FontFamily::Serif),
            ("Courier New", FontFamily::Mono),
            ("Consolas", FontFamily::Mono),
            ("Lucida Console", FontFamily::Mono),
        ] {
            assert_eq!(FontFamily::from_name(name), family, "{name}");
        }
    }

    #[test]
    fn parses_print_area_page_breaks_and_font_names() {
        let doc = parse_file(&demo("ДемонстрацияПечатиРазрывов_A4_Книжная.mxl")).unwrap();
        // ОбластьПечати R1C1:R49C12
        assert_eq!(
            doc.print_area,
            Some(PrintArea {
                left: Some(0),
                top: Some(0),
                right: Some(11),
                bottom: Some(48),
            })
        );
        // разрывы после строк 25 и 36 и после колонки 12 (1-базные)
        assert_eq!(doc.row_breaks(), vec![24, 35]);
        assert_eq!(doc.column_breaks(), vec![11]);
        // шрифт Consolas в таблице шрифтов — моноширинное семейство
        assert!(doc.fonts.iter().any(|f| f.family == FontFamily::Mono));
        assert!(doc.fonts.iter().any(|f| f.family == FontFamily::Sans));
    }

    #[test]
    fn documents_without_print_area_or_breaks() {
        let doc = parse_file(&demo("ДемонстрацияГраницОбъединений.mxl")).unwrap();
        assert_eq!(doc.print_area, None);
        assert!(doc.row_breaks().is_empty());
        assert!(doc.column_breaks().is_empty());
    }

    #[test]
    fn print_area_shapes() {
        let guid = || Value::Atom("00000000-0000-0000-0000-000000000000".to_string());
        let area = |items: [i64; 5]| {
            let mut v: Vec<Value> = items.iter().map(|&n| Value::Num(n)).collect();
            v.push(guid());
            print_area_from_value(&Value::List(v))
        };
        assert_eq!(area([0, -1, -1, -1, -1]), Some(None));
        assert_eq!(
            area([1, -1, 2, -1, 5]),
            Some(Some(PrintArea {
                left: None,
                top: Some(2),
                right: None,
                bottom: Some(5),
            }))
        );
        assert_eq!(area([3, 5, 0, 2, 0]), Some(None), "перевёрнутая область");
        assert_eq!(area([9, 0, 0, 1, 1]), None, "неизвестный тип");
        assert_eq!(print_area_from_value(&Value::Num(3)), None);
    }
}
