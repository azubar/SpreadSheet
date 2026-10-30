"""Генерация вариантов MXL без платформы 1С.

Скрипт читает готовый файл MXL8 (скобочная сериализация после 16-байтового
заголовка), меняет параметры печати или структуру и записывает новый файл.
Нужен для регрессионных образцов: формат листа, ориентация, масштаб печати,
«Автомасштаб», длинные таблицы без явных разрывов.

Важно: файлы созданы не платформой, а этим скриптом. Значения ключей
параметров печати взяты из файлов, сохранённых 1С (см. docs/format-mxl.md,
§5.1), но поведение 1С при открытии таких файлов не проверялось.

Запуск (из корня репозитория):

    python mxl-demo-collection/tools/mxl_variants.py

Результат — в mxl-demo-collection/generated/.
"""

from __future__ import annotations

import copy
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "ДемонстрацияПечатиРазрывов_A4_Книжная.mxl"
OUTPUT = ROOT / "generated"

HEADER_SIZE = 16  # «MOXCEL\0» + версия + BOM UTF-8


class Str(str):
    """Строка в кавычках (в отличие от «голого» атома: GUID, дробного числа)."""


TOKEN = re.compile(r'\s*(?:(\{)|(\})|("(?:[^"]|"")*")|([^,{}\s]+)|(,))')


def parse(text: str):
    """Скобочная сериализация → вложенные списки (числа, Str, атомы)."""
    stack: list[list] = [[]]
    pos = 0
    while pos < len(text):
        match = TOKEN.match(text, pos)
        if not match:
            pos += 1
            continue
        pos = match.end()
        if match.group(1):
            stack.append([])
        elif match.group(2):
            done = stack.pop()
            stack[-1].append(done)
        elif match.group(3):
            stack[-1].append(Str(match.group(3)[1:-1].replace('""', '"')))
        elif match.group(4):
            token = match.group(4)
            try:
                stack[-1].append(int(token))
            except ValueError:
                stack[-1].append(token)
    if len(stack) != 1:
        raise ValueError("несбалансированные скобки")
    return stack[0][0]


def serialize(value) -> str:
    if isinstance(value, list):
        return "{" + ",".join(serialize(v) for v in value) + "}"
    if isinstance(value, int):
        return str(value)
    if isinstance(value, Str):
        return '"' + value.replace('"', '""') + '"'
    return value  # атом


def load(path: Path):
    raw = path.read_bytes()
    return raw[:HEADER_SIZE], parse(raw[HEADER_SIZE:].decode("utf-8"))


def save(path: Path, header: bytes, root) -> None:
    path.write_bytes(header + serialize(root).encode("utf-8"))


# Ключи параметров печати (docs/format-mxl.md, §5.1)
PAGE_SIZE = 0  # {"S", "A4"}
ORIENTATION = 1  # {"N", 1 — книжная | 2 — альбомная}
PRINT_SCALE = 2  # {"N", проценты}
AUTO_SCALE = 12  # {"N", 0 | 1}
BLACK_AND_WHITE = 13  # {"N", 0 | 1}


def print_settings(root) -> list:
    """Запись параметров печати {{0, N, ключ, значение, ...}}."""
    for value in root:
        if (
            isinstance(value, list)
            and len(value) == 1
            and isinstance(value[0], list)
            and value[0][:1] == [0]
            and len(value[0]) > 3
        ):
            return value[0]
    raise KeyError("в документе нет записи параметров печати")


def set_print_setting(root, key: int, value) -> None:
    record = print_settings(root)
    pairs = record[2:]
    for i in range(0, len(pairs), 2):
        if pairs[i] == key:
            pairs[i + 1] = value
            break
    else:
        pairs += [key, value]
    record[1] = len(pairs) // 2
    record[2:] = pairs


def text(value: str) -> list:
    return [Str("S"), Str(value)]


def number(value: int) -> list:
    return [Str("N"), value]


def rows_block(root):
    """(начало блока строк, список строк [индекс, формат, n, ячейки], хвост)."""
    start = 15
    count = root[start]
    rows = []
    i = start + 1
    for _ in range(count):
        index, fmt, ncells = root[i], root[i + 1], root[i + 2]
        rows.append([index, fmt, ncells, root[i + 3 : i + 3 + 2 * ncells]])
        i += 3 + 2 * ncells
    return start, rows, root[i:]


def long_table(root, extra_rows: int):
    """Длинная таблица без явных разрывов и без области печати."""
    root = copy.deepcopy(root)
    start, rows, tail = rows_block(root)
    template = next(r for r in rows if r[0] == 17)  # строка данных
    for row in rows:
        if row[1] == 14:  # формат строки с разрывом страницы
            row[1] = template[1]
    last = max(r[0] for r in rows)
    for k in range(1, extra_rows + 1):
        row = copy.deepcopy(template)
        row[0] = last + k
        rows.append(row)
    flat = []
    for row in rows:
        flat += [row[0], row[1], row[2]] + row[3]
    root[start] = len(rows)
    root[start + 1 :] = flat + tail
    # после блока строк: группа колонок по умолчанию, затем число строк
    root[start + 1 + len(flat) + 1] = last + extra_rows + 1
    for value in root:
        if isinstance(value, list) and len(value) == 6 and value[0] == 3:
            value[:5] = [0, -1, -1, -1, -1]  # область печати не задана
    return root


def main() -> int:
    OUTPUT.mkdir(exist_ok=True)
    header, root = load(SOURCE)

    variants = {
        "Печать_A4_Альбомная_50.mxl": {ORIENTATION: number(2), PRINT_SCALE: number(50)},
        "Печать_A5_Книжная.mxl": {PAGE_SIZE: text("A5")},
        "Печать_A3_Альбомная_Автомасштаб.mxl": {
            PAGE_SIZE: text("A3"),
            ORIENTATION: number(2),
            AUTO_SCALE: number(1),
        },
        "Печать_A4_Книжная_Цветная.mxl": {BLACK_AND_WHITE: number(0)},
    }
    for name, settings in variants.items():
        variant = copy.deepcopy(root)
        for key, value in settings.items():
            set_print_setting(variant, key, value)
        save(OUTPUT / name, header, variant)
        print(f"[OK] {name}")

    save(OUTPUT / "ДлиннаяТаблица_БезРазрывов.mxl", header, long_table(root, 300))
    print("[OK] ДлиннаяТаблица_БезРазрывов.mxl")

    # контроль: сериализация без изменений восстанавливает файл по смыслу
    if parse(serialize(root)) != root:
        print("[ОШИБКА] сериализация меняет структуру документа", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
