"""Генерация демонстрационного бинарного MXL через COM-объект 1С."""

from __future__ import annotations

import argparse
from pathlib import Path

import win32com.client


PALETTE = [
    ("Красный", (255, 0, 0)),
    ("Зеленый", (0, 128, 0)),
    ("Синий", (0, 0, 255)),
    ("Желтый", (255, 255, 0)),
    ("Бирюзовый", (0, 255, 255)),
    ("Пурпурный", (255, 0, 255)),
    ("Черный", (0, 0, 0)),
    ("Белый", (255, 255, 255)),
    ("Темно-красный", (128, 0, 0)),
    ("Темно-зеленый", (0, 100, 0)),
    ("Темно-синий", (0, 0, 139)),
    ("Оливковый", (128, 128, 0)),
    ("Темно-бирюзовый", (0, 128, 128)),
    ("Темно-пурпурный", (128, 0, 128)),
    ("Серый", (128, 128, 128)),
    ("Серебряный", (192, 192, 192)),
    ("Оранжевый", (255, 165, 0)),
    ("Лаймовый", (0, 255, 0)),
    ("Розовый", (255, 192, 203)),
    ("Коричневый", (165, 42, 42)),
    ("Фиолетовый", (238, 130, 238)),
    ("Золотой", (255, 215, 0)),
    ("Индиго", (75, 0, 130)),
    ("Бежевый", (245, 245, 220)),
    ("Лососевый", (250, 128, 114)),
    ("Хаки", (240, 230, 140)),
    ("Лазурный", (240, 255, 255)),
    ("Аквамарин", (127, 255, 212)),
    ("Мятный", (152, 255, 152)),
    ("Небесный", (135, 206, 235)),
    ("Сапфировый", (8, 37, 103)),
    ("Бирюзовый светлый", (64, 224, 208)),
    ("Красный светлый", (255, 99, 71)),
    ("Алый", (220, 20, 60)),
    ("Малиновый", (199, 21, 133)),
    ("Персиковый", (255, 218, 185)),
    ("Абрикосовый", (251, 206, 177)),
    ("Лимонный", (255, 250, 205)),
    ("Салатовый", (124, 252, 0)),
    ("Травяной", (34, 139, 34)),
    ("Изумрудный", (80, 200, 120)),
    ("Оливковый светлый", (189, 183, 107)),
    ("Морской волны", (46, 139, 87)),
    ("Кобальтовый", (0, 71, 171)),
    ("Голубой", (30, 144, 255)),
    ("Синий светлый", (173, 216, 230)),
    ("Стальной", (70, 130, 180)),
    ("Пыльно-синий", (119, 158, 203)),
    ("Лавандовый", (230, 230, 250)),
    ("Сливовый", (221, 160, 221)),
    ("Сиреневый", (200, 162, 200)),
    ("Бордовый", (128, 0, 32)),
    ("Вишневый", (222, 49, 99)),
    ("Шоколадный", (210, 105, 30)),
    ("Медный", (184, 115, 51)),
    ("Песочный", (244, 164, 96)),
    ("Кремовый", (255, 253, 208)),
    ("Слоновая кость", (255, 255, 240)),
    ("Графитовый", (54, 69, 79)),
    ("Антрацитовый", (41, 41, 41)),
    ("Серый светлый", (211, 211, 211)),
    ("Серый темный", (80, 80, 80)),
    ("Дымчатый", (176, 196, 222)),
    ("Бирюзовый темный", (0, 206, 209)),
    ("Рубиновый", (224, 17, 95)),
]


def color(connection, rgb: tuple[int, int, int]):
    return connection.NewObject("Color", *rgb)


def font(connection, face: str, size: int, bold: bool = False,
         italic: bool = False, underline: bool = False,
         strikeout: bool = False):
    return connection.NewObject(
        "Font", face, size, bold, italic, underline, strikeout
    )


