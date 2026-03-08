"""
Publish ingested items to MeshMind core via HTTP/JSON.
Local-only; no cloud.
"""

import json
import os
import time

try:
    import requests
    HAS_REQUESTS = True
except ImportError:
    HAS_REQUESTS = False


def publish_batch(
    items: list,
    api_url: str,
    admin_token: str,
) -> tuple[int, int, str | None]:
    """
    POST items to core's /v1/ingest/items/batch.
    Returns (items_sent, docs_created, error_msg).
    """
    if not HAS_REQUESTS:
        return 0, 0, "requests not installed"

    url = f"{api_url.rstrip('/')}/v1/ingest/items/batch"
    headers = {
        "Content-Type": "application/json",
        "Authorization": f"Bearer {admin_token}",
    }
    payload = {"items": [item.to_dict() if hasattr(item, "to_dict") else item for item in items]}

    try:
        resp = requests.post(url, json=payload, headers=headers, timeout=120)
        if resp.status_code != 200:
            return len(items), 0, f"HTTP {resp.status_code}: {resp.text[:200]}"
        data = resp.json()
        return len(items), data.get("docs_created", 0), None
    except Exception as e:
        return 0, 0, str(e)
