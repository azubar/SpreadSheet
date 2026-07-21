"""Генерация MXL с настройками печати, страницами и разрывами."""

from __future__ import annotations

import argparse
from pathlib import Path

import win32com.client

from generate_mxl_demo import color, font, set_area


PROFILES = (
    ("A4_Книжная", "A4", "Portrait", 210, 297),
    ("A4_Альбомная", "A4", "Landscape", 210, 297),
    ("A3_Книжная", "A3", "Portrait", 297, 420),
)


def merged_label(document, connection, row, text, columns=12, background=(242, 242, 242)):
    area = document.Area(row, 1, row, columns)
    area.Объединить()
    set_area(
        area,
        text=text,
        background=color(connection, background),
        text_color=color(connection, (31, 31, 31)),
        area_font=font(connection, "Arial", 10, bold=True),
        wrap=True,
    )
    document.Area(row, 1, row, columns).ВысотаСтроки = 28
    return area


def settings_row(document, connection, row, name, value, columns=12):
    label = document.Area(row, 1, row, 4)
    label.Объединить()
    set_area(
        label,
        text=name,
        background=color(connection, (221, 235, 247)),
        area_font=font(connection, "Arial", 9, bold=True),
        wrap=True,
    )
    result = document.Area(row, 5, row, columns)
    result.Объединить()
    set_area(
        result,
        text=str(value),
        background=color(connection, (255, 255, 255)),
        area_font=font(connection, "Consolas", 9),
        wrap=True,
    )
    document.Area(row, 1, row, columns).ВысотаСтроки = 24


def data_block(document, connection, begin_row, end_row, begin_column, end_column, prefix):
    for row in range(begin_row, end_row + 1):
        for column in range(begin_column, end_column + 1):
            area = document.Area(row, column, row, column)
            set_area(
                area,
                text=f"{prefix}: строка {row}, колонка {column}",
                background=color(
                    connection,
                    (255, 255, 255) if row % 2 else (226, 239, 218),
                ),
                text_color=color(connection, (31, 31, 31)),
                area_font=font(connection, "Arial", 8),
                wrap=True,
            )
        document.Area(begin_row, begin_column, row, end_column).ВысотаСтроки = 25


