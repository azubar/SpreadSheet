"""Генерация MXL с границами по сторонам, толщинами и объединениями."""

from __future__ import annotations

import argparse
from pathlib import Path

import win32com.client

from generate_mxl_demo import color, font, set_area


LINE_TYPES = [
    ("Сплошная", "Сплошная"),
    ("Точечная", "Точечная"),
    ("Двойная", "Двойная"),
    ("РедкийПунктир", "Редкий пунктир"),
    ("ЧастыйПунктир", "Частый пунктир"),
    ("БольшойПунктир", "Большой пунктир"),
]
SIDES = [
    ("ГраницаСлева", "слева"),
    ("ГраницаСправа", "справа"),
    ("ГраницаСверху", "сверху"),
    ("ГраницаСнизу", "снизу"),
]
THICKNESSES = (1, 2, 3)


def make_line(connection, line_type: str, thickness: int):
    enum_value = getattr(
        connection.ТипЛинииЯчейкиТабличногоДокумента, line_type
    )
    return connection.NewObject("Line", enum_value, thickness)


def set_side_border(area, connection, side: str, line_type: str, thickness: int):
    setattr(area, side, make_line(connection, line_type, thickness))


def set_label(area, connection, text: str, background=(255, 255, 255), bold=False):
    set_area(
        area,
        text=text,
        background=color(connection, background),
        text_color=color(connection, (31, 31, 31)),
        area_font=font(connection, "Arial", 8 if not bold else 10, bold=bold),
        wrap=True,
    )


def apply_mixed_borders(area, connection, borders):
    for side, line_type, thickness in borders:
        set_side_border(area, connection, side, line_type, thickness)


