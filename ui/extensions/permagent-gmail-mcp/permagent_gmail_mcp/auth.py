"""Gmail OAuth2 token management — load, refresh, persist."""

import json
import os
from pathlib import Path

from google.auth.transport.requests import Request
from google.oauth2.credentials import Credentials

DEFAULT_TOKEN_PATH = os.path.expanduser("~/.permagent/secrets/gmail_token.json")
SCOPES = ["https://www.googleapis.com/auth/gmail.readonly"]


def _token_path() -> Path:
    return Path(os.environ.get("GMAIL_TOKEN_PATH", DEFAULT_TOKEN_PATH))


def load_credentials() -> Credentials:
    """Load OAuth2 credentials from the token file."""
    path = _token_path()
    if not path.exists():
        raise FileNotFoundError(
            f"Gmail token not found at {path}. "
            "Run 'permagent integrations connect gmail' or connect via Command Center."
        )
    data = json.loads(path.read_text())
    creds = Credentials.from_authorized_user_info(data, SCOPES)
    return creds


def refresh_if_needed(creds: Credentials) -> Credentials:
    """Refresh credentials if expired, persisting the updated token."""
    if creds.valid:
        return creds
    if not creds.refresh_token:
        raise RuntimeError("Gmail credentials expired and no refresh token available.")
    creds.refresh(Request())
    # Persist refreshed token
    path = _token_path()
    path.write_text(json.dumps({
        "token": creds.token,
        "refresh_token": creds.refresh_token,
        "token_uri": creds.token_uri,
        "client_id": creds.client_id,
        "client_secret": creds.client_secret,
        "scopes": list(creds.scopes or SCOPES),
        "expiry": creds.expiry.isoformat() if creds.expiry else None,
    }))
    os.chmod(path, 0o600)
    return creds


def get_credentials() -> Credentials:
    """Load and return valid credentials, refreshing if needed."""
    creds = load_credentials()
    return refresh_if_needed(creds)
