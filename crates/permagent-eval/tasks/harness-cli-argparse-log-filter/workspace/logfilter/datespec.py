from datetime import date


def parse_date(text: str) -> date:
    """Parse 'YYYY-MM-DD' into a date, raising ValueError on bad input."""
    parts = text.split("-")
    if len(parts) != 3:
        raise ValueError(f"invalid date: {text!r}")
    y, m, d = (int(p) for p in parts)
    return date(y, m, d)
