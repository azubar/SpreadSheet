"""Генерация MXL с демонстрацией вариантов РазмещениеТекста."""

from __future__ import annotations

import argparse
from pathlib import Path

import win32com.client

from generate_mxl_demo import color, font, set_area, set_border


PLACEMENTS = (
    (
        "Авто",
        "Auto",
        "Автоматическое размещение: поведение зависит от свободного места "
        "и соседних ячеек.",
        (226, 239, 218),
    ),
    (
        "Переносить",
        "Wrap",
        "Текст, который не помещается по ширине, переносится на следующие "
        "строки этой ячейки.",
        (221, 235, 247),
    ),
    (
        "Забивать",
        "Block",
        "Текст повторяется, заполняя доступную ширину области.",
        (255, 242, 204),
    ),
    (
        "Обрезать",
        "Cut",
        "Часть текста, не помещающаяся в область, обрезается.",
        (252, 228, 214),
    ),
)

PHRASE = (
    "Длинная фраза с пробелами: табличный документ показывает, "
    "как именно текст размещается внутри узкой ячейки."
)
LONG_WORD = "Сверхдлинноесловобезпробеловдляпроверкипереполненияячейки"


def placement_enum(connection, enum_name: str):
    return getattr(
        connection.ТипРазмещенияТекстаТабличногоДокумента,
        enum_name,
    )


def configure_area(area, connection, text, mode_name, enum_name, background):
    set_area(
        area,
        text=f"{mode_name}\n{text}",
        background=color(connection, background),
        text_color=color(connection, (31, 31, 31)),
        area_font=font(connection, "Arial", 9, bold=True),
        wrap=True,
    )
    area.РазмещениеТекста = placement_enum(connection, enum_name)
    set_border(area, connection, "Сплошная", 1)


def merged_header(document, connection, row, text, columns=8):
    area = document.Area(row, 1, row, columns)
    area.Объединить()
    set_area(
        area,
        text=text,
        background=color(connection, (31, 78, 121)),
        text_color=color(connection, (255, 255, 255)),
        area_font=font(connection, "Arial", 10, bold=True),
        wrap=True,
    )
    document.Area(row, 1, row, columns).ВысотаСтроки = 28


def mode_header(document, connection, row, begin_column, mode_name, caption):
    area = document.Area(row, begin_column, row, begin_column)
    set_area(
        area,
        text=f"{mode_name}\n{caption}",
        background=color(connection, (217, 225, 242)),
        area_font=font(connection, "Arial", 8, bold=True),
        wrap=True,
    )
    set_border(area, connection, "Сплошная", 1)


def add_placement_area(
    document,
    connection,
    coordinates,
    mode_index,
    text,
    placement_checks,
):
    begin_row, begin_column, end_row, end_column = coordinates
    mode_name, enum_name, _, background = PLACEMENTS[mode_index]
    area = document.Area(begin_row, begin_column, end_row, end_column)
    if end_row != begin_row or end_column != begin_column:
        area.Объединить()
    configure_area(area, connection, text, mode_name, enum_name, background)
    placement_checks.append((coordinates, enum_name, text))
    return area


