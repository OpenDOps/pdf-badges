"""Column letters."""


def excel_id(index: int) -> str:
    """Map a 0-based column index to the sheet letter. 0 is A, 26 is AA."""
    number = index + 1
    letters = []
    while number:
        number, remainder = divmod(number - 1, 26)
        letters.append(chr(ord("A") + remainder))
    return "".join(reversed(letters))
