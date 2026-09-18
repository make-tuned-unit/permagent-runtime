from datetime import date

VALID_LEVELS = {"debug", "info", "warn", "error"}


def parse_line(line: str):
    """Parse a log line 'TIMESTAMP LEVEL message...' into (date, level, text)."""
    parts = line.rstrip("\n").split(" ", 2)
    if len(parts) < 3:
        return None
    ts, level, message = parts
    try:
        day = date.fromisoformat(ts.split("T")[0])
    except ValueError:
        return None
    return day, level, message


def filter_lines(lines, level=None, since=None):
    """Return lines matching an optional level and/or since-date filter."""
    out = []
    for line in lines:
        parsed = parse_line(line)
        if parsed is None:
            continue
        day, lvl, _ = parsed
        if level is not None and lvl != level:
            continue
        if since is not None and day < since:
            continue
        out.append(line.rstrip("\n"))
    return out
