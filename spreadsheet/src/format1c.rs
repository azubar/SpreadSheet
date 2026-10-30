//! Представление значений ячеек (число, дата, булево) по форматной строке
//! 1С — аналог функции `Формат()` для русской локали.
//!
//! Поддерживаемые параметры (русские и английские имена):
//! * число: `ЧЦ`/`ND` (всего цифр, для `ЧВН`), `ЧДЦ`/`NFD` (дробных цифр),
//!   `ЧС`/`NS` (сдвиг разрядов), `ЧРД`/`NDS` (разделитель дробной части),
//!   `ЧРГ`/`NGS` (разделитель групп), `ЧГ`/`NG` (группировка), `ЧН`/`NZ`
//!   (представление нуля), `ЧВН`/`NLZ` (ведущие нули), `ЧО`/`NN`
//!   (представление отрицательных);
//! * дата: `ДФ`/`DF` (формат), `ДЛФ`/`DLF` (локальный формат: D, DD, T, DT,
//!   DDT), `ДП`/`DE` (пустая дата);
//! * булево: `БЛ`/`BF`, `БИ`/`BT`.
//!
//! Без форматной строки — как `Строка()` в 1С: число с группами разрядов,
//! ноль — пустая строка, дата — `дд.ММ.гггг Ч:мм:сс`, булево — «Да»/«Нет».
//!
//! Значение числа хранится строкой десятичной записи (как в файле MXL), и
//! округление выполняется над цифрами — без ошибок двоичной арифметики.

use std::collections::HashMap;

/// Значение ячейки (типизированное значение `{тип, значение}` в MXL).
#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    /// Число — десятичная запись, например `-1234.5`
    Number(String),
    /// Дата и время: год, месяц, день, час, минута, секунда
    Date([u32; 6]),
    Bool(bool),
    Str(String),
}

/// Разделитель групп разрядов по умолчанию (неразрывный пробел, как в 1С).
const NBSP: &str = "\u{a0}";

/// Параметры форматной строки: ключ (в верхнем регистре, латиница) → значение.
fn parse_format(format: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    let chars: Vec<char> = format.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // имя параметра
        while i < chars.len() && (chars[i] == ';' || chars[i].is_whitespace()) {
            i += 1;
        }
        let start = i;
        while i < chars.len() && chars[i] != '=' && chars[i] != ';' {
            i += 1;
        }
        let name: String = chars[start..i]
            .iter()
            .collect::<String>()
            .trim()
            .to_uppercase();
        if i >= chars.len() || chars[i] != '=' {
            continue;
        }
        i += 1;
        // значение: в кавычках (кавычка удваивается) или до «;»
        let mut value = String::new();
        if i < chars.len() && (chars[i] == '\'' || chars[i] == '"') {
            let quote = chars[i];
            i += 1;
            while i < chars.len() {
                if chars[i] == quote {
                    if i + 1 < chars.len() && chars[i + 1] == quote {
                        value.push(quote);
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                value.push(chars[i]);
                i += 1;
            }
            while i < chars.len() && chars[i] != ';' {
                i += 1;
            }
        } else {
            while i < chars.len() && chars[i] != ';' {
                value.push(chars[i]);
                i += 1;
            }
            value = value.trim().to_string();
        }
        if !name.is_empty() {
            params.insert(canonical(&name).to_string(), value);
        }
    }
    params
}

/// Латинское имя параметра.
fn canonical(name: &str) -> &str {
    match name {
        "ЧЦ" => "ND",
        "ЧДЦ" => "NFD",
        "ЧС" => "NS",
        "ЧРД" => "NDS",
        "ЧРГ" => "NGS",
        "ЧГ" => "NG",
        "ЧН" => "NZ",
        "ЧВН" => "NLZ",
        "ЧО" => "NN",
        "ДФ" => "DF",
        "ДЛФ" => "DLF",
        "ДП" => "DE",
        "БЛ" => "BF",
        "БИ" => "BT",
        other => other,
    }
}

/// Текст значения по форматной строке.
pub fn format_value(value: &CellValue, format: &str) -> String {
    let params = parse_format(format);
    match value {
        CellValue::Number(n) => format_number(n, &params),
        CellValue::Date(d) => format_date(d, &params),
        CellValue::Bool(b) => match (b, params.get("BT"), params.get("BF")) {
            (true, Some(t), _) => t.clone(),
            (false, _, Some(f)) => f.clone(),
            (true, None, _) => "Да".to_string(),
            (false, _, None) => "Нет".to_string(),
        },
        CellValue::Str(s) => s.clone(),
    }
}

/// Десятичное число: знак, цифры целой части, цифры дробной части.
struct Decimal {
    negative: bool,
    int: Vec<u8>,
    frac: Vec<u8>,
}

impl Decimal {
    fn parse(text: &str) -> Option<Decimal> {
        let text = text.trim();
        let (negative, digits) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text.strip_prefix('+').unwrap_or(text)),
        };
        // экспоненциальная запись встречается редко — через f64
        if digits.contains(['e', 'E']) {
            let v: f64 = text.parse().ok()?;
            return Decimal::parse(&format!("{v:.15}"));
        }
        let (int, frac) = digits.split_once('.').unwrap_or((digits, ""));
        if int.is_empty() && frac.is_empty()
            || !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let mut d = Decimal {
            negative,
            int: int.bytes().map(|b| b - b'0').collect(),
            frac: frac.bytes().map(|b| b - b'0').collect(),
        };
        d.normalize();
        Some(d)
    }

    fn normalize(&mut self) {
        while self.int.len() > 1 && self.int[0] == 0 {
            self.int.remove(0);
        }
        if self.int.is_empty() {
            self.int.push(0);
        }
        while self.frac.last() == Some(&0) {
            self.frac.pop();
        }
        if self.is_zero() {
            self.negative = false;
        }
    }

    fn is_zero(&self) -> bool {
        self.int.iter().chain(&self.frac).all(|&d| d == 0)
    }

    /// Делит на 10^shift (сдвиг разрядов вправо; отрицательный — влево).
    fn shift(&mut self, shift: i32) {
        let mut digits: Vec<u8> = self.int.iter().chain(&self.frac).copied().collect();
        let mut point = self.int.len() as i32 - shift;
        if point < 0 {
            let pad = (-point) as usize;
            digits.splice(0..0, std::iter::repeat_n(0, pad));
            point = 0;
        }
        while point as usize > digits.len() {
            digits.push(0);
        }
        self.frac = digits.split_off(point as usize);
        self.int = digits;
        self.normalize();
    }

    /// Округляет до `places` дробных цифр (половина — от нуля).
    fn round(&mut self, places: usize) {
        if self.frac.len() <= places {
            return;
        }
        let round_up = self.frac[places] >= 5;
        self.frac.truncate(places);
        if round_up {
            let mut carry = true;
            for d in self.frac.iter_mut().rev().chain(self.int.iter_mut().rev()) {
                if !carry {
                    break;
                }
                if *d == 9 {
                    *d = 0;
                } else {
                    *d += 1;
                    carry = false;
                }
            }
            if carry {
                self.int.insert(0, 1);
            }
        }
        self.normalize();
    }
}

