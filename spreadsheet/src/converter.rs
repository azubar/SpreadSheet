//! Рендер модели табличного документа (см. [`crate::mxl_parser`]) в растровую
//! картинку и кодирование в JPEG/PNG.
//!
//! Единицы измерения формата: ширина колонки — 1/8 ширины стандартного
//! символа (105 твипов), высота строки — 1/288 дюйма. Базовое разрешение
//! рендера — 96 DPI, т.е. 1 единица ширины = 7/8 px, 1 единица высоты = 1/3 px;
//! свойство «Масштаб» умножает обе величины.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::rc::Rc;

use ab_glyph::{Font, FontRef, GlyphId, PxScaleFont, ScaleFont};
use tiny_skia::{Paint, Pixmap, Rect, Transform};

use crate::mxl_parser::{
    Cell, CellFormat, Document, DrawingKind, FontFamily, FontSpec, LineSpec, Merge,
};

// Встроенные шрифты Liberation (лицензия SIL OFL, см. fonts/LICENSE) —
// метрически совместимы со шрифтами документов 1С: Sans — с Arial, Serif —
// с Times New Roman, Mono — с Courier New. Порядок: семейство × (обычный,
// жирный, курсив, жирный курсив), последним — запасной шрифт.
static FONT_FILES: [&[u8]; 13] = [
    include_bytes!("../fonts/LiberationSans-Regular.ttf"),
    include_bytes!("../fonts/LiberationSans-Bold.ttf"),
    include_bytes!("../fonts/LiberationSans-Italic.ttf"),
    include_bytes!("../fonts/LiberationSans-BoldItalic.ttf"),
    include_bytes!("../fonts/LiberationSerif-Regular.ttf"),
    include_bytes!("../fonts/LiberationSerif-Bold.ttf"),
    include_bytes!("../fonts/LiberationSerif-Italic.ttf"),
    include_bytes!("../fonts/LiberationSerif-BoldItalic.ttf"),
    include_bytes!("../fonts/LiberationMono-Regular.ttf"),
    include_bytes!("../fonts/LiberationMono-Bold.ttf"),
    include_bytes!("../fonts/LiberationMono-Italic.ttf"),
    include_bytes!("../fonts/LiberationMono-BoldItalic.ttf"),
    // DejaVu Sans (см. fonts/LICENSE-DejaVu) — для символов, которых нет в
    // Liberation (стрелки, математические и прочие знаки)
    include_bytes!("../fonts/DejaVuSans.ttf"),
];
/// Индекс запасного шрифта в [`FONT_FILES`].
const FALLBACK_FACE: usize = 12;

/// Ширина колонки по умолчанию, 1/8 символа (9 символов, как в 1С)
const DEFAULT_COL_WIDTH_UNITS: f32 = 72.0;
/// Пиксели на единицу ширины при 96 DPI: 105 твипов / 8 / 15 твипов-на-px
const PX_PER_WIDTH_UNIT: f32 = 7.0 / 8.0;
/// Пиксели на единицу высоты при 96 DPI: 96 / 288
const PX_PER_HEIGHT_UNIT: f32 = 1.0 / 3.0;
/// Горизонтальный отступ текста внутри ячейки, px (до масштабирования)
const CELL_PAD_X: f32 = 2.0;
/// Вертикальный запас автоматической высоты строки, px
const CELL_PAD_Y: f32 = 3.0;
/// Предел размеров картинки по каждой стороне. 65 500 — предел большинства
/// кодеков (в частности, JPEG не поддерживает сторону > 65 535). Длинные
/// документы (например, прайс на тысячу строк) при этом отрисовываются.
pub const MAX_DIMENSION: u32 = 65_500;
/// Предел площади картинки в пикселях. Буфер RGBA занимает 4 байта на
/// пиксель, ещё 3 байта на пиксель нужны при кодировании, — 250 Мпикс это
/// около 1,75 ГБ. Без этого предела картинка 65 500 × 65 500 потребовала бы
/// ~30 ГБ, и нехватка памяти аварийно завершила бы процесс 1С целиком
/// (`catch_unwind` такую ошибку не перехватывает).
pub const MAX_PIXELS: u64 = 250_000_000;
/// Предел размера закодированной картинки по умолчанию (свойство
/// «МаксимальныйРазмерКартинки»), байт.
pub const DEFAULT_MAX_IMAGE_BYTES: usize = 50_000_000;
/// Предел числа записей индекса объединений (см. [`MergeIndex`]): защита от
/// документов с множеством огромных (в т.ч. перекрывающихся) объединений.
const MAX_MERGE_INDEX_ENTRIES: usize = 10_000_000;

/// Проверяет, что картинку заданного размера можно создать: каждая сторона
/// не больше [`MAX_DIMENSION`], площадь не больше [`MAX_PIXELS`]. Вызывается
/// до выделения памяти под буфер. `hint` — подсказка, что уменьшить.
pub fn check_image_size(width: u64, height: u64, hint: &str) -> Result<(), String> {
    if width > MAX_DIMENSION as u64 || height > MAX_DIMENSION as u64 {
        return Err(format!(
            "слишком большая картинка {width}x{height}px \
             (предел стороны {MAX_DIMENSION}px) — {hint}"
        ));
    }
    if width * height > MAX_PIXELS {
        return Err(format!(
            "слишком большая картинка {width}x{height}px ({} Мпикс, \
             предел {} Мпикс) — {hint}",
            width * height / 1_000_000,
            MAX_PIXELS / 1_000_000
        ));
    }
    Ok(())
}

/// Степень сжатия PNG (свойство «СжатиеPNG»).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PngCompression {
    /// Быстрое кодирование, файл крупнее
    Fast,
    /// Баланс скорости и размера
    Default,
    /// Наименьший файл, кодирование медленнее
    Best,
}

#[derive(Debug, Clone)]
pub enum ImageKind {
    Jpeg { quality: u8 },
    Png { compression: PngCompression },
}

impl ImageKind {
    /// PNG со сжатием по умолчанию.
    pub const PNG: ImageKind = ImageKind::Png {
        compression: PngCompression::Default,
    };
}

/// Полный конвейер: модель → картинка на диске (предел размера файла —
/// [`DEFAULT_MAX_IMAGE_BYTES`]).
pub fn convert(
    doc: &Document,
    out_path: &Path,
    kind: &ImageKind,
    scale_percent: u32,
) -> Result<(), String> {
    let pixmap = render(doc, scale_percent)?;
    encode_to_file(&pixmap, out_path, kind, DEFAULT_MAX_IMAGE_BYTES)
}

// ---------------------------------------------------------------------------
// Раскладка
// ---------------------------------------------------------------------------

struct Layout {
    /// x-границы колонок для каждой строки не нужны целиком: группы колонок
    /// различаются по строкам, поэтому храним x-границы по группам, а для
    /// строки выбираем нужную. group_x[g][c] — левая граница колонки c;
    /// последний элемент — правая граница таблицы.
    group_x: Vec<Vec<f32>>,
    /// индекс в group_x для каждой строки (0 = группа по умолчанию)
    row_group_idx: HashMap<u32, usize>,
    /// y-границы строк: y[r] — верхняя граница строки r (0..=nrows)
    row_y: Vec<f32>,
    ncols: u32,
    width_px: u32,
    height_px: u32,
    scale: f32,
}

impl Layout {
    fn xs_for_row(&self, row: u32) -> &[f32] {
        let g = self.row_group_idx.get(&row).copied().unwrap_or(0);
        &self.group_x[g]
    }

    fn col_x(&self, row: u32, col: u32) -> (f32, f32) {
        let xs = self.xs_for_row(row);
        let c = (col as usize).min(xs.len() - 2);
        (xs[c], xs[c + 1])
    }

    fn row_range(&self, row: u32) -> (f32, f32) {
        let r = (row as usize).min(self.row_y.len() - 2);
        (self.row_y[r], self.row_y[r + 1])
    }
}

/// Растр глифа, готовый к наложению: покрытие пикселей прямоугольника
/// `w × h`, левый верхний угол которого смещён на `(min_x, min_y)` px от
/// целой части позиции глифа.
struct GlyphBitmap {
    min_x: f32,
    min_y: f32,
    w: usize,
    coverage: Vec<f32>,
}

/// Ключ кэша растров: начертание, глиф, размер и дробная часть позиции
/// (растр зависит только от них — см. `ab_glyph::OutlinedGlyph::px_bounds`).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    face: u8,
    glyph: u16,
    scale_x: u32,
    scale_y: u32,
    fract_x: u32,
    fract_y: u32,
}

/// Предел числа растров в кэше (каждый — от сотен байт до десятков КБ).
const GLYPH_CACHE_LIMIT: usize = 20_000;

struct Fonts<'a> {
    /// шрифты в порядке [`FONT_FILES`]
    faces: Vec<FontRef<'a>>,
    /// Кэш растров глифов: документ набран несколькими шрифтами и
    /// размерами, поэтому одни и те же глифы растеризуются многократно.
    glyphs: RefCell<HashMap<GlyphKey, Option<Rc<GlyphBitmap>>>>,
}

impl<'a> Fonts<'a> {
    fn load() -> Result<Fonts<'a>, String> {
        let load = |data| FontRef::try_from_slice(data).map_err(|e| format!("шрифт: {e}"));
        Ok(Fonts {
            faces: FONT_FILES
                .iter()
                .map(|data| load(data))
                .collect::<Result<_, _>>()?,
            glyphs: RefCell::new(HashMap::new()),
        })
    }

    fn face_index(spec: &FontSpec) -> usize {
        let family = match spec.family {
            FontFamily::Sans => 0,
            FontFamily::Serif => 4,
            FontFamily::Mono => 8,
        };
        family
            + match (spec.bold, spec.italic) {
                (false, false) => 0,
                (true, false) => 1,
                (false, true) => 2,
                (true, true) => 3,
            }
    }

    fn pick(&self, spec: &FontSpec) -> &FontRef<'a> {
        &self.faces[Self::face_index(spec)]
    }

    /// Шрифт нужного начертания в масштабе `px`.
    fn scaled(&self, spec: &FontSpec, px: f32) -> ScaledFace<'_, 'a> {
        ScaledFace {
            face: Self::face_index(spec) as u8,
            font: self.pick(spec).as_scaled(px),
            fallback: self.faces[FALLBACK_FACE].as_scaled(px),
        }
    }

    /// Растр глифа `id` в позиции `(x, y)`. Возвращает растр и целую часть
    /// позиции, к которой прибавляется смещение растра.
    fn glyph_bitmap(
        &self,
        sf: &ScaledFace,
        (fallback, id): FaceGlyph,
        x: f32,
        y: f32,
    ) -> (Option<Rc<GlyphBitmap>>, f32, f32) {
        let (trunc_x, fract_x) = (x.trunc(), x.fract());
        let (trunc_y, fract_y) = (y.trunc(), y.fract());
        let font = sf.font_of(fallback);
        let scale = font.scale();
        let key = GlyphKey {
            face: if fallback {
                FALLBACK_FACE as u8
            } else {
                sf.face
            },
            glyph: id.0,
            scale_x: scale.x.to_bits(),
            scale_y: scale.y.to_bits(),
            fract_x: fract_x.to_bits(),
            fract_y: fract_y.to_bits(),
        };
        if let Some(cached) = self.glyphs.borrow().get(&key) {
            return (cached.clone(), trunc_x, trunc_y);
        }
        // Растеризация в позиции, равной дробной части: результат совпадает
        // с растеризацией в исходной позиции со сдвигом на целую часть.
        let glyph = id.with_scale_and_position(scale, ab_glyph::point(fract_x, fract_y));
        let bitmap = font.font().outline_glyph(glyph).map(|outlined| {
            let bounds = outlined.px_bounds();
            let w = bounds.width() as usize;
            let h = bounds.height() as usize;
            let mut coverage = vec![0.0f32; w * h];
            outlined.draw(|gx, gy, cov| {
                coverage[gy as usize * w + gx as usize] = cov;
            });
            Rc::new(GlyphBitmap {
                min_x: bounds.min.x,
                min_y: bounds.min.y,
                w,
                coverage,
            })
        });
        let mut cache = self.glyphs.borrow_mut();
        if cache.len() >= GLYPH_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(key, bitmap.clone());
        (bitmap, trunc_x, trunc_y)
    }
}

/// Глиф с признаком «из запасного шрифта».
type FaceGlyph = (bool, GlyphId);

