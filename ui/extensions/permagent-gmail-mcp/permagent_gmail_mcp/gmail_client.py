"""Gmail API client — read-only operations (Phase 1)."""

import base64
from typing import Any

from googleapiclient.discovery import build

from .auth import get_credentials


def _service():
    creds = get_credentials()
    return build("gmail", "v1", credentials=creds)


def search(query: str, max_results: int = 10) -> list[dict[str, Any]]:
    """Search messages using Gmail query syntax. Returns message metadata."""
    svc = _service()
    resp = svc.users().messages().list(
        userId="me", q=query, maxResults=max_results
    ).execute()
    messages = resp.get("messages", [])
    results = []
    for msg in messages:
        detail = svc.users().messages().get(
            userId="me", id=msg["id"], format="metadata",
            metadataHeaders=["From", "To", "Subject", "Date"],
        ).execute()
        headers = {h["name"]: h["value"] for h in detail.get("payload", {}).get("headers", [])}
        results.append({
            "id": detail["id"],
            "threadId": detail["threadId"],
            "snippet": detail.get("snippet", ""),
            "from": headers.get("From", ""),
            "to": headers.get("To", ""),
            "subject": headers.get("Subject", ""),
            "date": headers.get("Date", ""),
            "labelIds": detail.get("labelIds", []),
        })
    return results


def read(message_id: str) -> dict[str, Any]:
    """Read full email content by message ID."""
    svc = _service()
    msg = svc.users().messages().get(userId="me", id=message_id, format="full").execute()
    headers = {h["name"]: h["value"] for h in msg.get("payload", {}).get("headers", [])}
    body = _extract_body(msg.get("payload", {}))
    return {
        "id": msg["id"],
        "threadId": msg["threadId"],
        "from": headers.get("From", ""),
        "to": headers.get("To", ""),
        "subject": headers.get("Subject", ""),
        "date": headers.get("Date", ""),
        "body": body,
        "labelIds": msg.get("labelIds", []),
    }


def _extract_body(payload: dict) -> str:
    """Extract plain text body from message payload, handling multipart."""
    if payload.get("mimeType") == "text/plain" and payload.get("body", {}).get("data"):
        return base64.urlsafe_b64decode(payload["body"]["data"]).decode("utf-8", errors="replace")
    for part in payload.get("parts", []):
        result = _extract_body(part)
        if result:
            return result
    return ""


def list_labels() -> list[dict[str, str]]:
    """List all Gmail labels."""
    svc = _service()
    resp = svc.users().labels().list(userId="me").execute()
    return [{"id": lbl["id"], "name": lbl["name"]} for lbl in resp.get("labels", [])]


def list_threads(max_results: int = 20, query: str = "") -> list[dict[str, Any]]:
    """List recent threads with pagination."""
    svc = _service()
    params: dict[str, Any] = {"userId": "me", "maxResults": max_results}
    if query:
        params["q"] = query
    resp = svc.users().threads().list(**params).execute()
    threads = resp.get("threads", [])
    results = []
    for t in threads:
        detail = svc.users().threads().get(userId="me", id=t["id"], format="metadata").execute()
        first_msg = detail.get("messages", [{}])[0]
        headers = {h["name"]: h["value"] for h in first_msg.get("payload", {}).get("headers", [])}
        results.append({
            "id": detail["id"],
            "snippet": first_msg.get("snippet", ""),
            "subject": headers.get("Subject", ""),
            "from": headers.get("From", ""),
            "date": headers.get("Date", ""),
            "messageCount": len(detail.get("messages", [])),
        })
    return results