def set_area(area, *, text=None, background=None, text_color=None,
             area_font=None, number_format=None, wrap=None,
             orientation=None):
    if text is not None:
        area.Текст = text
    if background is not None:
        area.ЦветФона = background
    if text_color is not None:
        area.ЦветТекста = text_color
    if area_font is not None:
        area.Шрифт = area_font
    if number_format is not None:
        area.Формат = number_format
    if orientation is not None:
        area.ОриентацияТекста = orientation
    # АвтоПеренос доступен в BSL, но не экспортируется как writable-свойство
    # COM-оберткой 1С 8.3.23; многострочный текст при этом сохраняется.


def set_border(area, connection, line_kind, thickness=1):
    """Устанавливает одну и ту же границу по четырем сторонам области."""
    line = connection.NewObject(
        "Line",
        getattr(connection.ТипЛинииЯчейкиТабличногоДокумента, line_kind),
        thickness,
    )
    for side in ("ГраницаСлева", "ГраницаСправа", "ГраницаСверху", "ГраницаСнизу"):
        setattr(area, side, line)


def text_color_for(rgb: tuple[int, int, int]) -> tuple[int, int, int]:
    luminance = 0.299 * rgb[0] + 0.587 * rgb[1] + 0.114 * rgb[2]
    return (0, 0, 0) if luminance >= 150 else (255, 255, 255)