fn format_number(text: &str, params: &HashMap<String, String>) -> String {
    let Some(mut d) = Decimal::parse(text) else {
        return text.to_string();
    };
    let num = |key: &str| params.get(key).and_then(|v| v.trim().parse::<i64>().ok());
    if let Some(shift) = num("NS") {
        d.shift(shift.clamp(-100, 100) as i32);
    }
    let fraction_digits = num("NFD").map(|n| n.clamp(0, 100) as usize);
    if let Some(places) = fraction_digits {
        d.round(places);
    }
    if d.is_zero() {
        return params.get("NZ").cloned().unwrap_or_default();
    }
    // целая часть: ведущие нули и группы разрядов
    let mut int: String = d.int.iter().map(|&x| (b'0' + x) as char).collect();
    if params.contains_key("NLZ") {
        if let Some(total) = num("ND") {
            let width = (total - fraction_digits.unwrap_or(0) as i64).max(0) as usize;
            while int.len() < width {
                int.insert(0, '0');
            }
        }
    }
    let group = match params.get("NG").map(|v| v.trim()) {
        Some(g) => g
            .split(',')
            .next()
            .and_then(|n| n.trim().parse::<usize>().ok())
            .unwrap_or(0),
        None => 3,
    };
    let group_sep = params.get("NGS").map_or(NBSP, String::as_str);
    if group > 0 && int.len() > group {
        let digits: Vec<char> = int.chars().collect();
        let mut grouped = String::new();
        for (i, c) in digits.iter().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(group) {
                grouped.push_str(group_sep);
            }
            grouped.push(*c);
        }
        int = grouped;
    }
    // дробная часть
    let mut frac: String = d.frac.iter().map(|&x| (b'0' + x) as char).collect();
    if let Some(places) = fraction_digits {
        while frac.len() < places {
            frac.push('0');
        }
    }
    let decimal_sep = params.get("NDS").map_or(",", String::as_str);
    let body = if frac.is_empty() {
        int
    } else {
        format!("{int}{decimal_sep}{frac}")
    };
    if !d.negative {
        return body;
    }
    match num("NN").unwrap_or(1) {
        0 => format!("({body})"),
        2 => format!("- {body}"),
        3 => format!("{body}-"),
        4 => format!("{body} -"),
        _ => format!("-{body}"),
    }
}