def generate(base_path: Path, output_path: Path) -> dict[str, int]:
    output_path.parent.mkdir(parents=True, exist_ok=True)

    connector = win32com.client.Dispatch("V83.COMConnector")
    connection = connector.Connect(f'File="{base_path}";')
    document = connection.NewObject("SpreadsheetDocument")

    total_columns = 12
    for column in range(1, total_columns + 1):
        document.Area(1, column, 1, column).ШиринаКолонки = 18

    title = document.Area(1, 1, 1, total_columns)
    title.Объединить()
    set_area(
        title,
        text="Демонстрация границ ячеек и объединенных областей MXL",
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
            "В каждой ячейке ниже задана только одна сторона границы. "
            "Подпись содержит сторону, тип линии и толщину."
        ),
        background=color(connection, (221, 235, 247)),
        area_font=font(connection, "Arial", 10, italic=True),
    )
    document.Area(2, 1, 2, total_columns).ВысотаСтроки = 34

    row = 4
    border_cells = []
    unsupported_cells = []
    for line_type, line_caption in LINE_TYPES:
        block_title = document.Area(row, 1, row, total_columns)
        block_title.Объединить()
        set_label(
            block_title,
            connection,
            f"Тип линии: {line_caption} ({line_type}) — толщины 1, 2, 3",
            background=(217, 225, 242),
            bold=True,
        )
        row += 1

        for side_index, (side, side_caption) in enumerate(SIDES):
            for thickness_index, thickness in enumerate(THICKNESSES):
                column = side_index * 3 + thickness_index + 1
                area = document.Area(row, column, row, column)
                is_unsupported = line_type == "Двойная" and thickness > 1
                if is_unsupported:
                    text = (
                        f"{side_caption}\n{line_caption}\n"
                        f"толщина {thickness}\nНЕ ПОДДЕРЖИВАЕТСЯ API 1С"
                    )
                    set_label(area, connection, text, background=(255, 199, 206))
                    unsupported_cells.append((row, column, side, thickness))
                else:
                    text = f"{side_caption}\n{line_caption}\nтолщина {thickness}"
                    set_label(area, connection, text)
                    set_side_border(area, connection, side, line_type, thickness)
                    border_cells.append((row, column, side, line_type, thickness))
        document.Area(row, 1, row, total_columns).ВысотаСтроки = 54
        row += 2

    underline_title = document.Area(row, 1, row, total_columns)
    underline_title.Объединить()
    set_label(
        underline_title,
        connection,
        "Нижнее подчеркивание символами «____» и настоящая нижняя граница",
        background=(242, 242, 242),
        bold=True,
    )
    row += 1
    underline_examples_row = row
    underline_cells = [
        (1, "Текст + символы\n____", False),
        (4, "Только символы\n____", False),
        (7, "Настоящее подчеркивание\nтекст", True),
        (10, "Символы + нижняя граница\n____", False),
    ]
    for column, text, real_underline in underline_cells:
        area = document.Area(row, column, row, column + 2)
        area.Объединить()
        set_label(area, connection, text, background=(255, 255, 255))
        if real_underline:
            area.Шрифт = font(connection, "Arial", 10, underline=True)
        else:
            set_side_border(area, connection, "ГраницаСнизу", "Сплошная", 1)
    document.Area(row, 1, row, total_columns).ВысотаСтроки = 46
    row += 2

    combination_title = document.Area(row, 1, row, total_columns)
    combination_title.Объединить()
    set_label(
        combination_title,
        connection,
        "Объединенные ячейки с комбинациями сторон, типов линий и толщин",
        background=(217, 225, 242),
        bold=True,
    )
    row += 1
    combination_rows = []

    combinations = [
        {
            "area": (row, 1, row, 4),
            "text": (
                "Объединение 1x4\n"
                "слева точечная 1; справа частый пунктир 2; "
                "сверху двойная 1; снизу сплошная 3"
            ),
            "borders": [
                ("ГраницаСлева", "Точечная", 1),
                ("ГраницаСправа", "ЧастыйПунктир", 2),
                ("ГраницаСверху", "Двойная", 1),
                ("ГраницаСнизу", "Сплошная", 3),
            ],
            "height": 64,
        },
        {
            "area": (row, 5, row, 8),
            "text": (
                "Объединение 1x4\n"
                "слева редкий пунктир 3; справа большой пунктир 1; "
                "сверху сплошная 2; снизу точечная 1"
            ),
            "borders": [
                ("ГраницаСлева", "РедкийПунктир", 3),
                ("ГраницаСправа", "БольшойПунктир", 1),
                ("ГраницаСверху", "Сплошная", 2),
                ("ГраницаСнизу", "Точечная", 1),
            ],
            "height": 64,
        },
        {
            "area": (row, 9, row + 1, 12),
            "text": (
                "Объединение 2x4\n"
                "все стороны: большой пунктир 3"
            ),
            "borders": [
                ("ГраницаСлева", "БольшойПунктир", 3),
                ("ГраницаСправа", "БольшойПунктир", 3),
                ("ГраницаСверху", "БольшойПунктир", 3),
                ("ГраницаСнизу", "БольшойПунктир", 3),
            ],
            "height": 64,
        },
        {
            "area": (row + 2, 1, row + 3, 6),
            "text": (
                "Объединение 2x6\n"
                "слева/справа редкий пунктир 2; "
                "сверху/снизу сплошная 3; текст ____"
            ),
            "borders": [
                ("ГраницаСлева", "РедкийПунктир", 2),
                ("ГраницаСправа", "РедкийПунктир", 2),
                ("ГраницаСверху", "Сплошная", 3),
                ("ГраницаСнизу", "Сплошная", 3),
            ],
            "height": 62,
        },
        {
            "area": (row + 2, 7, row + 3, 12),
            "text": (
                "Объединение 2x6\n"
                "слева/справа двойная 1; "
                "сверху/снизу точечная 2"
            ),
            "borders": [
                ("ГраницаСлева", "Двойная", 1),
                ("ГраницаСправа", "Двойная", 1),
                ("ГраницаСверху", "Точечная", 2),
                ("ГраницаСнизу", "Точечная", 2),
            ],
            "height": 62,
        },
    ]

    for combination in combinations:
        begin_row, begin_column, end_row, end_column = combination["area"]
        area = document.Area(begin_row, begin_column, end_row, end_column)
        area.Объединить()
        set_label(area, connection, combination["text"], background=(255, 255, 255))
        apply_mixed_borders(area, connection, combination["borders"])
        for current_row in range(begin_row, end_row + 1):
            document.Area(current_row, begin_column, current_row, end_column).ВысотаСтроки = combination["height"]
        combination_rows.append((combination["area"], combination["text"]))

    row = row + 5
    footer = document.Area(row, 1, row, total_columns)
    footer.Объединить()
    set_label(
        footer,
        connection,
        (
            f"Проверено: валидных односторонних границ {len(border_cells)}, "
            f"явно отмеченных ограничений API {len(unsupported_cells)}, "
            f"объединенных комбинаций {len(combination_rows)}"
        ),
        background=(31, 78, 121),
    )
    footer.ЦветТекста = color(connection, (255, 255, 255))

    document.Write(str(output_path), "mxl")

    check_document = connection.NewObject("SpreadsheetDocument")
    check_document.Read(str(output_path))

    for check_row, check_column, side, line_type, thickness in border_cells:
        area = check_document.Area(check_row, check_column, check_row, check_column)
        actual_thickness = int(getattr(area, side).Толщина)
        if actual_thickness != thickness:
            raise RuntimeError(
                f"Не сохранилась граница {line_type}/{side}/{thickness}: "
                f"получена толщина {actual_thickness}"
            )
        expected_text = dict(SIDES)[side]
        if expected_text not in area.Текст or str(thickness) not in area.Текст:
            raise RuntimeError(
                f"Не сохранилась подпись ячейки {check_row}:{check_column}"
            )

    for check_row, check_column, side, thickness in unsupported_cells:
        area = check_document.Area(check_row, check_column, check_row, check_column)
        if "НЕ ПОДДЕРЖИВАЕТСЯ API 1С" not in area.Текст:
            raise RuntimeError(
                f"Не сохранилась подпись ограничения {check_row}:{check_column}"
            )
        if int(getattr(area, side).Толщина) != 0:
            raise RuntimeError(
                f"Недопустимая двойная граница неожиданно записалась: "
                f"{check_row}:{check_column}"
            )

    for check_column, text, real_underline in underline_cells:
        underline_area = check_document.Area(
            underline_examples_row, check_column, underline_examples_row, check_column
        )
        actual_text = underline_area.Текст
        if "____" not in actual_text and not real_underline:
            raise RuntimeError(f"Не сохранилось нижнее подчеркивание {check_column}")
        if real_underline and not bool(underline_area.Шрифт.Underline):
            raise RuntimeError("Не сохранился флаг настоящего подчеркивания")

    for area_coordinates, expected_text in combination_rows:
        begin_row, begin_column, _, _ = area_coordinates
        actual_text = check_document.Area(
            begin_row, begin_column, begin_row, begin_column
        ).Текст
        if actual_text != expected_text:
            raise RuntimeError(
                f"Не сохранился текст объединенной области {area_coordinates}"
            )

    return {
        "rows": int(check_document.ВысотаТаблицы),
        "columns": int(check_document.ШиринаТаблицы),
        "valid_borders": len(border_cells),
        "unsupported_borders": len(unsupported_cells),
        "merged_combinations": len(combination_rows),
        "underline_examples": len(underline_cells),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    result = generate(args.base.resolve(), args.output.resolve())
    print(f"[OK] Создан файл: {args.output.resolve()}")
    print(
        "[OK] Проверка чтением 1С: "
        f"строк={result['rows']}, колонок={result['columns']}, "
        f"валидных_границ={result['valid_borders']}, "
        f"ограничений_API={result['unsupported_borders']}, "
        f"объединенных_комбинаций={result['merged_combinations']}, "
        f"подчеркиваний={result['underline_examples']}"
    )


if __name__ == "__main__":
    main()