/// Шрифт конкретного начертания в конкретном масштабе (с запасным шрифтом
/// того же размера). Метрики строки (`ascent`, `descent`) — основного шрифта.
struct ScaledFace<'f, 'a> {
    face: u8,
    font: PxScaleFont<&'f FontRef<'a>>,
    fallback: PxScaleFont<&'f FontRef<'a>>,
}

impl<'f, 'a> ScaledFace<'f, 'a> {
    /// Глиф символа: из основного шрифта, а если там его нет — из запасного.
    fn glyph(&self, ch: char) -> FaceGlyph {
        let id = self.font.glyph_id(ch);
        if id.0 == 0 && !ch.is_whitespace() {
            let fid = self.fallback.glyph_id(ch);
            if fid.0 != 0 {
                return (true, fid);
            }
        }
        (false, id)
    }

    fn font_of(&self, fallback: bool) -> &PxScaleFont<&'f FontRef<'a>> {
        if fallback {
            &self.fallback
        } else {
            &self.font
        }
    }

    /// Кернинг пары глифов (только внутри одного шрифта).
    fn kern(&self, prev: FaceGlyph, next: FaceGlyph) -> f32 {
        if prev.0 == next.0 {
            self.font_of(next.0).kern(prev.1, next.1)
        } else {
            0.0
        }
    }

    fn h_advance(&self, g: FaceGlyph) -> f32 {
        self.font_of(g.0).h_advance(g.1)
    }
}

impl<'f, 'a> std::ops::Deref for ScaledFace<'f, 'a> {
    type Target = PxScaleFont<&'f FontRef<'a>>;

    fn deref(&self) -> &Self::Target {
        &self.font
    }
}

fn font_px(spec: &FontSpec, scale: f32) -> f32 {
    // десятые доли пункта → px при 96 DPI (1 pt = 4/3 px)
    spec.size_tenth_pt as f32 / 10.0 * 4.0 / 3.0 * scale
}

/// Высота строки текста данного шрифта в px (ascent + descent).
fn line_height(fonts: &Fonts, spec: &FontSpec, scale: f32) -> f32 {
    let f = fonts.pick(spec).as_scaled(font_px(spec, scale));
    f.ascent() - f.descent()
}

/// Состояние измерения строки: ширина и последний глиф (для кернинга).
type Measure = (f32, Option<FaceGlyph>);

/// Продолжает измерение ширины строки с состояния `state` на `text`.
/// Измерение строки целиком и по частям даёт одинаковый результат.
fn measure_more(f: &ScaledFace, state: Measure, text: &str) -> Measure {
    let (mut w, mut prev) = state;
    for ch in text.chars() {
        // управляющие символы (табуляция, возврат каретки и пр.) не имеют
        // глифа — иначе они рисуются «квадратом .notdef»
        if ch.is_control() {
            prev = None;
            continue;
        }
        let g = f.glyph(ch);
        if let Some(p) = prev {
            w += f.kern(p, g);
        }
        w += f.h_advance(g);
        prev = Some(g);
    }
    (w, prev)
}

fn text_width(fonts: &Fonts, spec: &FontSpec, scale: f32, text: &str) -> f32 {
    let f = fonts.scaled(spec, font_px(spec, scale));
    measure_more(&f, (0.0, None), text).0
}