const MONTHS_GENITIVE: [&str; 12] = [
    "января",
    "февраля",
    "марта",
    "апреля",
    "мая",
    "июня",
    "июля",
    "августа",
    "сентября",
    "октября",
    "ноября",
    "декабря",
];
const MONTHS_NOMINATIVE: [&str; 12] = [
    "Январь",
    "Февраль",
    "Март",
    "Апрель",
    "Май",
    "Июнь",
    "Июль",
    "Август",
    "Сентябрь",
    "Октябрь",
    "Ноябрь",
    "Декабрь",
];
const MONTHS_SHORT: [&str; 12] = [
    "янв", "фев", "мар", "апр", "мая", "июн", "июл", "авг", "сен", "окт", "ноя", "дек",
];
const WEEKDAYS: [&str; 7] = [
    "понедельник",
    "вторник",
    "среда",
    "четверг",
    "пятница",
    "суббота",
    "воскресенье",
];
const WEEKDAYS_SHORT: [&str; 7] = ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Вс"];

/// День недели (0 — понедельник) по григорианскому календарю.
fn weekday(year: u32, month: u32, day: u32) -> usize {
    // алгоритм Сакамото: 0 — воскресенье
    const T: [u32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    let sunday_based = (y + y / 4 - y / 100 + y / 400 + T[(month as usize - 1) % 12] + day) % 7;
    ((sunday_based + 6) % 7) as usize
}

fn format_date(d: &[u32; 6], params: &HashMap<String, String>) -> String {
    let [year, month, day, hour, minute, second] = *d;
    if year <= 1 && month <= 1 && day <= 1 && hour == 0 && minute == 0 && second == 0 {
        return params.get("DE").cloned().unwrap_or_default();
    }
    if let Some(pattern) = params.get("DF") {
        return date_pattern(d, pattern);
    }
    let pattern = match params
        .get("DLF")
        .map(|v| v.trim().to_uppercase())
        .as_deref()
    {
        Some("D") => "dd.MM.yyyy",
        Some("DD") => "d MMMM yyyy 'г.'",
        Some("T") => "H:mm:ss",
        Some("DT") => "dd.MM.yyyy H:mm:ss",
        Some("DDT") => "d MMMM yyyy 'г.' H:mm:ss",
        _ => "dd.MM.yyyy H:mm:ss",
    };
    date_pattern(d, pattern)
}

/// Дата по шаблону `ДФ`: d, dd, ddd, dddd, M, MM, MMM, MMMM, y, yy, yyyy,
/// h, hh, H, HH, m, mm, s, ss, tt; текст в кавычках — как есть.
fn date_pattern(d: &[u32; 6], pattern: &str) -> String {
    let [year, month, day, hour, minute, second] = *d;
    let m = (month.clamp(1, 12) - 1) as usize;
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' || c == '"' {
            i += 1;
            while i < chars.len() && chars[i] != c {
                out.push(chars[i]);
                i += 1;
            }
            i += 1;
            continue;
        }
        let mut run = 1;
        while i + run < chars.len() && chars[i + run] == c {
            run += 1;
        }
        let hour12 = if hour % 12 == 0 { 12 } else { hour % 12 };
        let piece = match (c, run) {
            ('d', 1) => day.to_string(),
            ('d', 2) => format!("{day:02}"),
            ('d', 3) => WEEKDAYS_SHORT[weekday(year, month, day)].to_string(),
            ('d', _) => WEEKDAYS[weekday(year, month, day)].to_string(),
            ('M', 1) => month.to_string(),
            ('M', 2) => format!("{month:02}"),
            ('M', 3) => MONTHS_SHORT[m].to_string(),
            ('M', _) => {
                // родительный падеж после числа дня («19 июля»), иначе
                // именительный («Июль 2026»)
                if out.trim_end().ends_with(|ch: char| ch.is_ascii_digit()) {
                    MONTHS_GENITIVE[m].to_string()
                } else {
                    MONTHS_NOMINATIVE[m].to_string()
                }
            }
            ('y', 1) => (year % 100).to_string(),
            ('y', 2) => format!("{:02}", year % 100),
            ('y', _) => format!("{year:04}"),
            ('h', 1) => hour12.to_string(),
            ('h', _) => format!("{hour12:02}"),
            ('H', 1) => hour.to_string(),
            ('H', _) => format!("{hour:02}"),
            ('m', 1) => minute.to_string(),
            ('m', _) => format!("{minute:02}"),
            ('s', 1) => second.to_string(),
            ('s', _) => format!("{second:02}"),
            ('t', _) => if hour < 12 { "AM" } else { "PM" }.to_string(),
            _ => std::iter::repeat_n(c, run).collect(),
        };
        out.push_str(&piece);
        i += run;
    }
    out
}

