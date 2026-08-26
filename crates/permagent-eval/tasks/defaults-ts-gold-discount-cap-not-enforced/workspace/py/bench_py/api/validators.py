import re

_USERNAME_RE = re.compile(r"^[a-zA-Z0-9_]+$")


def validate_username(username: str) -> bool:
    return 3 <= len(username) <= 20 and bool(_USERNAME_RE.match(username))