/// Перенос текста по словам под заданную ширину. Ширина строки-кандидата
/// наращивается по словам, а не пересчитывается целиком (линейное время).
fn wrap_text(fonts: &Fonts, spec: &FontSpec, scale: f32, text: &str, max_w: f32) -> Vec<String> {
    let f = fonts.scaled(spec, font_px(spec, scale));
    let mut lines = Vec::new();
    for raw_line in text.split('\n') {
        let raw_line = raw_line.trim_end_matches('\r');
        if measure_more(&f, (0.0, None), raw_line).0 <= max_w {
            lines.push(raw_line.to_string());
            continue;
        }
        let mut current = String::new();
        let mut current_m: Measure = (0.0, None);
        for word in raw_line.split(' ') {
            if current.is_empty() {
                current.push_str(word);
                current_m = measure_more(&f, (0.0, None), word);
                continue;
            }
            let candidate_m = measure_more(&f, measure_more(&f, current_m, " "), word);
            if candidate_m.0 <= max_w {
                current.push(' ');
                current.push_str(word);
                current_m = candidate_m;
            } else {
                lines.push(std::mem::take(&mut current));
                current.push_str(word);
                current_m = measure_more(&f, (0.0, None), word);
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Индекс объединений: для каждой строки документа — отрезки колонок
/// `(левая, правая, индекс объединения)`, упорядоченные по левой колонке.
///
/// Хранятся только строки, в которых есть ячейки (запросы бывают только к
/// ним), поэтому размер индекса не зависит от площади объединений: прежняя
/// карта «ячейка → объединение» для объединения 100 000 × 100 000 из
/// повреждённого файла заняла бы 10¹⁰ записей.
struct MergeIndex {
    rows: HashMap<u32, Vec<(u32, u32, usize)>>,
}

impl MergeIndex {
    fn new(doc: &Document) -> Result<MergeIndex, String> {
        let mut row_indices: Vec<u32> = doc.rows.iter().map(|r| r.index).collect();
        row_indices.sort_unstable();
        row_indices.dedup();
        let span = |m: &Merge| {
            let a = row_indices.partition_point(|&r| r < m.top);
            let b = row_indices.partition_point(|&r| r <= m.bottom);
            &row_indices[a..b]
        };
        let entries: usize = doc.merges.iter().map(|m| span(m).len()).sum();
        if entries > MAX_MERGE_INDEX_ENTRIES {
            return Err(format!(
                "слишком сложная структура объединений ячеек ({entries} записей, \
                 предел {MAX_MERGE_INDEX_ENTRIES}) — файл, вероятно, повреждён"
            ));
        }
        let mut rows: HashMap<u32, Vec<(u32, u32, usize)>> = HashMap::new();
        for (i, m) in doc.merges.iter().enumerate() {
            for &r in span(m) {
                rows.entry(r).or_default().push((m.left, m.right, i));
            }
        }
        for spans in rows.values_mut() {
            spans.sort_unstable();
        }
        Ok(MergeIndex { rows })
    }

    /// Индекс объединения, покрывающего ячейку (строка, колонка).
    fn get(&self, &(row, col): &(u32, u32)) -> Option<&usize> {
        let spans = self.rows.get(&row)?;
        let k = spans.partition_point(|&(left, _, _)| left <= col);
        let (_, right, i) = spans.get(k.checked_sub(1)?)?;
        (col <= *right).then_some(i)
    }

    fn contains_key(&self, cell: &(u32, u32)) -> bool {
        self.get(cell).is_some()
    }
}

/// Данные документа, которые нужны на всех этапах рендера и вычисляются
/// один раз: индекс объединений и эффективные форматы ячеек.
struct Prepared {
    merges: MergeIndex,
    /// (формат строки, формат ячейки) → эффективный формат
    formats: HashMap<(u32, u32), CellFormat>,
    no_format: CellFormat,
}

impl Prepared {
    fn new(doc: &Document) -> Result<Prepared, String> {
        let mut formats = HashMap::new();
        for row in &doc.rows {
            for (_, cell) in &row.cells {
                formats
                    .entry((row.format, cell.format))
                    .or_insert_with(|| effective_format(doc, row.format, cell));
            }
        }
        Ok(Prepared {
            merges: MergeIndex::new(doc)?,
            formats,
            no_format: CellFormat::default(),
        })
    }

    /// Эффективный формат ячейки строки (см. [`effective_format`]).
    fn format(&self, row_format: u32, cell: &Cell) -> &CellFormat {
        self.formats
            .get(&(row_format, cell.format))
            .unwrap_or(&self.no_format)
    }
}

/// Эффективный формат ячейки: собственный формат ячейки поверх формата строки
/// (значения ячейки перекрывают значения строки).
fn effective_format(doc: &Document, row_format: u32, cell: &Cell) -> CellFormat {
    let base = doc.format(row_format).cloned().unwrap_or_default();
    let Some(top) = doc.format(cell.format) else {
        return base;
    };
    CellFormat {
        font: top.font.or(base.font),
        border_left: top.border_left.or(base.border_left),
        border_top: top.border_top.or(base.border_top),
        border_right: top.border_right.or(base.border_right),
        border_bottom: top.border_bottom.or(base.border_bottom),
        borders_color: top.borders_color.or(base.borders_color),
        height: top.height.or(base.height),
        width: top.width.or(base.width),
        h_align: top.h_align.or(base.h_align),
        v_align: top.v_align.or(base.v_align),
        text_color: top.text_color.or(base.text_color),
        bg_color: top.bg_color.or(base.bg_color),
        pattern: top.pattern.or(base.pattern),
        pattern_color: top.pattern_color.or(base.pattern_color),
        text_control: top.text_control.or(base.text_control),
        orientation: top.orientation.or(base.orientation),
    }
}

type Rgb = (u8, u8, u8);
const BLACK: Rgb = (0, 0, 0);

/// Битмапы узоров заливки 8×8 (узоры 1–17 формата; бит x строки y = пиксель
/// цвета узора). Взяты из классической реализации табличного документа.
const PATTERN_TILES: [[u8; 8]; 17] = [
    [0xAA, 0xFF, 0x55, 0xFF, 0xAA, 0xFF, 0x55, 0xFF],
    [0xAA, 0x55, 0xAA, 0x55, 0xAA, 0x55, 0xAA, 0x55],
    [0x00, 0x55, 0x00, 0xAA, 0x00, 0x55, 0x00, 0xAA],
    [0xFF, 0x01, 0x01, 0x01, 0xFF, 0x10, 0x10, 0x10],
    [0x11, 0x2A, 0x44, 0xA2, 0x11, 0xA8, 0x44, 0x8A],
    [0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66],
    [0x00, 0xFF, 0xFF, 0x00, 0x00, 0xFF, 0xFF, 0x00],
    [0x99, 0xCC, 0x66, 0x33, 0x99, 0xCC, 0x66, 0x33],
    [0xCC, 0x99, 0x33, 0x66, 0xCC, 0x99, 0x33, 0x66],
    [0xCC, 0xCC, 0x33, 0x33, 0xCC, 0xCC, 0x33, 0x33],
    [0xDD, 0xDD, 0x77, 0x77, 0xDD, 0xDD, 0x77, 0x77],
    [0xE1, 0xF0, 0x78, 0x3C, 0x1E, 0x0F, 0x87, 0xC3],
    [0x87, 0x0F, 0x1E, 0x3C, 0x78, 0xF0, 0xE1, 0xC3],
    [0x88, 0x44, 0x22, 0x11, 0x88, 0x44, 0x22, 0x11],
    [0x44, 0x88, 0x11, 0x22, 0x44, 0x88, 0x11, 0x22],
    [0x44, 0xFF, 0x44, 0x44, 0x44, 0xFF, 0x44, 0x44],
    [0xC0, 0xC0, 0x33, 0x33, 0x0C, 0x0C, 0x33, 0x33],
];

/// «Без узора»
const PATTERN_NONE: u32 = 255;
/// «Сплошной» — заливка цветом узора
const PATTERN_SOLID: u32 = 0;

/// Вертикальная ориентация текста (90° в десятых долях градуса).
const ORIENT_VERTICAL: u32 = 900;

fn is_vertical(fmt: &CellFormat) -> bool {
    fmt.orientation == Some(ORIENT_VERTICAL)
}

fn font_spec(doc: &Document, fmt: &CellFormat) -> FontSpec {
    fmt.font
        .and_then(|i| doc.fonts.get(i as usize))
        .cloned()
        .unwrap_or_default()
}

fn build_layout(
    doc: &Document,
    fonts: &Fonts,
    prep: &Prepared,
    scale: f32,
) -> Result<Layout, String> {
    // размер таблицы
    let (nrows, ncols) = table_size(doc);
    // Каждая строка занимает не меньше 1 px, поэтому строк больше
    // MAX_DIMENSION картинка вместить не может. Проверка — до раскладки:
    // номер строки из повреждённого файла (например, 2·10⁹) иначе приводит
    // к выделению гигабайтов памяти под границы строк и аварийному
    // завершению процесса 1С.
    if nrows > MAX_DIMENSION {
        return Err(format!(
            "слишком много строк: {nrows} (предел {MAX_DIMENSION}) — \
             картинка такой высоты невозможна или файл повреждён"
        ));
    }
    if ncols > MAX_DIMENSION {
        return Err(format!(
            "слишком много колонок: {ncols} (предел {MAX_DIMENSION}) — \
             картинка такой ширины невозможна или файл повреждён"
        ));
    }

    // x-границы по группам колонок
    let groups: Vec<&crate::mxl_parser::ColumnGroup> = std::iter::once(&doc.default_group)
        .chain(doc.extra_groups.iter())
        .collect();
    if groups.len().saturating_mul(ncols as usize + 1) > MAX_MERGE_INDEX_ENTRIES {
        return Err(format!(
            "слишком много групп колонок ({}) — файл, вероятно, повреждён",
            groups.len()
        ));
    }
    let mut group_x = Vec::with_capacity(groups.len());
    for (gi, g) in groups.iter().enumerate() {
        let widths: HashMap<u32, u32> = g.columns.iter().copied().collect();
        let mut xs = Vec::with_capacity(ncols as usize + 1);
        let mut x = 1.0f32; // поле слева под внешнюю рамку
        xs.push(x);
        for c in 0..ncols {
            let units = widths
                .get(&c)
                .and_then(|fmt| doc.format(*fmt))
                .and_then(|f| f.width)
                .map(|w| w as f32)
                // Дополнительная группа колонок — независимая полоса со своим
                // набором колонок: не заданная в ней колонка в этой полосе
                // просто отсутствует (ширина 0), иначе полоса «раздувается»
                // фантомными колонками справа. В группе по умолчанию все
                // колонки листа заданы, поэтому запасная ширина применяется
                // только к ней.
                .unwrap_or(if gi == 0 {
                    DEFAULT_COL_WIDTH_UNITS
                } else {
                    0.0
                });
            x += if units > 0.0 {
                (units * PX_PER_WIDTH_UNIT * scale).max(1.0)
            } else {
                0.0
            };
            xs.push(x.round());
        }
        group_x.push(xs);
    }
    let mut row_group_idx = HashMap::new();
    for &(row, g) in &doc.row_groups {
        // группа 0 в карте — первая дополнительная, т.е. индекс 1 в group_x
        let idx = g as usize + 1;
        if idx < group_x.len() {
            row_group_idx.insert(row, idx);
        }
    }

    // объединения понадобятся для ширины переноса
    let merge_of = &prep.merges;

    // высоты строк
    let default_font = FontSpec::default();
    let default_row_h = (line_height(fonts, &default_font, scale) + CELL_PAD_Y * scale).ceil();
    let rows_by_index: HashMap<u32, &crate::mxl_parser::Row> =
        doc.rows.iter().map(|r| (r.index, r)).collect();
    let mut row_y = Vec::with_capacity(nrows as usize + 1);
    let mut y = 1.0f32;
    row_y.push(y);
    for r in 0..nrows {
        let mut h = default_row_h;
        if let Some(row) = rows_by_index.get(&r) {
            // высота 0 при установленном флаге означает «автоподбор»
            let explicit = doc
                .format(row.format)
                .and_then(|f| f.height)
                .filter(|units| *units > 0)
                .map(|units| (units as f32 * PX_PER_HEIGHT_UNIT * scale).max(2.0));
            if let Some(hpx) = explicit {
                h = hpx;
            } else {
                // Автоподбор: высота по содержимому ячеек. Пустые ячейки и
                // нижние части вертикальных объединений собственной высоты не
                // задают — их содержимое относится к верхней строке
                // объединения. Поэтому строка без собственного текста
                // сворачивается (как в 1С), а не растягивается на строку.
                let mut content_h = 0.0f32;
                for (col, cell) in &row.cells {
                    // нижняя часть вертикального объединения — пропускаем
                    if let Some(&mi) = merge_of.get(&(r, *col)) {
                        let m = &doc.merges[mi];
                        if m.bottom > m.top && m.top != r {
                            continue;
                        }
                    }
                    let Some(text) = cell.text.as_deref().filter(|t| !t.trim().is_empty()) else {
                        continue;
                    };
                    let fmt = prep.format(row.format, cell);
                    let spec = font_spec(doc, fmt);
                    let lh = line_height(fonts, &spec, scale);
                    if is_vertical(fmt) {
                        // вертикальный текст: высота должна вместить самую
                        // длинную строку текста
                        let widest = text
                            .split('\n')
                            .map(|l| text_width(fonts, &spec, scale, l.trim_end_matches('\r')))
                            .fold(0.0f32, f32::max);
                        content_h = content_h.max(widest + CELL_PAD_Y * scale);
                        continue;
                    }
                    let mut lines = if fmt.text_control == Some(3) {
                        // ширина области переноса: ячейка или объединение
                        let (x0, x1) = match merge_of.get(&(r, *col)) {
                            Some(&mi) => {
                                let m = &doc.merges[mi];
                                let (a, _) = col_span_x(&group_x, &row_group_idx, r, m.left);
                                let (_, b) = col_span_x(&group_x, &row_group_idx, r, m.right);
                                (a, b)
                            }
                            None => col_span_x(&group_x, &row_group_idx, r, *col),
                        };
                        let max_w = (x1 - x0 - 2.0 * CELL_PAD_X * scale).max(4.0);
                        wrap_text(fonts, &spec, scale, text, max_w).len()
                    } else {
                        text.split('\n').count()
                    };
                    // текст верхней части вертикального объединения — одна строка
                    if let Some(&mi) = merge_of.get(&(r, *col)) {
                        let m = &doc.merges[mi];
                        if m.top == r && m.bottom > m.top {
                            lines = 1;
                        }
                    }
                    content_h = content_h.max(lines as f32 * lh + CELL_PAD_Y * scale);
                }
                h = if content_h > 0.0 {
                    content_h.ceil()
                } else {
                    // строка без собственного содержимого — минимум под рамку
                    (2.0 * scale).max(1.0)
                };
            }
        }
        y += h.max(1.0);
        row_y.push(y.round());
    }

    // Ширина картинки — по фактически занятой правой границе (правый край
    // самой правой непустой ячейки/объединения в каждой строке), а не по
    // полной ширине самой широкой группы: у полос с колонками-«хвостами»
    // за содержимым иначе остаётся большое пустое поле справа.
    let mut content_right = 0.0f32;
    for row in &doc.rows {
        let r = row.index;
        for (col, _) in &row.cells {
            let right = match merge_of.get(&(r, *col)) {
                Some(&mi) => col_span_x(&group_x, &row_group_idx, r, doc.merges[mi].right).1,
                None => col_span_x(&group_x, &row_group_idx, r, *col).1,
            };
            content_right = content_right.max(right);
        }
    }
    for m in &doc.merges {
        content_right = content_right.max(col_span_x(&group_x, &row_group_idx, m.top, m.right).1);
    }
    if content_right < 1.0 {
        // на случай документа без ячеек — исходная оценка по группам
        content_right = group_x
            .iter()
            .map(|xs| *xs.last().unwrap())
            .fold(0.0f32, f32::max);
    }
    let width_px = (content_right.ceil() as u32).saturating_add(2);
    let height_px = (*row_y.last().unwrap() as u32).saturating_add(2);
    Ok(Layout {
        group_x,
        row_group_idx,
        row_y,
        ncols,
        width_px,
        height_px,
        scale,
    })
}

fn col_span_x(
    group_x: &[Vec<f32>],
    row_group_idx: &HashMap<u32, usize>,
    row: u32,
    col: u32,
) -> (f32, f32) {
    let g = row_group_idx.get(&row).copied().unwrap_or(0);
    let xs = &group_x[g];
    let c = (col as usize).min(xs.len() - 2);
    (xs[c], xs[c + 1])
}

// ---------------------------------------------------------------------------
// Рисование
// ---------------------------------------------------------------------------

/// Рендерит документ в растровый буфер RGBA.
pub fn render(doc: &Document, scale_percent: u32) -> Result<Pixmap, String> {
    let scale = scale_percent as f32 / 100.0;
    let fonts = Fonts::load()?;
    let prep = Prepared::new(doc)?;
    let layout = build_layout(doc, &fonts, &prep, scale)?;
    check_image_size(
        layout.width_px as u64,
        layout.height_px as u64,
        "уменьшите Масштаб или выберите одну страницу (НомерСтраницы)",
    )?;
    draw_layout(doc, &fonts, &prep, &layout)
}

/// Страница документа: диапазоны строк и колонок (0-базные, включительно).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageRange {
    pub rows: (u32, u32),
    pub cols: (u32, u32),
}

/// Делит отрезок `[from, to]` на части после позиций `breaks`.
fn split_by_breaks(from: u32, to: u32, breaks: &[u32]) -> Vec<(u32, u32)> {
    let mut parts = Vec::new();
    let mut start = from;
    for &b in breaks {
        if b >= start && b < to {
            parts.push((start, b));
            start = b + 1;
        }
    }
    parts.push((start, to));
    parts
}

/// Число строк и колонок таблицы (по ячейкам, объединениям и колонкам).
fn table_size(doc: &Document) -> (u32, u32) {
    let mut ncols: u32 = 1;
    let mut nrows: u32 = 1;
    for row in &doc.rows {
        nrows = nrows.max(row.index.saturating_add(1));
        for (col, _) in &row.cells {
            ncols = ncols.max(col.saturating_add(1));
        }
    }
    for m in &doc.merges {
        nrows = nrows.max(m.bottom.saturating_add(1));
        ncols = ncols.max(m.right.saturating_add(1));
    }
    for (col, _) in &doc.default_group.columns {
        ncols = ncols.max(col.saturating_add(1));
    }
    (nrows, ncols)
}

/// Страницы документа для печати: область печати (а без неё — вся
/// таблица), разделённая явными горизонтальными и вертикальными разрывами
/// страниц. Порядок страниц — «сначала вниз, затем вправо». Автоматическое
/// разбиение по размеру листа бумаги не выполняется.
pub fn pages(doc: &Document) -> Vec<PageRange> {
    let (nrows, ncols) = table_size(doc);
    let area = doc.print_area.unwrap_or_default();
    // граница области за пределами таблицы ограничивается таблицей
    let top = area.top.unwrap_or(0).min(nrows - 1);
    let bottom = area.bottom.unwrap_or(nrows - 1).min(nrows - 1).max(top);
    let left = area.left.unwrap_or(0).min(ncols - 1);
    let right = area.right.unwrap_or(ncols - 1).min(ncols - 1).max(left);
    let row_parts = split_by_breaks(top, bottom, &doc.row_breaks());
    let col_parts = split_by_breaks(left, right, &doc.column_breaks());
    col_parts
        .iter()
        .flat_map(|&cols| row_parts.iter().map(move |&rows| PageRange { rows, cols }))
        .collect()
}

/// Рендерит страницу `page_number` (с 1) документа (см. [`pages`]).
/// Раскладка строится для всего документа, но рисуется только окно
/// страницы: память нужна лишь под картинку страницы, а содержимое на
/// границе страницы (объединения, выпущенный текст) обрезается так же, как
/// при печати.
pub fn render_page(doc: &Document, scale_percent: u32, page_number: u32) -> Result<Pixmap, String> {
    let all = pages(doc);
    let page = page_number
        .checked_sub(1)
        .and_then(|i| all.get(i as usize))
        .ok_or_else(|| {
            format!(
                "запрошена страница {page_number}, но в документе всего {} стр.",
                all.len()
            )
        })?;
    let scale = scale_percent as f32 / 100.0;
    let fonts = Fonts::load()?;
    let prep = Prepared::new(doc)?;
    let mut layout = build_layout(doc, &fonts, &prep, scale)?;
    // поле 1 px слева/сверху и 2 px справа/снизу под рамки — как у всего
    // документа
    let x0 = layout.group_x[0][page.cols.0 as usize] - 1.0;
    let x1 = layout.group_x[0][page.cols.1 as usize + 1] + 2.0;
    let y0 = layout.row_y[page.rows.0 as usize] - 1.0;
    let y1 = layout.row_y[page.rows.1 as usize + 1] + 2.0;
    let (w, h) = ((x1 - x0).ceil().max(1.0), (y1 - y0).ceil().max(1.0));
    check_image_size(w as u64, h as u64, "уменьшите Масштаб")?;
    for xs in &mut layout.group_x {
        for x in xs.iter_mut() {
            *x -= x0;
        }
    }
    for y in &mut layout.row_y {
        *y -= y0;
    }
    layout.width_px = w as u32;
    layout.height_px = h as u32;
    draw_layout(doc, &fonts, &prep, &layout)
}

/// Рисует документ по готовой раскладке в буфер размера раскладки.
fn draw_layout(
    doc: &Document,
    fonts: &Fonts,
    prep: &Prepared,
    layout: &Layout,
) -> Result<Pixmap, String> {
    let mut pixmap = Pixmap::new(layout.width_px, layout.height_px)
        .ok_or_else(|| "не удалось создать буфер изображения".to_string())?;
    pixmap.fill(tiny_skia::Color::WHITE);

    draw_backgrounds(doc, layout, prep, &mut pixmap);
    draw_borders(doc, layout, prep, &mut pixmap);
    draw_texts(doc, fonts, layout, prep, &mut pixmap);
    draw_drawings(doc, fonts, layout, &mut pixmap);
    Ok(pixmap)
}

/// Рисует графические объекты листа поверх таблицы: картинки, надписи,
/// прямоугольники. Геометрия — как в классическом формате: ячейки-якоря
/// начала/конца плюс смещения; вырожденный прямоугольник заменяется
/// естественным размером содержимого.
fn draw_drawings(doc: &Document, fonts: &Fonts, layout: &Layout, pixmap: &mut Pixmap) {
    let scale = layout.scale;
    for d in &doc.drawings {
        if d.kind == DrawingKind::Other {
            continue;
        }
        let x0 = layout.col_x(d.row_start, d.col_start).0
            + d.offsets.0 as f32 * PX_PER_WIDTH_UNIT * scale;
        let y0 = layout.row_range(d.row_start).0 + d.offsets.1 as f32 * PX_PER_HEIGHT_UNIT * scale;
        let mut x1 =
            layout.col_x(d.row_end, d.col_end).0 + d.offsets.2 as f32 * PX_PER_WIDTH_UNIT * scale;
        let mut y1 =
            layout.row_range(d.row_end).0 + d.offsets.3 as f32 * PX_PER_HEIGHT_UNIT * scale;

        match d.kind {
            DrawingKind::Picture => {
                let Some(img) = d
                    .picture
                    .and_then(|i| doc.pictures.get(i as usize - 1))
                    .and_then(|bytes| image::load_from_memory(bytes).ok())
                else {
                    continue;
                };
                let rgba = img.to_rgba8();
                let (iw, ih) = (rgba.width() as f32, rgba.height() as f32);
                if x1 - x0 < 8.0 || y1 - y0 < 8.0 {
                    // вырожденная геометрия: естественный размер
                    x1 = x0 + iw * scale;
                    y1 = y0 + ih * scale;
                } else {
                    // вписываем с сохранением пропорций
                    let k = ((x1 - x0) / iw).min((y1 - y0) / ih);
                    x1 = x0 + iw * k;
                    y1 = y0 + ih * k;
                }
                blit_image(pixmap, &rgba, x0, y0, x1, y1);
            }
            DrawingKind::Text | DrawingKind::Rectangle => {
                let fmt = doc.format(d.format).cloned().unwrap_or_default();
                let spec = font_spec(doc, &fmt);
                let lines: Vec<String> = d
                    .text
                    .as_deref()
                    .unwrap_or("")
                    .split('\n')
                    .map(|l| l.trim_end_matches('\r').to_string())
                    .collect();
                let lh = line_height(fonts, &spec, scale);
                if x1 - x0 < 8.0 || y1 - y0 < 8.0 {
                    let widest = lines
                        .iter()
                        .map(|l| text_width(fonts, &spec, scale, l))
                        .fold(0.0f32, f32::max);
                    x1 = x0 + (widest + 8.0 * scale).max(60.0 * scale);
                    y1 = y0 + (lines.len() as f32 * lh + 6.0 * scale).max(20.0 * scale);
                }
                // рамка объекта
                let border_color = fmt
                    .borders_color
                    .and_then(|i| doc.color_rgb(i))
                    .unwrap_or(BLACK);
                let t = scale.max(1.0).round();
                fill_rect(pixmap, x0, y0, x1 - x0, t, border_color);
                fill_rect(pixmap, x0, y1 - t, x1 - x0, t, border_color);
                fill_rect(pixmap, x0, y0, t, y1 - y0, border_color);
                fill_rect(pixmap, x1 - t, y0, t, y1 - y0, border_color);
                if d.kind == DrawingKind::Text {
                    let scaled = fonts.scaled(&spec, font_px(&spec, scale));
                    let color = fmt
                        .text_color
                        .and_then(|i| doc.color_rgb(i))
                        .unwrap_or(BLACK);
                    let pad = CELL_PAD_X * scale;
                    for (li, line) in lines.iter().enumerate() {
                        let baseline = y0 + 3.0 * scale + li as f32 * lh + scaled.ascent();
                        if baseline > y1 {
                            break;
                        }
                        draw_line_of_text(
                            pixmap,
                            fonts,
                            &scaled,
                            line,
                            x0 + pad,
                            baseline,
                            (x0, y0, x1, y1),
                            color,
                        );
                    }
                }
            }
            DrawingKind::Other => {}
        }
    }
}

/// Вписывает растровую картинку в прямоугольник с альфа-смешиванием.
/// Масштабирование — с фильтрацией: при уменьшении треугольный фильтр
/// (без «лестницы» и потери тонких линий печатей и логотипов), при
/// увеличении — Catmull-Rom (без «квадратиков» ближайшего соседа).
fn blit_image(pixmap: &mut Pixmap, rgba: &image::RgbaImage, x0: f32, y0: f32, x1: f32, y1: f32) {
    let (tw, th) = (x1 - x0, y1 - y0);
    if tw < 1.0 || th < 1.0 || rgba.width() == 0 || rgba.height() == 0 {
        return;
    }
    let (w, h) = (tw.round() as u32, th.round() as u32);
    // картинка много больше листа (повреждённые смещения) — не рисуем:
    // масштабирование заняло бы непропорционально много памяти и времени
    let page_area = pixmap.width() as u64 * pixmap.height() as u64;
    if w as u64 * h as u64 > (4 * page_area).min(MAX_PIXELS) {
        return;
    }
    let scaled;
    let src = if (w, h) == rgba.dimensions() {
        rgba
    } else {
        let filter = if w < rgba.width() || h < rgba.height() {
            image::imageops::FilterType::Triangle
        } else {
            image::imageops::FilterType::CatmullRom
        };
        scaled = image::imageops::resize(rgba, w, h, filter);
        &scaled
    };
    let width = pixmap.width() as i32;
    let height = pixmap.height() as i32;
    let (ox, oy) = (x0.round() as i32, y0.round() as i32);
    let data = pixmap.data_mut();
    for (sx, sy, p) in src.enumerate_pixels() {
        let (px, py) = (ox + sx as i32, oy + sy as i32);
        if px < 0 || py < 0 || px >= width || py >= height {
            continue;
        }
        let a = p[3] as u16;
        if a == 0 {
            continue;
        }
        let idx = (py as usize * width as usize + px as usize) * 4;
        for (offset, src) in p.0[..3].iter().enumerate() {
            let old = data[idx + offset] as u16;
            data[idx + offset] = ((old * (255 - a) + *src as u16 * a) / 255) as u8;
        }
    }
}

/// Копирует `src` в `dst` левым верхним углом в точку (x, y) построчно
/// (без смешивания). Части `src` за пределами `dst` отбрасываются.
pub fn copy_pixmap(dst: &mut Pixmap, src: &Pixmap, x: u32, y: u32) {
    let (dw, dh) = (dst.width() as usize, dst.height() as usize);
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let (x, y) = (x as usize, y as usize);
    if x >= dw || y >= dh {
        return;
    }
    let w = sw.min(dw - x) * 4;
    let src_data = src.data();
    let dst_data = dst.data_mut();
    for row in 0..sh.min(dh - y) {
        let s = row * sw * 4;
        let d = ((y + row) * dw + x) * 4;
        dst_data[d..d + w].copy_from_slice(&src_data[s..s + w]);
    }
}

fn fill_rect(pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, color: Rgb) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    if let Some(rect) = Rect::from_xywh(x, y, w, h) {
        let mut paint = Paint::default();
        paint.set_color_rgba8(color.0, color.1, color.2, 255);
        paint.anti_alias = false;
        pixmap.fill_rect(rect, &paint, Transform::identity(), None);
    }
}

/// Накладывает узор 8×8 цветом узора поверх уже залитого фона.
fn fill_pattern_overlay(
    pixmap: &mut Pixmap,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    tile: &[u8; 8],
    color: Rgb,
) {
    let width = pixmap.width() as i32;
    let height = pixmap.height() as i32;
    let data = pixmap.data_mut();
    let (px0, py0) = (x0.max(0.0) as i32, y0.max(0.0) as i32);
    let (px1, py1) = ((x1 as i32).min(width), (y1 as i32).min(height));
    for py in py0..py1 {
        let row_mask = tile[(py % 8) as usize];
        for px in px0..px1 {
            if row_mask >> (px % 8) & 1 == 1 {
                let idx = (py as usize * width as usize + px as usize) * 4;
                data[idx] = color.0;
                data[idx + 1] = color.1;
                data[idx + 2] = color.2;
            }
        }
    }
}

/// Заливка фона ячеек, включая узоры (до рамок и текста).
fn draw_backgrounds(doc: &Document, layout: &Layout, prep: &Prepared, pixmap: &mut Pixmap) {
    let merge_of = &prep.merges;
    for row in &doc.rows {
        let r = row.index;
        for (col, cell) in &row.cells {
            let fmt = prep.format(row.format, cell);
            let bg = fmt.bg_color.and_then(|i| doc.color_rgb(i));
            let pattern = fmt.pattern.unwrap_or(PATTERN_NONE);
            // Узор заливается только при явном (абсолютном) цвете узора. Если
            // цвет узора не задан или «авто», сплошной/штриховой узор НЕ
            // рисуется — иначе ячейка ошибочно закрашивается чёрным (реальные
            // счета/прайсы помечают колонку цены узором без цвета).
            let pattern_color = fmt.pattern_color.and_then(|i| doc.color_rgb(i));
            let draw_pattern = pattern != PATTERN_NONE && pattern_color.is_some();
            if bg.is_none() && !draw_pattern {
                continue;
            }
            let (mut x0, mut x1) = layout.col_x(r, *col);
            let (mut y0, mut y1) = layout.row_range(r);
            // верхняя-левая ячейка объединения закрашивает весь регион
            if let Some(&mi) = merge_of.get(&(r, *col)) {
                let m = &doc.merges[mi];
                if m.top == r && m.left == *col {
                    x0 = layout.col_x(r, m.left).0;
                    x1 = layout.col_x(r, m.right).1;
                    y0 = layout.row_range(m.top).0;
                    y1 = layout.row_range(m.bottom).1;
                }
            }
            if let Some(color) = bg {
                fill_rect(pixmap, x0, y0, x1 - x0, y1 - y0, color);
            }
            if let Some(pc) = pattern_color {
                match pattern {
                    PATTERN_NONE => {}
                    PATTERN_SOLID => {
                        fill_rect(pixmap, x0, y0, x1 - x0, y1 - y0, pc);
                    }
                    p if (1..=17).contains(&p) => {
                        fill_pattern_overlay(
                            pixmap,
                            x0,
                            y0,
                            x1,
                            y1,
                            &PATTERN_TILES[p as usize - 1],
                            pc,
                        );
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Параметры отрисовки линии рамки: (толщина в px, штрих «(вкл, выкл)» либо
/// сплошная, признак двойной линии).
type BorderStroke = (f32, Option<(f32, f32)>, bool);

/// Разрешает индекс рамки формата в описание линии. Установленный бит рамки
/// означает «линия есть», поэтому при отсутствии таблицы линий (или неверном
/// индексе) используется тонкая сплошная линия.
fn resolve_line(doc: &Document, index: u32) -> LineSpec {
    let solid_thin = LineSpec {
        kind: 1,
        thickness: 1,
    };
    doc.line(index).unwrap_or(solid_thin)
}

/// Параметры отрисовки по описанию линии из таблицы линий документа.
/// Тип линии: 0 — служебная «нет линии/авто» (1С не печатает такие рамки —
/// это экранная сетка/умолчание), 1 — сплошная, 2 — точечная, 3 — двойная,
/// 4 — редкий пунктир, 5 — частый пунктир, 6 — большой пунктир. `None` — линию
/// рисовать не нужно.
fn border_pattern(line: LineSpec, scale: f32) -> Option<BorderStroke> {
    // Тип 0 — служебная рамка (по умолчанию/авто): на печати 1С её не выводит.
    if line.kind == 0 {
        return None;
    }
    let px = |v: f32| (v * scale).round().max(1.0);
    let t = px(line.thickness.max(1) as f32);
    match line.kind {
        // сплошная
        1 => Some((t, None, false)),
        // точечная
        2 => Some((t, Some((px(1.0), px(2.0))), false)),
        // двойная (в 1С допускается только толщина 1)
        3 => Some((t, None, true)),
        // редкий пунктир
        4 => Some((t, Some((px(6.0), px(4.0))), false)),
        // частый пунктир
        5 => Some((t, Some((px(3.0), px(2.0))), false)),
        // большой пунктир
        6 => Some((t, Some((px(9.0), px(4.0))), false)),
        _ => Some((t, None, false)),
    }
}

fn draw_h_border(
    pixmap: &mut Pixmap,
    x0: f32,
    x1: f32,
    y: f32,
    line: LineSpec,
    scale: f32,
    color: Rgb,
) {
    let Some((t, dash, double)) = border_pattern(line, scale) else {
        return;
    };
    if double {
        let gap = (scale).round().max(1.0);
        fill_rect(pixmap, x0, y - t - gap / 2.0, x1 - x0, t, color);
        fill_rect(pixmap, x0, y + gap / 2.0, x1 - x0, t, color);
        return;
    }
    match dash {
        Some((on, off)) => {
            let mut x = x0;
            while x < x1 {
                fill_rect(pixmap, x, y - t / 2.0, on.min(x1 - x), t, color);
                x += on + off;
            }
        }
        None => fill_rect(pixmap, x0, y - t / 2.0, x1 - x0, t, color),
    }
}

fn draw_v_border(
    pixmap: &mut Pixmap,
    x: f32,
    y0: f32,
    y1: f32,
    line: LineSpec,
    scale: f32,
    color: Rgb,
) {
    let Some((t, dash, double)) = border_pattern(line, scale) else {
        return;
    };
    if double {
        let gap = (scale).round().max(1.0);
        fill_rect(pixmap, x - t - gap / 2.0, y0, t, y1 - y0, color);
        fill_rect(pixmap, x + gap / 2.0, y0, t, y1 - y0, color);
        return;
    }
    match dash {
        Some((on, off)) => {
            let mut y = y0;
            while y < y1 {
                fill_rect(pixmap, x - t / 2.0, y, t, on.min(y1 - y), color);
                y += on + off;
            }
        }
        None => fill_rect(pixmap, x - t / 2.0, y0, t, y1 - y0, color),
    }
}

fn draw_borders(doc: &Document, layout: &Layout, prep: &Prepared, pixmap: &mut Pixmap) {
    let merge_of = &prep.merges;
    let scale = layout.scale;
    for row in &doc.rows {
        let r = row.index;
        for (col, cell) in &row.cells {
            // Прямоугольник рамки — с учётом объединения: рамки хранятся на
            // верхней-левой ячейке объединения и охватывают весь регион.
            // Остальные ячейки объединения рамок не рисуют (иначе внутри
            // региона появляются лишние линии, а внешняя рамка обрезается по
            // ширине одной колонки — как «короткий» подчерк поля).
            let (x0, x1, y0, y1) = match merge_of.get(&(r, *col)) {
                Some(&mi) => {
                    let m = &doc.merges[mi];
                    if m.top != r || m.left != *col {
                        continue;
                    }
                    (
                        layout.col_x(r, m.left).0,
                        layout.col_x(r, m.right).1,
                        layout.row_range(m.top).0,
                        layout.row_range(m.bottom).1,
                    )
                }
                None => {
                    let (a, b) = layout.col_x(r, *col);
                    let (c, d) = layout.row_range(r);
                    (a, b, c, d)
                }
            };
            let fmt = prep.format(row.format, cell);
            let color = fmt
                .borders_color
                .and_then(|i| doc.color_rgb(i))
                .unwrap_or(BLACK);
            if let Some(idx) = fmt.border_left {
                draw_v_border(pixmap, x0, y0, y1, resolve_line(doc, idx), scale, color);
            }
            if let Some(idx) = fmt.border_right {
                draw_v_border(pixmap, x1, y0, y1, resolve_line(doc, idx), scale, color);
            }
            if let Some(idx) = fmt.border_top {
                draw_h_border(pixmap, x0, x1, y0, resolve_line(doc, idx), scale, color);
            }
            if let Some(idx) = fmt.border_bottom {
                draw_h_border(pixmap, x0, x1, y1, resolve_line(doc, idx), scale, color);
            }
        }
    }
}

fn draw_texts(
    doc: &Document,
    fonts: &Fonts,
    layout: &Layout,
    prep: &Prepared,
    pixmap: &mut Pixmap,
) {
    let merge_of = &prep.merges;
    let scale = layout.scale;
    // тексты соседних ячеек, чтобы «выпускать» текст поверх пустых
    let occupied: HashMap<(u32, u32), bool> = doc
        .rows
        .iter()
        .flat_map(|row| {
            row.cells.iter().map(move |(c, cell)| {
                (
                    (row.index, *c),
                    cell.text.as_deref().is_some_and(|t| !t.trim().is_empty()),
                )
            })
        })
        .collect();

    for row in &doc.rows {
        let r = row.index;
        for (col, cell) in &row.cells {
            let Some(text) = &cell.text else { continue };
            if text.is_empty() {
                continue;
            }
            // ячейка, накрытая объединением, но не верхняя-левая — пропуск
            let merge: Option<&Merge> = merge_of.get(&(r, *col)).map(|&i| &doc.merges[i]);
            if let Some(m) = merge {
                if m.top != r || m.left != *col {
                    continue;
                }
            }
            let fmt = prep.format(row.format, cell);
            let spec = font_spec(doc, fmt);
            let px = font_px(&spec, scale);
            let scaled = fonts.scaled(&spec, px);
            let text_color = fmt
                .text_color
                .and_then(|i| doc.color_rgb(i))
                .unwrap_or(BLACK);

            // прямоугольник ячейки/объединения
            let (mut x0, mut x1) = layout.col_x(r, *col);
            let (y0, mut y1) = layout.row_range(r);
            if let Some(m) = merge {
                x1 = layout.col_x(r, m.right).1;
                y1 = layout.row_range(m.bottom).1;
            }

            let pad = CELL_PAD_X * scale;

            if is_vertical(fmt) {
                let vlines: Vec<String> = text
                    .split('\n')
                    .map(|l| l.trim_end_matches('\r').to_string())
                    .collect();
                draw_vertical_text(
                    pixmap,
                    fonts,
                    &spec,
                    scale,
                    &vlines,
                    (x0, y0, x1, y1),
                    fmt,
                    text_color,
                );
                continue;
            }
            let control = fmt.text_control.unwrap_or(0);
            let h_align = fmt.h_align.unwrap_or(0);
            let lines: Vec<String> = if control == 3 {
                wrap_text(fonts, &spec, scale, text, (x1 - x0 - 2.0 * pad).max(4.0))
            } else {
                text.split('\n')
                    .map(|l| l.trim_end_matches('\r').to_string())
                    .collect()
            };

            // «выпуск» текста на соседние пустые ячейки (контроль = авто)
            let (mut clip_x0, mut clip_x1) = (x0, x1);
            if control == 0 && merge.is_none() {
                let widest = lines
                    .iter()
                    .map(|l| text_width(fonts, &spec, scale, l))
                    .fold(0.0f32, f32::max);
                if widest > x1 - x0 - 2.0 * pad {
                    let free = |c: u32| -> bool {
                        c < layout.ncols
                            && !occupied.get(&(r, c)).copied().unwrap_or(false)
                            && !merge_of.contains_key(&(r, c))
                    };
                    if h_align == 0 || h_align == 4 {
                        let mut c = *col + 1;
                        while free(c) && clip_x1 - x0 < widest + 2.0 * pad {
                            clip_x1 = layout.col_x(r, c).1;
                            c += 1;
                        }
                    } else if h_align == 2 {
                        let mut c = *col;
                        while c > 0 && free(c - 1) && x1 - clip_x0 < widest + 2.0 * pad {
                            c -= 1;
                            clip_x0 = layout.col_x(r, c).0;
                        }
                    } else if h_align == 6 {
                        let (mut cl, mut cr) = (*col, *col + 1);
                        while clip_x1 - clip_x0 < widest + 2.0 * pad {
                            let grew_r = free(cr);
                            if grew_r {
                                clip_x1 = layout.col_x(r, cr).1;
                                cr += 1;
                            }
                            let grew_l = cl > 0 && free(cl - 1);
                            if grew_l {
                                cl -= 1;
                                clip_x0 = layout.col_x(r, cl).0;
                            }
                            if !grew_r && !grew_l {
                                break;
                            }
                        }
                    }
                    x0 = clip_x0;
                    x1 = clip_x1;
                }
            }

            // вертикальное положение блока текста
            let lh = scaled.ascent() - scaled.descent();
            let total_h = lines.len() as f32 * lh;
            let v_align = fmt.v_align.unwrap_or(8); // умолчание 1С — низ
            let top = match v_align {
                0 => y0 + 1.0 * scale,
                24 => (y0 + y1 - total_h) / 2.0,
                _ => y1 - total_h - 1.0 * scale,
            };

            for (li, line) in lines.iter().enumerate() {
                if line.is_empty() {
                    continue;
                }
                let mut line_to_draw = std::borrow::Cow::Borrowed(line);
                if control == 2 {
                    let lw = text_width(fonts, &spec, scale, line);
                    let cell_w = x1 - x0 - 2.0 * pad;
                    if lw > 0.5 && cell_w > lw {
                        let reps = ((cell_w / lw).ceil() as usize + 1).min(500);
                        line_to_draw = std::borrow::Cow::Owned(line.repeat(reps));
                    }
                }

                let lw = text_width(fonts, &spec, scale, &line_to_draw);
                let tx = if control == 2 {
                    x0 + pad
                } else {
                    match h_align {
                        2 => x1 - pad - lw,
                        6 => (x0 + x1 - lw) / 2.0,
                        _ => x0 + pad,
                    }
                };
                let baseline = top + li as f32 * lh + scaled.ascent();
                draw_line_of_text(
                    pixmap,
                    fonts,
                    &scaled,
                    &line_to_draw,
                    tx,
                    baseline,
                    (clip_x0, y0, clip_x1, y1),
                    text_color,
                );
                let deco_x = tx.max(clip_x0);
                let deco_w = lw.min(clip_x1 - deco_x);
                let deco_t = scale.max(1.0).round();
                if spec.underline {
                    let uy = (baseline + 1.5 * scale).min(y1 - 1.0);
                    fill_rect(pixmap, deco_x, uy, deco_w, deco_t, text_color);
                }
                if spec.strikeout {
                    // линия через середину строчных букв (~ x-height / 2)
                    let sy = baseline - scaled.ascent() * 0.35;
                    fill_rect(pixmap, deco_x, sy, deco_w, deco_t, text_color);
                }
            }
        }
    }
}

/// Смешивает пиксель текста с фоном по коверджу глифа.
#[inline]
fn blend_pixel(data: &mut [u8], width: usize, px: i32, py: i32, cov: f32, color: Rgb) {
    let a = (cov * 255.0) as u16;
    if a == 0 {
        return;
    }
    let idx = (py as usize * width + px as usize) * 4;
    let src = [color.0 as u16, color.1 as u16, color.2 as u16];
    for offset in 0..3 {
        let old = data[idx + offset] as u16;
        data[idx + offset] = ((old * (255 - a) + src[offset] * a) / 255) as u8;
    }
}

/// Рисует одну строку текста с альфа-смешиванием, с отсечением по clip.
#[allow(clippy::too_many_arguments)]
fn draw_line_of_text(
    pixmap: &mut Pixmap,
    fonts: &Fonts,
    scaled: &ScaledFace,
    line: &str,
    start_x: f32,
    baseline: f32,
    clip: (f32, f32, f32, f32),
    color: Rgb,
) {
    let width = pixmap.width() as i32;
    let height = pixmap.height() as i32;
    let (cx0, cy0, cx1, cy1) = clip;
    let data = pixmap.data_mut();
    let mut x = start_x;
    let mut prev = None;
    for ch in line.chars() {
        if ch.is_control() {
            prev = None;
            continue;
        }
        let g = scaled.glyph(ch);
        if let Some(p) = prev {
            x += scaled.kern(p, g);
        }
        let (bitmap, base_x, base_y) = fonts.glyph_bitmap(scaled, g, x, baseline);
        if let Some(bitmap) = bitmap {
            let min_x = (base_x + bitmap.min_x) as i32;
            let min_y = (base_y + bitmap.min_y) as i32;
            for (i, &cov) in bitmap.coverage.iter().enumerate() {
                let px = min_x + (i % bitmap.w) as i32;
                let py = min_y + (i / bitmap.w) as i32;
                if px < 0 || py < 0 || px >= width || py >= height {
                    continue;
                }
                let fx = px as f32;
                let fy = py as f32;
                if fx < cx0 - 0.5 || fx > cx1 - 0.5 || fy < cy0 || fy > cy1 {
                    continue;
                }
                blend_pixel(data, width as usize, px, py, cov, color);
            }
        }
        x += scaled.h_advance(g);
        prev = Some(g);
    }
}

/// Рисует блок вертикального текста (ориентация 90°, чтение снизу вверх).
/// Каждая строка текста становится вертикальной «колонкой»; колонки идут
/// слева направо.
#[allow(clippy::too_many_arguments)]
fn draw_vertical_text(
    pixmap: &mut Pixmap,
    fonts: &Fonts,
    spec: &FontSpec,
    scale: f32,
    lines: &[String],
    rect: (f32, f32, f32, f32),
    fmt: &CellFormat,
    color: Rgb,
) {
    let (x0, y0, x1, y1) = rect;
    let px = font_px(spec, scale);
    let scaled = fonts.scaled(spec, px);
    let lh = scaled.ascent() - scaled.descent();
    let pad = CELL_PAD_X * scale;
    let block_w = lines.len() as f32 * lh;
    // горизонтальное положение блока колонок
    let left = match fmt.h_align.unwrap_or(6) {
        0 | 4 => x0 + pad,
        2 => x1 - pad - block_w,
        _ => (x0 + x1 - block_w) / 2.0,
    };
    for (li, line) in lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let lw = text_width(fonts, spec, scale, line);
        // вертикальное положение: текст идёт от bottom вверх
        let bottom = match fmt.v_align.unwrap_or(24) {
            0 => y0 + pad + lw,
            8 => y1 - pad,
            _ => (y0 + y1 + lw) / 2.0,
        };
        let axis_x = left + li as f32 * lh + scaled.ascent();
        draw_line_of_text_rotated(pixmap, fonts, &scaled, line, axis_x, bottom, rect, color);
    }
}

/// Рисует строку текста, повёрнутую на 90° против часовой стрелки
/// (чтение снизу вверх). `axis_x` — x вертикальной базовой линии,
/// `bottom_y` — y начала текста.
#[allow(clippy::too_many_arguments)]
fn draw_line_of_text_rotated(
    pixmap: &mut Pixmap,
    fonts: &Fonts,
    scaled: &ScaledFace,
    line: &str,
    axis_x: f32,
    bottom_y: f32,
    clip: (f32, f32, f32, f32),
    color: Rgb,
) {
    let width = pixmap.width() as i32;
    let height = pixmap.height() as i32;
    let (cx0, cy0, cx1, cy1) = clip;
    let data = pixmap.data_mut();
    let mut pen = 0.0f32; // продвижение вдоль текста
    let mut prev = None;
    for ch in line.chars() {
        if ch.is_control() {
            prev = None;
            continue;
        }
        let g = scaled.glyph(ch);
        if let Some(p) = prev {
            pen += scaled.kern(p, g);
        }
        // глиф раскладывается в «горизонтальном» пространстве от (pen, 0),
        // затем каждый пиксель поворачивается: (dx, dy) → (dy, -dx)
        let (bitmap, base_x, base_y) = fonts.glyph_bitmap(scaled, g, pen, 0.0);
        if let Some(bitmap) = bitmap {
            let min_x = base_x + bitmap.min_x;
            let min_y = base_y + bitmap.min_y;
            for (i, &cov) in bitmap.coverage.iter().enumerate() {
                let dx = min_x + (i % bitmap.w) as f32;
                let dy = min_y + (i / bitmap.w) as f32;
                let tx = (axis_x + dy).round() as i32;
                let ty = (bottom_y - dx).round() as i32;
                if tx < 0 || ty < 0 || tx >= width || ty >= height {
                    continue;
                }
                let fx = tx as f32;
                let fy = ty as f32;
                if fx < cx0 - 0.5 || fx > cx1 - 0.5 || fy < cy0 || fy > cy1 {
                    continue;
                }
                blend_pixel(data, width as usize, tx, ty, cov, color);
            }
        }
        pen += scaled.h_advance(g);
        prev = Some(g);
    }
}

// ---------------------------------------------------------------------------
// Кодирование
// ---------------------------------------------------------------------------

/// Размер в мегабайтах с одним знаком после запятой (для сообщений).
fn megabytes(bytes: usize) -> String {
    format!("{:.1}", bytes as f64 / 1_000_000.0)
}

/// Кодирует растровый буфер в JPEG или PNG в памяти; результат больше
/// `max_bytes` считается ошибкой.
pub fn encode_to_vec(
    pixmap: &Pixmap,
    kind: &ImageKind,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let (w, h) = (pixmap.width(), pixmap.height());
    // Pixmap хранит премультиплицированный RGBA; все этапы рендера дают
    // непрозрачные пиксели (альфа 255), поэтому альфа просто отбрасывается.
    let mut rgb = Vec::with_capacity(w as usize * h as usize * 3);
    for p in pixmap.data().chunks_exact(4) {
        rgb.extend_from_slice(&p[..3]);
    }
    let mut out = Vec::new();
    match kind {
        ImageKind::Jpeg { quality } => {
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, *quality);
            image::ImageEncoder::write_image(encoder, &rgb, w, h, image::ExtendedColorType::Rgb8)
                .map_err(|e| format!("ошибка кодирования JPEG: {e}"))?;
        }
        ImageKind::Png { compression } => {
            use image::codecs::png::{CompressionType, FilterType, PngEncoder};
            let compression = match compression {
                PngCompression::Fast => CompressionType::Fast,
                PngCompression::Default => CompressionType::Default,
                PngCompression::Best => CompressionType::Best,
            };
            let encoder = PngEncoder::new_with_quality(&mut out, compression, FilterType::Adaptive);
            image::ImageEncoder::write_image(encoder, &rgb, w, h, image::ExtendedColorType::Rgb8)
                .map_err(|e| format!("ошибка кодирования PNG: {e}"))?;
        }
    }
    if out.len() > max_bytes {
        let hint = match kind {
            ImageKind::Jpeg { .. } => "уменьшите Масштаб или Качество",
            ImageKind::Png { .. } => "уменьшите Масштаб, выберите JPEG или СжатиеPNG = 2",
        };
        return Err(format!(
            "слишком большая картинка: {} МБ (предел МаксимальныйРазмерКартинки — {} МБ) — \
             {hint} либо увеличьте МаксимальныйРазмерКартинки",
            megabytes(out.len()),
            megabytes(max_bytes)
        ));
    }
    Ok(out)
}

/// Кодирует растровый буфер в файл JPEG или PNG (см. [`encode_to_vec`]).
///
/// Запись атомарная: данные пишутся во временный файл рядом с целевым и
/// затем переименовываются, поэтому при ошибке записи (нет места на диске и
/// т.п.) на месте картинки не остаётся обрезанный файл, а прежний файл с
/// тем же именем сохраняется.
pub fn encode_to_file(
    pixmap: &Pixmap,
    path: &Path,
    kind: &ImageKind,
    max_bytes: usize,
) -> Result<(), String> {
    let bytes = encode_to_vec(pixmap, kind, max_bytes)?;
    write_atomically(path, &bytes)
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let file_name = path
        .file_name()
        .ok_or_else(|| format!("не задано имя файла картинки: {}", path.display()))?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(file_name);
    tmp_name.push(format!(
        ".{}-{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp_path = path.with_file_name(tmp_name);

    let result = (|| {
        let mut file = std::fs::File::create(&tmp_path)
            .map_err(|e| format!("не удалось создать файл {}: {e}", path.display()))?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| format!("не удалось записать файл {}: {e}", path.display()))?;
        drop(file);
        std::fs::rename(&tmp_path, path)
            .map_err(|e| format!("не удалось записать файл {}: {e}", path.display()))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }
    result
}

// ---------------------------------------------------------------------------
// Тесты
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mxl_parser::{parse_file, FontFamily, FontSpec, Merge, Row};
    use std::path::PathBuf;

    fn mxl(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../mxl-files")
            .join(name)
    }

    /// Тестовые mxl-файлы не входят в репозиторий; без них тесты на
    /// реальных данных пропускаются (например, в CI).
    fn samples_available() -> bool {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../mxl-files");
        if dir.is_dir() {
            true
        } else {
            eprintln!("mxl-files/ отсутствует — тест на реальных файлах пропущен");
            false
        }
    }

    fn out_dir() -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/test-render");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Краткий конструктор линии для тестов.
    fn ls(kind: u32, thickness: u32) -> LineSpec {
        LineSpec { kind, thickness }
    }

    /// Маленький синтетический документ — для тестов без файлов-примеров.
    fn synthetic_doc() -> Document {
        Document {
            // линия 0 — сплошная толщиной 1
            lines: vec![ls(1, 1)],
            formats: vec![CellFormat {
                border_left: Some(0),
                border_top: Some(0),
                border_right: Some(0),
                border_bottom: Some(0),
                h_align: Some(6),
                ..CellFormat::default()
            }],
            rows: vec![Row {
                index: 0,
                format: 0,
                cells: vec![(
                    0,
                    Cell {
                        format: 1,
                        text: Some("Тестова комірка".to_string()),
                    },
                )],
            }],
            ..Document::default()
        }
    }

    #[test]
    fn renders_synthetic_doc_to_both_formats() {
        let doc = synthetic_doc();
        let jpg = out_dir().join("synthetic.jpg");
        let png = out_dir().join("synthetic.png");
        convert(&doc, &jpg, &ImageKind::Jpeg { quality: 90 }, 100).unwrap();
        convert(&doc, &png, &ImageKind::PNG, 100).unwrap();
        let img = image::open(&png).unwrap();
        assert!(img.width() > 10 && img.height() > 5);
        // текст и рамки дают тёмные пиксели
        let pm = render(&doc, 100).unwrap();
        let dark = pm.data().chunks_exact(4).filter(|p| p[0] < 128).count();
        assert!(dark > 50, "тёмных пикселей всего {dark}");
    }

    #[test]
    fn synthetic_scale_changes_dimensions() {
        let doc = synthetic_doc();
        let p100 = render(&doc, 100).unwrap();
        let p200 = render(&doc, 200).unwrap();
        assert!(p200.width() > p100.width() * 3 / 2);
    }

    #[test]
    fn border_pattern_follows_line_spec() {
        // Тип 0 — служебная рамка (нет линии/авто): 1С её не печатает.
        assert!(border_pattern(ls(0, 1), 1.0).is_none(), "тип 0 не рисуется");
        // Сплошная (тип 1) — без штриха и удвоения; толщина в px.
        let (t, dash, double) = border_pattern(ls(1, 2), 1.0).unwrap();
        assert_eq!(t, 2.0, "тип 1: толщина 2px");
        assert!(dash.is_none() && !double, "тип 1: сплошная");
        // Двойная (тип 3).
        let (_, _, double) = border_pattern(ls(3, 1), 1.0).unwrap();
        assert!(double, "тип 3 — двойная");
        // Пунктиры/точечная (типы 2, 4, 5, 6) имеют штрих.
        for kind in [2u32, 4, 5, 6] {
            let (_, dash, _) = border_pattern(ls(kind, 1), 1.0).unwrap();
            assert!(dash.is_some(), "тип {kind} — штриховая линия");
        }
    }

    #[test]
    fn resolve_line_falls_back_to_solid() {
        // Без таблицы линий установленная рамка всё равно рисуется сплошной.
        let doc = Document::default();
        assert_eq!(resolve_line(&doc, 0), ls(1, 1));
    }

    #[test]
    fn test_block_text_placement_repeating() {
        let doc = Document {
            formats: vec![CellFormat {
                h_align: Some(0),      // Left align
                text_control: Some(2), // Block / Забивать
                ..CellFormat::default()
            }],
            rows: vec![Row {
                index: 0,
                format: 0,
                cells: vec![(
                    0,
                    Cell {
                        format: 1,
                        text: Some("*".to_string()),
                    },
                )],
            }],
            ..Document::default()
        };

        let pm = render(&doc, 100).unwrap();

        let doc_normal = Document {
            formats: vec![CellFormat {
                h_align: Some(0),
                text_control: Some(0),
                ..CellFormat::default()
            }],
            rows: vec![Row {
                index: 0,
                format: 0,
                cells: vec![(
                    0,
                    Cell {
                        format: 1,
                        text: Some("*".to_string()),
                    },
                )],
            }],
            ..Document::default()
        };
        let pm_normal = render(&doc_normal, 100).unwrap();

        let dark_block = pm.data().chunks_exact(4).filter(|p| p[0] < 128).count();
        let dark_normal = pm_normal
            .data()
            .chunks_exact(4)
            .filter(|p| p[0] < 128)
            .count();

        assert!(
            dark_normal > 0,
            "should draw at least one pixel for normal text"
        );
        assert!(
            dark_block > dark_normal * 4,
            "block should repeat text and draw more pixels: {} vs {}",
            dark_block,
            dark_normal
        );
    }

    #[test]
    fn sparse_column_group_does_not_inflate_width() {
        // Полоса (доп. группа колонок) задаёт только колонку 0; строка,
        // использующая её, содержит ячейку лишь в колонке 0. Ширина картинки
        // должна определяться фактическим содержимым, а не «хвостом»
        // неопределённых колонок группы по умолчанию (иначе справа —
        // огромное пустое поле, как в исходном баге).
        let doc = Document {
            // формат 1 — ширина колонки 40 единиц
            formats: vec![CellFormat {
                width: Some(40),
                ..CellFormat::default()
            }],
            rows: vec![
                Row {
                    index: 0,
                    format: 0,
                    cells: vec![(
                        0,
                        Cell {
                            format: 0,
                            text: Some("A".into()),
                        },
                    )],
                },
                Row {
                    index: 1,
                    format: 0,
                    cells: vec![(
                        0,
                        Cell {
                            format: 0,
                            text: Some("B".into()),
                        },
                    )],
                },
            ],
            // группа по умолчанию задаёт 10 широких колонок
            default_group: crate::mxl_parser::ColumnGroup {
                columns: (0..10).map(|c| (c, 1)).collect(),
            },
            // доп. полоса — всего одна колонка
            extra_groups: vec![crate::mxl_parser::ColumnGroup {
                columns: vec![(0, 1)],
            }],
            // строка 1 отнесена к доп. полосе 0
            row_groups: vec![(1, 0)],
            ..Document::default()
        };
        let pm = render(&doc, 100).unwrap();
        // содержимое — одна колонка 40 единиц (~35px); ширина не должна
        // раздуваться до ~10 колонок (что давало бы 300+px)
        assert!(
            pm.width() < 120,
            "картинка неожиданно широкая: {}px (фантомные колонки?)",
            pm.width()
        );
    }

    #[test]
    fn solid_pattern_without_color_is_not_filled() {
        use crate::mxl_parser::ColorSpec;
        // Узор «сплошной» (0) без заданного цвета узора не должен закрашивать
        // ячейку чёрным (реальные счета/прайсы так помечают колонку цены).
        let doc = Document {
            colors: vec![ColorSpec::Rgb(255, 255, 255)],
            formats: vec![CellFormat {
                pattern: Some(0),  // сплошной
                bg_color: Some(0), // белый
                // pattern_color не задан
                ..CellFormat::default()
            }],
            rows: vec![Row {
                index: 0,
                format: 0,
                cells: vec![(
                    0,
                    Cell {
                        format: 1,
                        text: Some("X".into()),
                    },
                )],
            }],
            ..Document::default()
        };
        let pm = render(&doc, 100).unwrap();
        let total = (pm.width() * pm.height()) as usize;
        let black = pm
            .data()
            .chunks_exact(4)
            .filter(|p| p[0] < 30 && p[1] < 30 && p[2] < 30)
            .count();
        assert!(
            black * 2 < total,
            "ячейка ошибочно закрашена чёрным: {black}/{total}"
        );
    }

    #[test]
    fn empty_merge_tail_row_collapses() {
        use crate::mxl_parser::{ColumnGroup, Merge};
        // Документ из двух строк: колонка 0 объединена по вертикали (строки
        // 0–1). Вторая строка — только нижняя часть объединения; своего
        // текста в ней нет, поэтому она должна сворачиваться (как в 1С).
        let doc = |row1_text: Option<&str>| Document {
            default_group: ColumnGroup {
                columns: vec![(0, 0), (1, 0)],
            },
            merges: vec![Merge {
                left: 0,
                top: 0,
                right: 0,
                bottom: 1,
            }],
            rows: vec![
                Row {
                    index: 0,
                    format: 0,
                    cells: vec![
                        (
                            0,
                            Cell {
                                format: 0,
                                text: Some("Верх".into()),
                            },
                        ),
                        (
                            1,
                            Cell {
                                format: 0,
                                text: Some("A".into()),
                            },
                        ),
                    ],
                },
                Row {
                    index: 1,
                    format: 0,
                    cells: vec![
                        (
                            0,
                            Cell {
                                format: 0,
                                text: None,
                            },
                        ),
                        (
                            1,
                            Cell {
                                format: 0,
                                text: row1_text.map(str::to_string),
                            },
                        ),
                    ],
                },
            ],
            ..Document::default()
        };
        let collapsed = render(&doc(None), 100).unwrap();
        let expanded = render(&doc(Some("B")), 100).unwrap();
        assert!(
            collapsed.height() < expanded.height(),
            "пустая строка-продолжение объединения должна сворачиваться: {} vs {}",
            collapsed.height(),
            expanded.height()
        );
    }

    #[test]
    fn renders_akt_to_jpeg_and_png() {
        if !samples_available() {
            return;
        }
        let doc = parse_file(&mxl(
            "Акт сверки взаиморасчетов № 12 от 18 декабря 2023 г..mxl",
        ))
        .unwrap();
        let jpg = out_dir().join("akt.jpg");
        let png = out_dir().join("akt.png");
        convert(&doc, &jpg, &ImageKind::Jpeg { quality: 90 }, 100).unwrap();
        convert(&doc, &png, &ImageKind::PNG, 100).unwrap();
        // валидность: декодируется обратно и размеры совпадают
        let img = image::open(&jpg).unwrap();
        let img2 = image::open(&png).unwrap();
        assert_eq!(img.width(), img2.width());
        assert!(img.width() > 300 && img.height() > 300);
    }

    #[test]
    fn scale_changes_dimensions() {
        if !samples_available() {
            return;
        }
        let doc = parse_file(&mxl("Сетка.mxl")).unwrap();
        let p100 = render(&doc, 100).unwrap();
        let p200 = render(&doc, 200).unwrap();
        assert!(p200.width() > p100.width() * 3 / 2);
    }

    #[test]
    fn quality_changes_file_size() {
        if !samples_available() {
            return;
        }
        let doc = parse_file(&mxl("124.mxl")).unwrap();
        let lo = out_dir().join("q10.jpg");
        let hi = out_dir().join("q95.jpg");
        convert(&doc, &lo, &ImageKind::Jpeg { quality: 10 }, 100).unwrap();
        convert(&doc, &hi, &ImageKind::Jpeg { quality: 95 }, 100).unwrap();
        let s_lo = std::fs::metadata(&lo).unwrap().len();
        let s_hi = std::fs::metadata(&hi).unwrap().len();
        assert!(
            s_hi > s_lo,
            "quality 95 ({s_hi}) должно быть больше 10 ({s_lo})"
        );
    }

    #[test]
    fn renders_demo_with_colors() {
        if !samples_available() {
            return;
        }
        let path = mxl("ДемонстрацияВозможностейТабличногоДокумента.mxl");
        if !path.is_file() {
            eprintln!("демо-файл отсутствует — тест пропущен");
            return;
        }
        let doc = parse_file(&path).unwrap();
        let pm = render(&doc, 100).unwrap();
        let mut has_red = false;
        let mut has_title_blue = false;
        for p in pm.data().chunks_exact(4) {
            if p[0] > 200 && p[1] < 60 && p[2] < 60 {
                has_red = true;
            }
            // фон титула (31,78,121)
            if p[0].abs_diff(31) < 12 && p[1].abs_diff(78) < 12 && p[2].abs_diff(121) < 12 {
                has_title_blue = true;
            }
        }
        assert!(has_red, "нет красной заливки палитры");
        assert!(has_title_blue, "нет синего фона заголовка");
    }

    #[test]
    fn renders_patterns_and_picture() {
        if !samples_available() {
            return;
        }
        let path = mxl("ДемонстрацияУзоровРисунковБитов15-32.mxl");
        if !path.is_file() {
            eprintln!("демо-файл узоров отсутствует — тест пропущен");
            return;
        }
        let doc = parse_file(&path).unwrap();
        let pm = render(&doc, 100).unwrap();
        // ячейка «Узор 17» (строка 7, колонка 0): фон + штриховка цветом
        // узора дают минимум два разных цвета внутри ячейки
        let fonts = Fonts::load().unwrap();
        let prep = Prepared::new(&doc).unwrap();
        let layout = build_layout(&doc, &fonts, &prep, 1.0).unwrap();
        let (x0, x1) = layout.col_x(7, 0);
        let (y0, y1) = layout.row_range(7);
        let mut distinct = std::collections::HashSet::new();
        for py in (y0 as u32 + 2)..(y1 as u32 - 2) {
            for px in (x0 as u32 + 2)..(x1 as u32 - 2) {
                let i = ((py * pm.width() + px) * 4) as usize;
                distinct.insert((pm.data()[i], pm.data()[i + 1], pm.data()[i + 2]));
            }
        }
        assert!(distinct.len() >= 2, "узор не наложен: {distinct:?}");
        // картинка отрисована: в верхней зоне есть фирменный синий текст
        // логотипа (не из палитры документа)
        let non_white = pm
            .data()
            .chunks_exact(4)
            .filter(|p| p[0] < 250 || p[1] < 250 || p[2] < 250)
            .count();
        assert!(non_white > 10_000, "слишком пустой рендер: {non_white}");
    }

    #[test]
    fn renders_not_blank() {
        if !samples_available() {
            return;
        }
        let doc = parse_file(&mxl("Сетка.mxl")).unwrap();
        let pm = render(&doc, 100).unwrap();
        let dark = pm.data().chunks_exact(4).filter(|p| p[0] < 128).count();
        // на картинке должно быть заметное количество тёмных пикселей (текст)
        assert!(dark > 1000, "тёмных пикселей всего {dark}");
    }

    /// Документ из одной ячейки с текстом в строке `row`, колонке `col`.
    fn one_cell_doc(row: u32, col: u32) -> Document {
        Document {
            rows: vec![Row {
                index: row,
                format: 0,
                cells: vec![(
                    col,
                    Cell {
                        format: 0,
                        text: Some("x".to_string()),
                    },
                )],
            }],
            ..Document::default()
        }
    }

    #[test]
    fn huge_row_or_column_index_is_rejected_before_layout() {
        // раньше: выделение ~8 ГБ под границы строк и аварийное завершение
        for (row, col) in [
            (2_000_000_000, 0),
            (u32::MAX, 0),
            (0, 2_000_000_000),
            (0, u32::MAX),
        ] {
            let started = std::time::Instant::now();
            let e = render(&one_cell_doc(row, col), 100).unwrap_err();
            assert!(e.contains("слишком много"), "{e}");
            assert!(started.elapsed().as_secs() < 2);
        }
    }

    #[test]
    fn huge_merge_does_not_hang() {
        // раньше: карта «ячейка → объединение» на 10¹⁰ записей
        let mut doc = synthetic_doc();
        doc.merges.push(Merge {
            left: 0,
            top: 0,
            right: 60_000,
            bottom: 60_000,
        });
        let started = std::time::Instant::now();
        let e = render(&doc, 100).unwrap_err();
        assert!(e.contains("слишком большая картинка"), "{e}");
        assert!(started.elapsed().as_secs() < 2);
    }

    #[test]
    fn too_many_merge_index_entries_are_rejected() {
        // 200 перекрывающихся объединений по 60 000 строк каждое
        let mut doc = Document::default();
        for r in 0..60_000 {
            doc.rows.push(Row {
                index: r,
                format: 0,
                cells: vec![],
            });
        }
        for _ in 0..200 {
            doc.merges.push(Merge {
                left: 0,
                top: 0,
                right: 1,
                bottom: 59_999,
            });
        }
        let e = Prepared::new(&doc).err().unwrap();
        assert!(e.contains("объединений"), "{e}");
    }

    #[test]
    fn merge_index_finds_covering_merge() {
        let doc = Document {
            rows: (0..5)
                .map(|index| Row {
                    index,
                    format: 0,
                    cells: vec![],
                })
                .collect(),
            merges: vec![
                Merge {
                    left: 1,
                    top: 1,
                    right: 2,
                    bottom: 3,
                },
                Merge {
                    left: 4,
                    top: 0,
                    right: 6,
                    bottom: 0,
                },
            ],
            ..Document::default()
        };
        let index = MergeIndex::new(&doc).unwrap();
        assert_eq!(index.get(&(1, 1)), Some(&0));
        assert_eq!(index.get(&(3, 2)), Some(&0));
        assert_eq!(index.get(&(4, 2)), None);
        assert_eq!(index.get(&(2, 0)), None);
        assert_eq!(index.get(&(2, 3)), None);
        assert_eq!(index.get(&(0, 5)), Some(&1));
        assert!(!index.contains_key(&(0, 7)));
    }

    #[test]
    fn image_size_limits() {
        assert!(check_image_size(65_500, 3_000, "x").is_ok());
        assert!(check_image_size(65_501, 1, "x").is_err());
        let e = check_image_size(20_000, 20_000, "уменьшите Масштаб").unwrap_err();
        assert!(
            e.contains("Мпикс") && e.contains("уменьшите Масштаб"),
            "{e}"
        );
    }

    #[test]
    fn wrap_matches_reference_implementation() {
        // эталон — прежний алгоритм с полным пересчётом ширины кандидата
        fn reference(fonts: &Fonts, spec: &FontSpec, text: &str, max_w: f32) -> Vec<String> {
            let mut lines = Vec::new();
            for raw_line in text.split('\n') {
                let raw_line = raw_line.trim_end_matches('\r');
                if text_width(fonts, spec, 1.0, raw_line) <= max_w {
                    lines.push(raw_line.to_string());
                    continue;
                }
                let mut current = String::new();
                for word in raw_line.split(' ') {
                    let candidate = if current.is_empty() {
                        word.to_string()
                    } else {
                        format!("{current} {word}")
                    };
                    if text_width(fonts, spec, 1.0, &candidate) <= max_w || current.is_empty() {
                        current = candidate;
                    } else {
                        lines.push(std::mem::take(&mut current));
                        current = word.to_string();
                    }
                }
                if !current.is_empty() {
                    lines.push(current);
                }
            }
            if lines.is_empty() {
                lines.push(String::new());
            }
            lines
        }
        let fonts = Fonts::load().unwrap();
        let spec = FontSpec::default();
        let texts = [
            "Поставщик: ООО «Ромашка», ИНН 7700000000, г. Москва, ул. Ленина, д. 1",
            "  двойные  пробелы   и\r\nперевод строки",
            "оченьдлинноесловобезпробеловкотороене помещается",
            "",
            " ",
        ];
        for text in texts {
            for max_w in [4.0, 30.0, 80.0, 150.0, 1000.0] {
                assert_eq!(
                    wrap_text(&fonts, &spec, 1.0, text, max_w),
                    reference(&fonts, &spec, text, max_w),
                    "{text:?} @ {max_w}"
                );
            }
        }
    }

    #[test]
    fn encode_to_file_replaces_file_atomically() {
        let dir = out_dir();
        let path = dir.join("atomic.png");
        std::fs::write(&path, b"old").unwrap();
        let pm = render(&synthetic_doc(), 100).unwrap();
        encode_to_file(&pm, &path, &ImageKind::PNG, DEFAULT_MAX_IMAGE_BYTES).unwrap();
        assert!(std::fs::read(&path).unwrap().starts_with(b"\x89PNG"));
        // при ошибке (превышен предел размера) прежний файл не тронут
        let e = encode_to_file(&pm, &path, &ImageKind::PNG, 10).unwrap_err();
        assert!(e.contains("МаксимальныйРазмерКартинки"), "{e}");
        assert!(std::fs::read(&path).unwrap().starts_with(b"\x89PNG"));
        // временных файлов не осталось
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".atomic.png"))
            .collect();
        assert!(leftovers.is_empty());
        // в несуществующий каталог — понятная ошибка
        let e = encode_to_file(
            &pm,
            &dir.join("no/such/dir.png"),
            &ImageKind::PNG,
            DEFAULT_MAX_IMAGE_BYTES,
        )
        .unwrap_err();
        assert!(e.contains("не удалось создать файл"), "{e}");
    }

    fn demo_doc(name: &str) -> Document {
        parse_file(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../mxl-demo-collection")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn split_by_breaks_ignores_breaks_outside_range() {
        assert_eq!(split_by_breaks(0, 10, &[]), vec![(0, 10)]);
        assert_eq!(
            split_by_breaks(0, 10, &[3, 7]),
            vec![(0, 3), (4, 7), (8, 10)]
        );
        // разрыв на последней строке и вне диапазона — не делит
        assert_eq!(split_by_breaks(2, 10, &[1, 10, 15]), vec![(2, 10)]);
    }

    #[test]
    fn pages_follow_print_area_and_breaks() {
        let doc = demo_doc("ДемонстрацияПечатиРазрывов_A4_Книжная.mxl");
        let all = pages(&doc);
        assert_eq!(
            all,
            vec![
                PageRange {
                    rows: (0, 24),
                    cols: (0, 11)
                },
                PageRange {
                    rows: (25, 35),
                    cols: (0, 11)
                },
                PageRange {
                    rows: (36, 48),
                    cols: (0, 11)
                },
            ]
        );
        let full = render(&doc, 100).unwrap();
        let heights: Vec<u32> = (1..=3)
            .map(|n| {
                let p = render_page(&doc, 100, n).unwrap();
                assert_eq!(p.width(), full.width(), "страница {n}");
                p.height()
            })
            .collect();
        // страницы вместе покрывают документ (у каждой поля 1 + 2 px)
        assert_eq!(heights.iter().sum::<u32>(), full.height() + 2 * 3);
        let e = render_page(&doc, 100, 4).unwrap_err();
        assert!(e.contains("всего 3 стр."), "{e}");
        assert!(render_page(&doc, 100, 0).is_err());
    }

    #[test]
    fn page_is_a_crop_of_the_full_render() {
        let doc = demo_doc("ДемонстрацияПечатиРазрывов_A4_Книжная.mxl");
        let full = render(&doc, 100).unwrap();
        let page = render_page(&doc, 100, 2).unwrap();
        // строка 26 (индекс 25) начинается на row_y[25]; страница — с поля 1 px
        let fonts = Fonts::load().unwrap();
        let prep = Prepared::new(&doc).unwrap();
        let layout = build_layout(&doc, &fonts, &prep, 1.0).unwrap();
        let top = layout.row_y[25] as u32 - 1;
        let w = full.width() as usize * 4;
        // внутренние строки страницы совпадают с полной картинкой; глифы
        // могут отличаться на 1–2 единицы яркости — базовая линия текста на
        // странице сдвинута на целое число px, но считается во float с
        // другим округлением дробной части
        let mut max_diff = 0u8;
        let mut differing = 0usize;
        for y in 3..page.height() - 3 {
            let src = &full.data()[(top + y) as usize * w..][..w];
            let dst = &page.data()[y as usize * w..][..w];
            for (a, b) in src.iter().zip(dst) {
                max_diff = max_diff.max(a.abs_diff(*b));
                differing += (a != b) as usize;
            }
        }
        assert!(max_diff <= 2, "максимальное отличие {max_diff}");
        assert!(differing < 500, "отличающихся значений {differing}");
    }

    #[test]
    fn document_without_print_area_is_one_page() {
        let doc = synthetic_doc();
        assert_eq!(pages(&doc).len(), 1);
        let p = render_page(&doc, 100, 1).unwrap();
        let full = render(&doc, 100).unwrap();
        assert_eq!((p.width(), p.height()), (full.width(), full.height()));
    }

    #[test]
    fn page_of_a_document_too_large_for_one_image() {
        // 6000 строк с разрывом через каждые 100: целиком не помещается,
        // постранично — помещается
        let mut doc = Document {
            formats: vec![CellFormat {
                width: Some(4),
                ..CellFormat::default()
            }],
            ..Document::default()
        };
        for r in 0..6000 {
            doc.rows.push(Row {
                index: r,
                format: if r % 100 == 99 { 1 } else { 0 },
                cells: vec![(
                    0,
                    Cell {
                        format: 0,
                        text: Some(format!("строка {r}")),
                    },
                )],
            });
        }
        assert!(render(&doc, 100).unwrap_err().contains("НомерСтраницы"));
        assert_eq!(pages(&doc).len(), 60);
        let p = render_page(&doc, 100, 60).unwrap();
        assert!(p.height() < 3000, "{}", p.height());
    }

    #[test]
    fn font_families_and_fallback_glyphs() {
        let fonts = Fonts::load().unwrap();
        let mono = FontSpec {
            family: FontFamily::Mono,
            ..FontSpec::default()
        };
        // моноширинный: одинаковая ширина узких и широких букв
        let narrow = text_width(&fonts, &mono, 1.0, "iiii");
        let wide = text_width(&fonts, &mono, 1.0, "MMMM");
        assert!((narrow - wide).abs() < 0.01, "{narrow} != {wide}");
        let sans = FontSpec::default();
        assert!(text_width(&fonts, &sans, 1.0, "iiii") < text_width(&fonts, &sans, 1.0, "MMMM"));
        let serif = FontSpec {
            family: FontFamily::Serif,
            ..FontSpec::default()
        };
        assert_ne!(
            text_width(&fonts, &serif, 1.0, "Счёт на оплату"),
            text_width(&fonts, &sans, 1.0, "Счёт на оплату")
        );
        // символа «✓» нет в Liberation — берётся из запасного шрифта
        let sf = fonts.scaled(&sans, 20.0);
        assert!(sf.glyph('✓').0, "✓ должен браться из запасного шрифта");
        assert!(!sf.glyph('Ж').0);
        assert!(text_width(&fonts, &sans, 1.0, "✓") > 1.0);
    }

    #[test]
    fn fallback_glyph_is_drawn() {
        let mut doc = synthetic_doc();
        doc.rows[0].cells[0].1.text = Some("✓✓✓".to_string());
        let pm = render(&doc, 300).unwrap();
        let dark = pm.data().chunks_exact(4).filter(|p| p[0] < 100).count();
        // рамка + глифы (без запасного шрифта рисовались бы пустые .notdef)
        let mut empty = synthetic_doc();
        empty.rows[0].cells[0].1.text = None;
        let frame = render(&empty, 300)
            .unwrap()
            .data()
            .chunks_exact(4)
            .filter(|p| p[0] < 100)
            .count();
        assert!(dark > frame + 100, "глифов нет: {dark} vs {frame}");
    }

    #[test]
    fn picture_downscaling_is_filtered() {
        // шахматная доска 1 px, уменьшенная вдвое, даёт серый, а не
        // чёрный/белый (как при выборке ближайшего пикселя)
        let board = image::RgbaImage::from_fn(64, 64, |x, y| {
            if (x + y) % 2 == 0 {
                image::Rgba([0, 0, 0, 255])
            } else {
                image::Rgba([255, 255, 255, 255])
            }
        });
        let mut pm = Pixmap::new(40, 40).unwrap();
        pm.fill(tiny_skia::Color::WHITE);
        blit_image(&mut pm, &board, 4.0, 4.0, 36.0, 36.0);
        let i = ((20 * 40 + 20) * 4) as usize;
        let v = pm.data()[i];
        assert!((60..=200).contains(&v), "ожидался серый, получено {v}");
        // за пределами картинки — фон
        assert_eq!(pm.data()[0], 255);
    }
}