def generate(base_path: Path, output_path: Path) -> dict[str, int]:
    output_path.parent.mkdir(parents=True, exist_ok=True)

    connector = win32com.client.Dispatch("V83.COMConnector")
    connection = connector.Connect(f'File="{base_path}";')
    document = connection.NewObject("SpreadsheetDocument")
    total_columns = 8
    placement_checks = []

    for column in range(1, total_columns + 1):
        document.Area(1, column, 1, column).ШиринаКолонки = 16

    title = document.Area(1, 1, 1, total_columns)
    title.Объединить()
    set_area(
        title,
        text="Демонстрация РазмещениеТекста в ячейках MXL",
        background=color(connection, (31, 78, 121)),
        text_color=color(connection, (255, 255, 255)),
        area_font=font(connection, "Arial", 15, bold=True),
    )
    document.Area(1, 1, 1, total_columns).ВысотаСтроки = 32

    subtitle = document.Area(2, 1, 2, total_columns)
    subtitle.Объединить()
    set_area(
        subtitle,
        text=(
            "Каждый пример подписан режимом. Одна и та же длинная фраза и "
            "длинное слово показаны в обычных и объединенных областях."
        ),
        background=color(connection, (221, 235, 247)),
        area_font=font(connection, "Arial", 9, italic=True),
        wrap=True,
    )
    document.Area(2, 1, 2, total_columns).ВысотаСтроки = 34

    merged_header(document, connection, 4, "Назначение вариантов РазмещениеТекста")
    for row, (mode_name, enum_name, description, background) in enumerate(
        PLACEMENTS, start=5
    ):
        label = document.Area(row, 1, row, 2)
        label.Объединить()
        set_area(
            label,
            text=f"{mode_name}\n{enum_name}",
            background=color(connection, background),
            area_font=font(connection, "Arial", 9, bold=True),
            wrap=True,
        )
        set_border(label, connection, "Сплошная", 1)
        explanation = document.Area(row, 3, row, total_columns)
        explanation.Объединить()
        set_area(
            explanation,
            text=description,
            background=color(connection, (255, 255, 255)),
            area_font=font(connection, "Arial", 9),
            wrap=True,
        )
        set_border(explanation, connection, "Сплошная", 1)
        document.Area(row, 1, row, total_columns).ВысотаСтроки = 38

    merged_header(document, connection, 10, "1. Обычные одиночные ячейки: фраза и длинное слово")
    for index, (mode_name, _, _, _) in enumerate(PLACEMENTS):
        mode_header(document, connection, 11, index + 1, mode_name, "фраза")
        mode_header(document, connection, 11, index + 5, mode_name, "слово")
    for index in range(4):
        add_placement_area(
            document,
            connection,
            (12, index + 1, 12, index + 1),
            index,
            PHRASE,
            placement_checks,
        )
        add_placement_area(
            document,
            connection,
            (12, index + 5, 12, index + 5),
            index,
            LONG_WORD,
            placement_checks,
        )
    document.Area(12, 1, 12, total_columns).ВысотаСтроки = 78

    note = document.Area(14, 1, 14, total_columns)
    note.Объединить()
    set_area(
        note,
        text=(
            "Особый случай Авто: слева длинный текст, справа пустая ячейка. "
            "Пустое соседнее пространство позволяет увидеть автоматическое "
            "размещение отдельно от заполненной соседней ячейки."
        ),
        background=color(connection, (242, 242, 242)),
        area_font=font(connection, "Arial", 9, italic=True),
        wrap=True,
    )
    set_border(note, connection, "Сплошная", 1)
    document.Area(14, 1, 14, total_columns).ВысотаСтроки = 38
    auto_area = document.Area(15, 1, 15, 1)
    configure_area(
        auto_area,
        connection,
        "Авто: текст может занять свободное место справа",
        "Авто",
        "Auto",
        (226, 239, 218),
    )
    placement_checks.append(((15, 1, 15, 1), "Auto", "свободное место"))
    empty_neighbor = document.Area(15, 2, 15, 2)
    set_area(
        empty_neighbor,
        text="Пустая соседняя ячейка",
        background=color(connection, (255, 255, 255)),
        area_font=font(connection, "Arial", 8),
        wrap=True,
    )
    set_border(empty_neighbor, connection, "Сплошная", 1)
    document.Area(15, 1, 15, total_columns).ВысотаСтроки = 48

    merged_header(document, connection, 17, "2. Горизонтально объединенные области 1x2")
    for index, (mode_name, _, _, _) in enumerate(PLACEMENTS):
        begin_column = index * 2 + 1
        area = document.Area(18, begin_column, 18, begin_column + 1)
        area.Объединить()
        set_area(
            area,
            text=f"{mode_name}\nобласть 1x2",
            background=color(connection, (217, 225, 242)),
            area_font=font(connection, "Arial", 9, bold=True),
            wrap=True,
        )
        set_border(area, connection, "Сплошная", 1)
        document.Area(18, begin_column, 18, begin_column + 1).ВысотаСтроки = 42
        add_placement_area(
            document,
            connection,
            (19, begin_column, 19, begin_column + 1),
            index,
            PHRASE,
            placement_checks,
        )
    document.Area(19, 1, 19, total_columns).ВысотаСтроки = 76

    merged_header(document, connection, 21, "3. Вертикально и горизонтально объединенные области 2x2")
    for index in range(4):
        begin_column = index * 2 + 1
        add_placement_area(
            document,
            connection,
            (22, begin_column, 23, begin_column + 1),
            index,
            f"{LONG_WORD} и еще одна длинная фраза внутри области 2x2",
            placement_checks,
        )
    document.Area(22, 1, 23, total_columns).ВысотаСтроки = 34

    merged_header(document, connection, 25, "4. Объединения разного размера")
    add_placement_area(
        document,
        connection,
        (26, 1, 26, 4),
        1,
        "Переносить: область 1x4 с длинной фразой, которая переносится по словам.",
        placement_checks,
    )
    add_placement_area(
        document,
        connection,
        (26, 5, 26, 8),
        3,
        "Обрезать: область 1x4 с текстом, выходящим за правую границу.",
        placement_checks,
    )
    document.Area(26, 1, 26, total_columns).ВысотаСтроки = 60
    add_placement_area(
        document,
        connection,
        (27, 1, 29, 4),
        0,
        "Авто: большая область 3x4 показывает автоматическое размещение длинного текста.",
        placement_checks,
    )
    add_placement_area(
        document,
        connection,
        (27, 5, 29, 8),
        2,
        "Забивать: большая область 3x4 показывает заполнение повторением текста.",
        placement_checks,
    )
    document.Area(27, 1, 29, total_columns).ВысотаСтроки = 34

    footer = document.Area(31, 1, 31, total_columns)
    footer.Объединить()
    set_area(
        footer,
        text=(
            f"Проверено режимов: {len(PLACEMENTS)}; примеров областей: "
            f"{len(placement_checks)}. Все режимы записаны в MXL через COM 1С."
        ),
        background=color(connection, (31, 78, 121)),
        text_color=color(connection, (255, 255, 255)),
        area_font=font(connection, "Arial", 9, italic=True),
        wrap=True,
    )
    document.Area(31, 1, 31, total_columns).ВысотаСтроки = 30
    document.ОбластьПечати = document.Область(1, 1, 31, total_columns)

    document.Write(str(output_path), "mxl")

    check_document = connection.NewObject("SpreadsheetDocument")
    check_document.Read(str(output_path))
    enum_checks = connection.ТипРазмещенияТекстаТабличногоДокумента
    for coordinates, enum_name, expected_text in placement_checks:
        begin_row, begin_column, end_row, end_column = coordinates
        area = check_document.Area(begin_row, begin_column, end_row, end_column)
        if area.РазмещениеТекста != getattr(enum_checks, enum_name):
            raise RuntimeError(
                f"Не сохранился режим {enum_name} в области {coordinates}"
            )
        if expected_text not in str(area.Текст):
            raise RuntimeError(f"Не сохранился текст в области {coordinates}")

    return {
        "rows": int(check_document.ВысотаТаблицы),
        "columns": int(check_document.ШиринаТаблицы),
        "placements": len(PLACEMENTS),
        "areas": len(placement_checks),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    result = generate(args.base.resolve(), args.output.resolve())
    print(
        f"[OK] Создан файл: {args.output.resolve()}\n"
        f"[OK] Проверка чтением 1С: строк={result['rows']}, "
        f"колонок={result['columns']}, режимов={result['placements']}, "
        f"областей={result['areas']}"
    )


if __name__ == "__main__":
    main()
