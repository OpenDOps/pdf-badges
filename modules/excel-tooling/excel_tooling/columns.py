"""Column letters."""

_MAX_COLUMN = 16384  # XFD, the last Excel column


def excel_id(index: int) -> str:
    """Map a 0-based column index to the sheet letter. 0 is A, 26 is AA."""
    number = index + 1
    letters = []
    while number:
        number, remainder = divmod(number - 1, 26)
        letters.append(chr(ord("A") + remainder))
    return "".join(reversed(letters))


def parse_excel_id(text: str) -> int | None:
    """Map a sheet letter to a 0-based index. `A` and `a` are 0. Anything else is None."""
    letters = text.strip().upper()
    if not letters.isascii() or not letters.isalpha():
        return None
    number = 0
    for char in letters:
        number = number * 26 + (ord(char) - ord("A") + 1)
        if number > _MAX_COLUMN:
            return None
    return number - 1