def generate_one(base_path: Path, output_path: Path, profile) -> dict[str, int | str]:
    profile_name, page_size, orientation, expected_width, expected_height = profile
    output_path.parent.mkdir(parents=True, exist_ok=True)

    connector = win32com.client.Dispatch("V83.COMConnector")
    connection = connector.Connect(f'File="{base_path}";')
    document = connection.NewObject("SpreadsheetDocument")

    total_columns = 12
    for column in range(1, total_columns + 1):
        document.Area(1, column, 1, column).ШиринаКолонки = 18

    document.РазмерСтраницы = page_size
    document.ОриентацияСтраницы = getattr(
        connection.ОриентацияСтраницы, orientation
    )
    document.ПолеСверху = 10
    document.ПолеСлева = 12
    document.ПолеСнизу = 10
    document.ПолеСправа = 12
    document.МасштабПечати = 85
    document.АвтоМасштаб = False
    document.ЧерноБелаяПечать = True
    document.ЧерноБелыйПросмотр = False
    document.ОтображатьСетку = False
    document.ОтображатьЗаголовки = False
    document.КлючПараметровПечати = f"MXL_DEMO_{profile_name}"

    title = document.Area(1, 1, 1, total_columns)
    title.Объединить()
    set_area(
        title,
        text=f"Демонстрация печати MXL: {profile_name}",
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
            "Демо создано платформой 1С. Файл содержит настройки страницы, "
            "область печати, повторяемые строки/колонки и явные разрывы."
        ),
        background=color(connection, (221, 235, 247)),
        area_font=font(connection, "Arial", 9, italic=True),
        wrap=True,
    )
    document.Area(2, 1, 2, total_columns).ВысотаСтроки = 34

    merged_label(document, connection, 4, "Настройки печати, сохраненные в MXL", total_columns)
    settings = [
        ("РазмерСтраницы", page_size),
        ("ШиринаСтраницы / ВысотаСтраницы", f"{expected_width} x {expected_height} мм"),
        ("ОриентацияСтраницы", "книжная" if orientation == "Portrait" else "альбомная"),
        ("Поля", "сверху 10 мм; слева 12 мм; снизу 10 мм; справа 12 мм"),
        ("МасштабПечати / АвтоМасштаб", "85% / Ложь"),
        ("ЧерноБелаяПечать", "Истина"),
        ("ОтображатьСетку / ОтображатьЗаголовки", "Ложь / Ложь"),
        ("ОбластьПечати", "R1C1:R49C12"),
        ("ПовторятьПриПечатиСтроки", "R1C1:R3C12 — шапка на каждой странице"),
        ("ПовторятьПриПечатиКолонки", "R1C1:R49C1 — первый столбец на каждой странице"),
    ]
    for offset, (name, value) in enumerate(settings, start=5):
        settings_row(document, connection, offset, name, value, total_columns)

    data_title_row = 16
    merged_label(
        document,
        connection,
        data_title_row,
        "Секция 1: горизонтальный разрыв будет установлен после строки 25",
        total_columns,
        background=(217, 225, 242),
    )
    data_block(document, connection, 17, 25, 1, total_columns, "Секция 1")
    document.ВывестиГоризонтальныйРазделительСтраниц()

    merged_label(
        document,
        connection,
        27,
        "Секция 2: после нее будет установлен второй горизонтальный разрыв",
        total_columns,
        background=(217, 225, 242),
    )
    data_block(document, connection, 28, 36, 1, total_columns, "Секция 2")
    document.ВывестиГоризонтальныйРазделительСтраниц()

    merged_label(
        document,
        connection,
        38,
        "Секция 3: вертикальный разделитель страниц добавлен программно",
        total_columns,
        background=(217, 225, 242),
    )
    data_block(document, connection, 39, 47, 1, total_columns, "Секция 3")
    document.ВывестиВертикальныйРазделительСтраниц()

    footer = document.Area(49, 1, 49, total_columns)
    footer.Объединить()
    set_area(
        footer,
        text=(
            "Разрывы страниц: 2 горизонтальных + 1 вертикальный; "
            "проверка выполняется чтением MXL обратно через COM 1С."
        ),
        background=color(connection, (31, 78, 121)),
        text_color=color(connection, (255, 255, 255)),
        area_font=font(connection, "Arial", 9, italic=True),
        wrap=True,
    )
    document.Area(49, 1, 49, total_columns).ВысотаСтроки = 30

    print_area = document.Область(1, 1, 49, total_columns)
    document.ОбластьПечати = print_area
    document.ПовторятьПриПечатиСтроки = document.Область(1, 1, 3, total_columns)
    document.ПовторятьПриПечатиКолонки = document.Область(1, 1, 49, 1)

    pages_before_write = int(document.КоличествоСтраниц())
    if pages_before_write < 2:
        raise RuntimeError(
            f"Для профиля {profile_name} не сформировалось несколько страниц: "
            f"{pages_before_write}"
        )

    document.Write(str(output_path), "mxl")

    check_document = connection.NewObject("SpreadsheetDocument")
    check_document.Read(str(output_path))
    if check_document.РазмерСтраницы != page_size:
        raise RuntimeError(
            f"Не сохранился размер страницы {page_size}: "
            f"получено {check_document.РазмерСтраницы}"
        )
    if int(check_document.ШиринаСтраницы) != expected_width:
        raise RuntimeError("Не сохранилась ширина страницы")
    if int(check_document.ВысотаСтраницы) != expected_height:
        raise RuntimeError("Не сохранилась высота страницы")
    expected_orientation = getattr(connection.ОриентацияСтраницы, orientation)
    if check_document.ОриентацияСтраницы != expected_orientation:
        raise RuntimeError("Не сохранилась ориентация страницы")
    if int(check_document.КоличествоСтраниц()) < 2:
        raise RuntimeError("После чтения MXL пропали разрывы или настройки печати")
    restored_print_area = check_document.ОбластьПечати
    if not restored_print_area or int(restored_print_area.Низ) != 49:
        raise RuntimeError("Не сохранилась область печати")
    if not bool(check_document.ЧерноБелаяПечать):
        raise RuntimeError("Не сохранилась черно-белая печать")
    if bool(check_document.ОтображатьСетку) or bool(check_document.ОтображатьЗаголовки):
        raise RuntimeError("Не сохранились настройки отображения сетки/заголовков")

    return {
        "file": str(output_path),
        "page_size": page_size,
        "orientation": orientation,
        "pages": int(check_document.КоличествоСтраниц()),
        "rows": int(check_document.ВысотаТаблицы),
        "columns": int(check_document.ШиринаТаблицы),
    }


def generate(base_path: Path, output_dir: Path) -> list[dict[str, int | str]]:
    output_dir.mkdir(parents=True, exist_ok=True)
    results = []
    for profile in PROFILES:
        profile_name = profile[0]
        output_path = output_dir / f"ДемонстрацияПечатиРазрывов_{profile_name}.mxl"
        results.append(generate_one(base_path, output_path, profile))
    return results


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()

    results = generate(args.base.resolve(), args.output_dir.resolve())
    for result in results:
        print(
            f"[OK] {result['file']}: размер={result['page_size']}, "
            f"ориентация={result['orientation']}, страниц={result['pages']}, "
            f"строк={result['rows']}, колонок={result['columns']}"
        )


if __name__ == "__main__":
    main()
