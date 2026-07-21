"""Генерация MXL с узорами, встроенными рисунками и битами 15-32."""

from __future__ import annotations

import argparse
from pathlib import Path

import win32com.client

from generate_mxl_demo import color, font, set_area, set_border


PATTERNS = [("БезУзора", "Без узора"), ("Сплошной", "Сплошная заливка")]
PATTERNS.extend((f"Узор{index}", f"Узор {index}") for index in range(1, 18))


def set_pattern(area, connection, pattern_name: str, background_rgb, pattern_rgb):
    area.Узор = getattr(
        connection.ТипУзораТабличногоДокумента, pattern_name
    )
    area.ЦветФона = color(connection, background_rgb)
    area.ЦветУзора = color(connection, pattern_rgb)


def create_drawing(
    document,
    connection,
    drawing_type,
    name: str,
    begin_row: int,
    begin_column: int,
    end_row: int,
    end_column: int,
    text: str,
):
    drawing = document.Рисунки.Добавить(
        getattr(connection.ТипРисункаТабличногоДокумента, drawing_type),
        begin_row,
        begin_column,
        end_row,
        end_column,
    )
    drawing.Имя = name
    drawing.Текст = text
    drawing.ЦветФона = color(connection, (255, 255, 255))
    drawing.ЦветУзора = color(connection, (31, 78, 121))
    drawing.Узор = connection.ТипУзораТабличногоДокумента.Узор3
    drawing.ЦветТекста = color(connection, (31, 31, 31))
    drawing.АвтоРазмер = False
    return drawing


def bit_values(bit_number: int) -> tuple[int, str, int]:
    mask = 1 << (bit_number - 1)
    signed_int32 = mask if mask < (1 << 31) else mask - (1 << 32)
    return mask, f"0x{mask:08X}", signed_int32