def generate(base_path: Path, output_path: Path) -> dict[str, int]:
    output_path.parent.mkdir(parents=True, exist_ok=True)

    connector = win32com.client.Dispatch("V83.COMConnector")
    connection = connector.Connect(f'File="{base_path}";')
    document = connection.NewObject("SpreadsheetDocument")

    total_columns = 8
    for column in range(1, total_columns + 1):
        document.Area(1, column, 1, column).ШиринаКолонки = 18

    # Заголовок и объединение ячеек.
    title = document.Area(1, 1, 1, total_columns)
    title.Объединить()
    set_area(
        title,
        text="Демонстрация возможностей табличного документа 1С",
        background=color(connection, (31, 78, 121)),
        text_color=color(connection, (255, 255, 255)),
        area_font=font(connection, "Arial", 16, bold=True),
    )
    document.Area(1, 1, 1, total_columns).ВысотаСтроки = 30

    subtitle = document.Area(2, 1, 2, total_columns)
    subtitle.Объединить()
    set_area(
        subtitle,
        text=(
            "64 базовых цвета заливки, цвет текста, шрифты, объединения, "
            "ширины колонок, высоты строк и числовые форматы"
        ),
        background=color(connection, (221, 235, 247)),
        text_color=color(connection, (31, 31, 31)),
        area_font=font(connection, "Arial", 10, italic=True),
        wrap=True,
    )
    document.Area(2, 1, 2, total_columns).ВысотаСтроки = 34

    label = document.Area(4, 1, 4, total_columns)
    label.Объединить()
    set_area(
        label,
        text="Палитра: каждая ячейка содержит название и RGB-код цвета",
        background=color(connection, (242, 242, 242)),
        area_font=font(connection, "Arial", 11, bold=True),
    )

    palette_start_row = 5
    for index, (name, rgb) in enumerate(PALETTE):
        row = palette_start_row + index // total_columns
        column = 1 + index % total_columns
        area = document.Area(row, column, row, column)
        set_area(
            area,
            text=f"{index + 1:02d}\n{name}\n#{rgb[0]:02X}{rgb[1]:02X}{rgb[2]:02X}",
            background=color(connection, rgb),
            text_color=color(connection, text_color_for(rgb)),
            area_font=font(connection, "Arial", 8),
            wrap=True,
        )
    palette_end_row = palette_start_row + (
        len(PALETTE) + total_columns - 1
    ) // total_columns - 1
    for row in range(palette_start_row, palette_end_row + 1):
        document.Area(row, 1, row, total_columns).ВысотаСтроки = 36

    text_palette_label_row = palette_end_row + 2
    text_palette_label = document.Area(
        text_palette_label_row, 1, text_palette_label_row, total_columns
    )
    text_palette_label.Объединить()
    set_area(
        text_palette_label,
        text="Палитра цвета текста: 65 ячеек с разными RGB-значениями",
        background=color(connection, (242, 242, 242)),
        area_font=font(connection, "Arial", 11, bold=True),
    )

    text_palette_start_row = text_palette_label_row + 1
    for index, (name, rgb) in enumerate(PALETTE):
        row = text_palette_start_row + index // total_columns
        column = 1 + index % total_columns
        area = document.Area(row, column, row, column)
        set_area(
            area,
            text=f"{index + 1:02d}\n{name}\n#{rgb[0]:02X}{rgb[1]:02X}{rgb[2]:02X}",
            background=color(connection, (255, 255, 255)),
            text_color=color(connection, rgb),
            area_font=font(connection, "Arial", 8, bold=True),
            wrap=True,
        )
    text_palette_end_row = text_palette_start_row + (
        len(PALETTE) + total_columns - 1
    ) // total_columns - 1
    for row in range(text_palette_start_row, text_palette_end_row + 1):
        document.Area(row, 1, row, total_columns).ВысотаСтроки = 36

    examples_row = text_palette_end_row + 2
    examples = document.Area(examples_row, 1, examples_row, total_columns)
    examples.Объединить()
    set_area(
        examples,
        text="Примеры шрифтов и оформления",
        background=color(connection, (217, 225, 242)),
        area_font=font(connection, "Arial", 11, bold=True),
    )

    row = examples_row + 1
    set_area(
        document.Area(row, 1, row, 2),
        text="Обычный текст",
        area_font=font(connection, "Arial", 10),
        background=color(connection, (255, 255, 255)),
    )
    set_area(
        document.Area(row, 3, row, 4),
        text="Жирный текст",
        area_font=font(connection, "Arial", 10, bold=True),
        background=color(connection, (255, 242, 204)),
    )
    set_area(
        document.Area(row, 5, row, 6),
        text="Курсивный текст",
        area_font=font(connection, "Arial", 10, italic=True),
        background=color(connection, (226, 239, 218)),
    )
    set_area(
        document.Area(row, 7, row, 8),
        text="Подчеркнутый текст",
        area_font=font(connection, "Arial", 10, underline=True),
        background=color(connection, (221, 235, 247)),
    )
    document.Area(row, 1, row, total_columns).ВысотаСтроки = 24

    row += 1
    set_area(
        document.Area(row, 1, row, 2),
        text="Зачеркнутый текст",
        area_font=font(connection, "Arial", 10, strikeout=True),
        background=color(connection, (252, 228, 214)),
    )
    set_area(
        document.Area(row, 3, row, 4),
        text="Жирный + курсив +\nподчеркнутый + зачеркнутый",
        area_font=font(
            connection, "Arial", 10, bold=True, italic=True,
            underline=True, strikeout=True
        ),
        background=color(connection, (234, 209, 220)),
        wrap=True,
    )
    set_area(
        document.Area(row, 5, row, 6),
        text="Косой текст",
        area_font=font(connection, "Arial", 10, italic=True),
        background=color(connection, (226, 239, 218)),
    )
    set_area(
        document.Area(row, 7, row, 8),
        text="Вертикальный\nтекст",
        area_font=font(connection, "Arial", 10, bold=True),
        background=color(connection, (255, 242, 204)),
        orientation=90,
        wrap=True,
    )
    document.Area(row, 1, row, total_columns).ВысотаСтроки = 90

    row += 2
    borders_title = document.Area(row, 1, row, total_columns)
    borders_title.Объединить()
    set_area(
        borders_title,
        text="Границы ячеек: сплошная 1/2/3, двойная, редкий пунктир, точечная, частый пунктир",
        background=color(connection, (217, 225, 242)),
        area_font=font(connection, "Arial", 10, bold=True),
    )
    row += 1
    border_examples = [
        ("Сплошная\nтолщина 1", "Сплошная", 1),
        ("Сплошная\nтолщина 2", "Сплошная", 2),
        ("Сплошная\nтолщина 3", "Сплошная", 3),
        ("Двойная", "Двойная", 1),
        ("Редкий\nпунктир", "РедкийПунктир", 1),
        ("Точечная", "Точечная", 1),
        ("Частый\nпунктир", "ЧастыйПунктир", 1),
        ("Двойная\nтолщина 1", "Двойная", 1),
    ]
    border_row = row
    for index, (caption, line_kind, thickness) in enumerate(border_examples):
        cell = document.Area(row, index + 1, row, index + 1)
        set_area(
            cell,
            text=caption,
            background=color(connection, (255, 255, 255)),
            area_font=font(connection, "Arial", 9),
            wrap=True,
        )
        set_border(cell, connection, line_kind, thickness)
    document.Area(row, 1, row, total_columns).ВысотаСтроки = 45

    row += 2
    formats = document.Area(row, 1, row, total_columns)
    formats.Объединить()
    set_area(
        formats,
        text="Форматирование значений: число, сумма и дата",
        background=color(connection, (242, 242, 242)),
        area_font=font(connection, "Arial", 10, bold=True),
    )
    row += 1
    set_area(
        document.Area(row, 1, row, 2),
        text="1234567,89",
        number_format="ЧДЦ=2; ЧРД=,",
        background=color(connection, (255, 255, 255)),
        area_font=font(connection, "Consolas", 10),
    )
    set_area(
        document.Area(row, 3, row, 4),
        text="98765,43 ₽",
        number_format="ЧДЦ=2; ЧРД=,",
        background=color(connection, (255, 255, 255)),
        area_font=font(connection, "Consolas", 10),
    )
    set_area(
        document.Area(row, 5, row, 6),
        text="19.07.2026",
        number_format="ДФ=dd.MM.yyyy",
        background=color(connection, (255, 255, 255)),
        area_font=font(connection, "Consolas", 10),
    )
    set_area(
        document.Area(row, 7, row, 8),
        text="Многострочный текст с переносом",
        background=color(connection, (255, 255, 255)),
        area_font=font(connection, "Arial", 9),
        wrap=True,
    )
    document.Area(row, 1, row, total_columns).ВысотаСтроки = 30

    row += 2
    footer = document.Area(row, 1, row, total_columns)
    footer.Объединить()
    set_area(
        footer,
        text="Файл создан платформой 1С:Предприятие через COM; формат MXL",
        background=color(connection, (31, 78, 121)),
        text_color=color(connection, (255, 255, 255)),
        area_font=font(connection, "Arial", 9, italic=True),
    )

    document.Write(str(output_path), "mxl")

    # Читаем записанный файл обратно, чтобы проверить, что он распознается 1С.
    check_document = connection.NewObject("SpreadsheetDocument")
    check_document.Read(str(output_path))
    check_palette = check_document.Area(palette_start_row, 1, palette_start_row, 1)
    check_color = check_palette.ЦветФона
    for index, (_, expected_rgb) in enumerate(PALETTE):
        row = palette_start_row + index // total_columns
        column = 1 + index % total_columns
        actual_color = check_document.Area(row, column, row, column).ЦветФона
        actual_rgb = (
            int(actual_color.Красный),
            int(actual_color.Зеленый),
            int(actual_color.Синий),
        )
        if actual_rgb != expected_rgb:
            raise RuntimeError(
                f"Цвет ячейки {index + 1} изменился после записи: "
                f"ожидался {expected_rgb}, получен {actual_rgb}"
            )


    for index, (_, expected_rgb) in enumerate(PALETTE):
        row = text_palette_start_row + index // total_columns
        column = 1 + index % total_columns
        text_color = check_document.Area(row, column, row, column).ЦветТекста
        actual_text_rgb = (
            int(text_color.Красный),
            int(text_color.Зеленый),
            int(text_color.Синий),
        )
        if actual_text_rgb != expected_rgb:
            raise RuntimeError(
                f"Цвет текста ячейки {index + 1} изменился после записи: "
                f"ожидался {expected_rgb}, получен {actual_text_rgb}"
            )

    # Проверяем флаги шрифта и вертикальную ориентацию после сериализации.
    normal_font = check_document.Area(examples_row + 1, 1, examples_row + 1, 1).Шрифт
    bold_font = check_document.Area(examples_row + 1, 3, examples_row + 1, 3).Шрифт
    italic_font = check_document.Area(examples_row + 1, 5, examples_row + 1, 5).Шрифт
    underline_font = check_document.Area(
        examples_row + 1, 7, examples_row + 1, 7
    ).Шрифт
    strikeout_font = check_document.Area(examples_row + 2, 1, examples_row + 2, 1).Шрифт
    all_styles_font = check_document.Area(
        examples_row + 2, 3, examples_row + 2, 3
    ).Шрифт
    if bool(normal_font.Жирный) or bool(normal_font.Наклонный):
        raise RuntimeError("Обычный шрифт после записи имеет неожиданные флаги")
    if not bool(bold_font.Жирный):
        raise RuntimeError("Не сохранился флаг жирного шрифта")
    if not bool(italic_font.Наклонный):
        raise RuntimeError("Не сохранился флаг курсива")
    if not bool(underline_font.Underline):
        raise RuntimeError("Не сохранился флаг подчеркивания")
    if not bool(strikeout_font.Зачеркивание):
        raise RuntimeError("Не сохранился флаг зачеркивания")
    if not all(
        bool(getattr(all_styles_font, prop))
        for prop in ("Жирный", "Наклонный", "Underline", "Зачеркивание")
    ):
        raise RuntimeError("Не сохранилась комбинация начертаний шрифта")

    vertical_area = check_document.Area(
        examples_row + 2, 7, examples_row + 2, 7
    )
    if int(vertical_area.ОриентацияТекста) != 90:
        raise RuntimeError("Не сохранилась вертикальная ориентация текста")

    border_thicknesses = []
    for index, (_, _, thickness) in enumerate(border_examples):
        actual_thickness = int(
            check_document.Area(border_row, index + 1, border_row, index + 1)
            .ГраницаСлева.Толщина
        )
        if actual_thickness != thickness:
            raise RuntimeError(
                f"Не сохранилась толщина границы {index + 1}: "
                f"ожидалась {thickness}, получена {actual_thickness}"
            )
        border_thicknesses.append(actual_thickness)

    return {
        "rows": int(check_document.ВысотаТаблицы),
        "columns": int(check_document.ШиринаТаблицы),
        "palette_cells": len(PALETTE),
        "text_color_cells": len(PALETTE),
        "unique_colors": len({rgb for _, rgb in PALETTE}),
        "font_examples": 8,
        "border_examples": len(border_examples),
        "orientation_degrees": 90,
        "max_border_thickness": max(border_thicknesses),
        "first_color_red": int(check_color.Красный),
        "first_color_green": int(check_color.Зеленый),
        "first_color_blue": int(check_color.Синий),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", type=Path, required=True, help="Путь к файловой базе 1С")
    parser.add_argument("--output", type=Path, required=True, help="Путь к MXL-файлу")
    args = parser.parse_args()

    result = generate(args.base.resolve(), args.output.resolve())
    print(f"[OK] Создан файл: {args.output.resolve()}")
    print(
        "[OK] Проверка чтением 1С: "
        f"строк={result['rows']}, колонок={result['columns']}, "
        f"заливок={result['palette_cells']}, "
        f"цветов_текста={result['text_color_cells']}, "
        f"уникальных={result['unique_colors']}, "
        f"шрифтов={result['font_examples']}, "
        f"границ={result['border_examples']}, "
        f"макс_толщина_границы={result['max_border_thickness']}, "
        f"первый RGB=({result['first_color_red']}, "
        f"{result['first_color_green']}, {result['first_color_blue']})"
    )


if __name__ == "__main__":
    main()