/// Дата из записи `ГГГГММДДччммсс` (14 цифр).
pub fn parse_date(text: &str) -> Option<[u32; 6]> {
    let t = text.trim();
    if t.len() != 14 || !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let part = |a: usize, b: usize| t[a..b].parse::<u32>().ok();
    Some([
        part(0, 4)?,
        part(4, 6)?,
        part(6, 8)?,
        part(8, 10)?,
        part(10, 12)?,
        part(12, 14)?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(v: &str, f: &str) -> String {
        format_value(&CellValue::Number(v.into()), f).replace(NBSP, " ")
    }

    fn date() -> CellValue {
        CellValue::Date([2026, 7, 19, 14, 30, 5])
    }

    #[test]
    fn numbers() {
        assert_eq!(n("1234567.891", ""), "1 234 567,891");
        assert_eq!(n("1234567.891", "ЧДЦ=2"), "1 234 567,89");
        assert_eq!(n("1234567.891", "ЧДЦ=2; ЧРД=.; ЧРГ=','"), "1,234,567.89");
        assert_eq!(n("1234567.891", "ЧДЦ=2; ЧГ=0"), "1234567,89");
        // ведущие нули тоже делятся на группы (сверяется по эталону 1С)
        assert_eq!(n("12.5", "ЧЦ=6; ЧДЦ=2; ЧВН="), "0 012,50");
        assert_eq!(n("12.5", "ЧЦ=6; ЧДЦ=2; ЧВН=; ЧГ=0"), "0012,50");
        assert_eq!(n("0.125", "ЧДЦ=2"), "0,13");
        assert_eq!(n("9.995", "ЧДЦ=2"), "10,00");
        assert_eq!(n("1500000", "ЧС=3; ЧДЦ=1"), "1 500,0");
        assert_eq!(n("1234", "NFD=2; NDS=."), "1 234.00");
    }

    #[test]
    fn negative_and_zero() {
        assert_eq!(n("-1234.5", "ЧДЦ=2"), "-1 234,50");
        assert_eq!(n("-1234.5", "ЧДЦ=2; ЧО=0"), "(1 234,50)");
        assert_eq!(n("-1234.5", "ЧДЦ=2; ЧО=3"), "1 234,50-");
        assert_eq!(n("0", "ЧДЦ=2"), "");
        assert_eq!(n("0", "ЧДЦ=2; ЧН=-"), "-");
        assert_eq!(n("0", "ЧН='Ноль'"), "Ноль");
        // округление до нуля даёт ноль
        assert_eq!(n("0.001", "ЧДЦ=2"), "");
        assert_eq!(n("-0.001", "ЧДЦ=2; ЧН=0"), "0");
    }

    #[test]
    fn dates() {
        let f = |fmt: &str| format_value(&date(), fmt);
        assert_eq!(f(""), "19.07.2026 14:30:05");
        assert_eq!(f("ДФ=dd.MM.yyyy"), "19.07.2026");
        assert_eq!(f("ДФ='d MMMM yyyy \"г.\"'"), "19 июля 2026 г.");
        assert_eq!(f("ДФ='MMMM yyyy'"), "Июль 2026");
        assert_eq!(f("ДФ=dddd"), "воскресенье");
        assert_eq!(f("ДЛФ=D"), "19.07.2026");
        assert_eq!(f("ДЛФ=DD"), "19 июля 2026 г.");
        assert_eq!(f("ДЛФ=T"), "14:30:05");
        assert_eq!(f("ДЛФ=DT"), "19.07.2026 14:30:05");
        assert_eq!(f("DF='hh:mm tt'"), "02:30 PM");
        let empty = CellValue::Date([1, 1, 1, 0, 0, 0]);
        assert_eq!(format_value(&empty, ""), "");
        assert_eq!(format_value(&empty, "ДП='не указана'"), "не указана");
    }

    #[test]
    fn booleans_and_parsing() {
        assert_eq!(format_value(&CellValue::Bool(true), ""), "Да");
        assert_eq!(
            format_value(&CellValue::Bool(false), "БЛ=Нет; БИ=Да"),
            "Нет"
        );
        assert_eq!(format_value(&CellValue::Bool(true), "БЛ=Off; БИ=On"), "On");
        assert_eq!(parse_date("20260719143005"), Some([2026, 7, 19, 14, 30, 5]));
        assert_eq!(parse_date("2026"), None);
        assert_eq!(weekday(2026, 7, 19), 6);
        assert_eq!(weekday(2000, 1, 1), 5);
    }
}