def generate(base_path: Path, output_path: Path, image_path: Path) -> dict[str, int | str]:
    output_path.parent.mkdir(parents=True, exist_ok=True)
    if not image_path.is_file():
        raise FileNotFoundError(f"Не найден файл картинки: {image_path}")

    connector = win32com.client.Dispatch("V83.COMConnector")
    connection = connector.Connect(f'File="{base_path}";')
    document = connection.NewObject("SpreadsheetDocument")

    total_columns = 6
    for column in range(1, total_columns + 1):
        document.Area(1, column, 1, column).ШиринаКолонки = 22

    title = document.Area(1, 1, 1, total_columns)
    title.Объединить()
    set_area(
        title,
        text="Демонстрация узоров, встроенных рисунков и битов 15-32",
        background=color(connection, (31, 78, 121)),
        text_color=color(connection, (255, 255, 255)),
        area_font=font(connection, "Arial", 15, bold=True),
    )
    document.Area(1, 1, 1, total_columns).ВысотаСтроки = 30

    subtitle = document.Area(2, 1, 2, total_columns)
    subtitle.Объединить()
    set_area(
        subtitle,
        text=(
            "Узоры задаются перечислением ТипУзораТабличногоДокумента; "
            "рисунки добавляются в коллекцию Рисунки; биты подписаны "
            "с нумерацией от 1"
        ),
        background=color(connection, (221, 235, 247)),
        area_font=font(connection, "Arial", 10, italic=True),
    )
    document.Area(2, 1, 2, total_columns).ВысотаСтроки = 34

    pattern_title_row = 4
    pattern_title = document.Area(pattern_title_row, 1, pattern_title_row, total_columns)
    pattern_title.Объединить()
    set_area(
        pattern_title,
        text="Узоры заливки: имя узора, цвет фона и цвет узора",
        background=color(connection, (242, 242, 242)),
        area_font=font(connection, "Arial", 11, bold=True),
    )

    pattern_start_row = pattern_title_row + 1
    pattern_columns = total_columns
    for index, (pattern_name, caption) in enumerate(PATTERNS):
        row = pattern_start_row + index // pattern_columns
        column = 1 + index % pattern_columns
        area = document.Area(row, column, row, column)
        background = (255, 255, 255) if index % 2 == 0 else (221, 235, 247)
        pattern_color = (31, 78, 121) if index % 2 == 0 else (192, 0, 0)
        set_area(
            area,
            text=f"{caption}\n{pattern_name}",
            text_color=color(connection, (31, 31, 31)),
            area_font=font(connection, "Arial", 8, bold=True),
        )
        set_pattern(area, connection, pattern_name, background, pattern_color)
        set_border(area, connection, "Сплошная", 1)
    pattern_end_row = pattern_start_row + (
        len(PATTERNS) + pattern_columns - 1
    ) // pattern_columns - 1
    for row in range(pattern_start_row, pattern_end_row + 1):
        document.Area(row, 1, row, total_columns).ВысотаСтроки = 38

    drawings_title_row = pattern_end_row + 2
    drawings_title = document.Area(
        drawings_title_row, 1, drawings_title_row, total_columns
    )
    drawings_title.Объединить()
    set_area(
        drawings_title,
        text="Встроенные объекты-рисунки: картинка, текст, прямоугольник и диаграмма",
        background=color(connection, (217, 225, 242)),
        area_font=font(connection, "Arial", 11, bold=True),
    )

    drawing_start_row = drawings_title_row + 1
    drawing_picture = document.Рисунки.Добавить(
        connection.ТипРисункаТабличногоДокумента.Картинка,
        drawing_start_row,
        1,
        drawing_start_row + 4,
        2,
    )
    drawing_picture.Имя = "ВстроеннаяКартинка"
    drawing_picture.Картинка = connection.NewObject("Picture", str(image_path))
    drawing_picture.АвтоРазмер = False
    drawing_picture.Ширина = 120
    drawing_picture.Высота = 90

    drawing_text = create_drawing(
        document,
        connection,
        "Текст",
        "РисунокТекст",
        drawing_start_row,
        3,
        drawing_start_row + 2,
        4,
        "Текстовый рисунок\nс подписью",
    )
    drawing_text.Шрифт = font(connection, "Arial", 11, bold=True)

    drawing_rectangle = create_drawing(
        document,
        connection,
        "Прямоугольник",
        "РисунокПрямоугольник",
        drawing_start_row,
        5,
        drawing_start_row + 2,
        6,
        "Прямоугольник",
    )
    drawing_rectangle.ЦветФона = color(connection, (255, 242, 204))
    drawing_rectangle.Узор = connection.ТипУзораТабличногоДокумента.Узор7

    drawing_chart = create_drawing(
        document,
        connection,
        "Диаграмма",
        "РисунокДиаграмма",
        drawing_start_row + 3,
        3,
        drawing_start_row + 5,
        6,
        "Объект-рисунок\nДиаграмма",
    )
    drawing_chart.ЦветФона = color(connection, (226, 239, 218))
    drawing_chart.Узор = connection.ТипУзораТабличногоДокумента.Узор11

    drawing_note_row = drawing_start_row + 6
    drawing_note = document.Area(
        drawing_note_row, 1, drawing_note_row, total_columns
    )
    drawing_note.Объединить()
    set_area(
        drawing_note,
        text=(
            "Картинка загружена из PNG и встроена в MXL; остальные объекты "
            "созданы коллекцией Рисунки. Имена объектов сохранены в свойствах Имя."
        ),
        background=color(connection, (242, 242, 242)),
        area_font=font(connection, "Arial", 9),
    )
    document.Area(drawing_note_row, 1, drawing_note_row, total_columns).ВысотаСтроки = 32

    bits_title_row = drawing_note_row + 2
    bits_title = document.Area(bits_title_row, 1, bits_title_row, total_columns)
    bits_title.Объединить()
    set_area(
        bits_title,
        text="Биты 15-32: маска, HEX и знаковое значение Int32",
        background=color(connection, (217, 225, 242)),
        area_font=font(connection, "Arial", 11, bold=True),
    )

    header_row = bits_title_row + 1
    headers = ["Бит", "Маска", "HEX / signed Int32"] * 2
    for column, header in enumerate(headers, start=1):
        set_area(
            document.Area(header_row, column, header_row, column),
            text=header,
            background=color(connection, (31, 78, 121)),
            text_color=color(connection, (255, 255, 255)),
            area_font=font(connection, "Arial", 9, bold=True),
        )

    bit_start_row = header_row + 1
    for pair_index, first_bit in enumerate(range(15, 33, 2)):
        row = bit_start_row + pair_index
        for offset, bit_number in enumerate((first_bit, first_bit + 1)):
            column = 1 + offset * 3
            mask, hexadecimal, signed_int32 = bit_values(bit_number)
            values = (
                f"Бит {bit_number}",
                str(mask),
                f"{hexadecimal}\nsigned={signed_int32}",
            )
            for value_offset, value in enumerate(values):
                set_area(
                    document.Area(row, column + value_offset, row, column + value_offset),
                    text=value,
                    background=color(
                        connection,
                        (255, 255, 255) if pair_index % 2 == 0 else (242, 242, 242),
                    ),
                    area_font=font(connection, "Consolas", 9),
                )
        document.Area(row, 1, row, total_columns).ВысотаСтроки = 30

    bits_end_row = bit_start_row + 8
    summary_row = bits_end_row + 1
    summary = document.Area(summary_row, 1, summary_row, total_columns)
    summary.Объединить()
    all_bits_mask = (1 << 32) - (1 << 14)
    set_area(
        summary,
        text=(
            f"Все биты 15-32: unsigned={all_bits_mask}, "
            f"HEX=0x{all_bits_mask:08X}, signed Int32={all_bits_mask - (1 << 32)}"
        ),
        background=color(connection, (255, 242, 204)),
        area_font=font(connection, "Arial", 10, bold=True),
    )

    document.Write(str(output_path), "mxl")

    check_document = connection.NewObject("SpreadsheetDocument")
    check_document.Read(str(output_path))

    for index, (pattern_name, _) in enumerate(PATTERNS):
        row = pattern_start_row + index // pattern_columns
        column = 1 + index % pattern_columns
        area = check_document.Area(row, column, row, column)
        pattern_color = area.ЦветУзора
        if pattern_color is None:
            raise RuntimeError(f"Не сохранился цвет узора {pattern_name}")
        if area.Узор is None:
            raise RuntimeError(f"Не сохранился узор {pattern_name}")

    if int(check_document.Рисунки.Количество()) != 4:
        raise RuntimeError(
            f"Ожидалось 4 встроенных рисунка, получено "
            f"{check_document.Рисунки.Количество()}"
        )
    drawing_names = []
    for index in range(check_document.Рисунки.Количество()):
        drawing_names.append(check_document.Рисунки.Получить(index).Имя)
    expected_names = [
        "ВстроеннаяКартинка",
        "РисунокТекст",
        "РисунокПрямоугольник",
        "РисунокДиаграмма",
    ]
    if drawing_names != expected_names:
        raise RuntimeError(
            f"Имена рисунков не сохранились: ожидались {expected_names}, "
            f"получены {drawing_names}"
        )

    for bit_number in range(15, 33):
        mask, hexadecimal, signed_int32 = bit_values(bit_number)
        row = bit_start_row + (bit_number - 15) // 2
        column = 1 + ((bit_number - 15) % 2) * 3
        check_text = check_document.Area(row, column, row, column).Текст
        check_mask = check_document.Area(row, column + 1, row, column + 1).Текст
        check_values = check_document.Area(
            row, column + 2, row, column + 2
        ).Текст
        if check_text != f"Бит {bit_number}" or check_mask != str(mask):
            raise RuntimeError(f"Не сохранилась подпись бита {bit_number}")
        if hexadecimal not in check_values or str(signed_int32) not in check_values:
            raise RuntimeError(f"Не сохранились значения бита {bit_number}")

    return {
        "rows": int(check_document.ВысотаТаблицы),
        "columns": int(check_document.ШиринаТаблицы),
        "patterns": len(PATTERNS),
        "drawings": int(check_document.Рисунки.Количество()),
        "bits_from": 15,
        "bits_to": 32,
        "all_bits_mask": all_bits_mask,
        "all_bits_hex": f"0x{all_bits_mask:08X}",
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--image", type=Path, required=True)
    args = parser.parse_args()

    result = generate(
        args.base.resolve(), args.output.resolve(), args.image.resolve()
    )
    print(f"[OK] Создан файл: {args.output.resolve()}")
    print(
        "[OK] Проверка чтением 1С: "
        f"строк={result['rows']}, колонок={result['columns']}, "
        f"узоров={result['patterns']}, рисунков={result['drawings']}, "
        f"биты={result['bits_from']}-{result['bits_to']}, "
        f"общая_маска={result['all_bits_hex']}"
    )


if __name__ == "__main__":
    main()
