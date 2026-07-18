"""Gmail OAuth2 token management — load, refresh, persist.

Token sources, in order:

1. ``GMAIL_OAUTH_TOKEN`` environment variable — the token JSON injected by
   permagentd at spawn time from the system keyring (the daemon's standard
   ``env_keys`` secret-injection path). Preferred; nothing touches disk.
2. Legacy token file (``GMAIL_TOKEN_PATH`` or the default path) — plaintext
   fallback for tokens that have not been migrated to the keyring yet.

Refreshed tokens are persisted back only in file mode, and atomically: the
new content is staged in a same-directory temp file created 0600 from the
first byte and renamed over the target, so the token is never observable
world-readable or half-written. In keyring mode the refreshed access token is
kept in memory only — the durable refresh token already lives in the keyring.
"""

import json
import os
import tempfile
from pathlib import Path

from google.auth.transport.requests import Request
from google.oauth2.credentials import Credentials

TOKEN_ENV_VAR = "GMAIL_OAUTH_TOKEN"
DEFAULT_TOKEN_PATH = os.path.expanduser("~/.permagent/secrets/gmail_token.json")
SCOPES = ["https://www.googleapis.com/auth/gmail.readonly"]


def _token_path() -> Path:
    return Path(os.environ.get("GMAIL_TOKEN_PATH", DEFAULT_TOKEN_PATH))


def _token_from_env() -> dict | None:
    raw = os.environ.get(TOKEN_ENV_VAR)
    if not raw:
        return None
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        return None


def load_credentials() -> Credentials:
    """Load OAuth2 credentials from the keyring-injected env var or token file."""
    data = _token_from_env()
    if data is None:
        path = _token_path()
        if not path.exists():
            raise FileNotFoundError(
                f"Gmail token not found in ${TOKEN_ENV_VAR} or at {path}. "
                "Run 'permagent integrations connect gmail' or connect via Command Center."
            )
        data = json.loads(path.read_text())
    creds = Credentials.from_authorized_user_info(data, SCOPES)
    return creds


def _persist_token_file(path: Path, payload: dict) -> None:
    """Atomically replace ``path`` with ``payload``, 0600 from the first byte."""
    fd, tmp_name = tempfile.mkstemp(dir=str(path.parent), prefix=".gmail_token-")
    try:
        # mkstemp already creates 0600; enforce explicitly so the guarantee
        # does not hinge on the implementation's default.
        os.fchmod(fd, 0o600)
        with os.fdopen(fd, "w") as f:
            f.write(json.dumps(payload))
        os.replace(tmp_name, path)
    except BaseException:
        try:
            os.unlink(tmp_name)
        except OSError:
            pass
        raise


def refresh_if_needed(creds: Credentials) -> Credentials:
    """Refresh credentials if expired, persisting the updated token (file mode)."""
    if creds.valid:
        return creds
    if not creds.refresh_token:
        raise RuntimeError("Gmail credentials expired and no refresh token available.")
    creds.refresh(Request())

    if os.environ.get(TOKEN_ENV_VAR):
        # Keyring mode: the durable refresh token already lives in the
        # keyring; the short-lived access token is refreshed in memory.
        return creds

    path = _token_path()
    _persist_token_file(
        path,
        {
            "token": creds.token,
            "refresh_token": creds.refresh_token,
            "token_uri": creds.token_uri,
            "client_id": creds.client_id,
            "client_secret": creds.client_secret,
            "scopes": list(creds.scopes or SCOPES),
            "expiry": creds.expiry.isoformat() if creds.expiry else None,
        },
    )
    return creds


def get_credentials() -> Credentials:
    """Load and return valid credentials, refreshing if needed."""
    creds = load_credentials()
    return refresh_if_needed(creds)
