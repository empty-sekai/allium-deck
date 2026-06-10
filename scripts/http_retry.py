"""Small urllib retry helper for CI network calls."""

from __future__ import annotations

import http.client
import sys
import time
import urllib.error
import urllib.request
from collections.abc import Mapping


DEFAULT_ATTEMPTS = 5
RETRYABLE_HTTP_CODES = {408, 425, 429, 500, 502, 503, 504}
RETRYABLE_ERRORS = (
    urllib.error.URLError,
    http.client.IncompleteRead,
    http.client.RemoteDisconnected,
    TimeoutError,
    ConnectionError,
    OSError,
)


def _retryable(exc: BaseException) -> bool:
    if isinstance(exc, urllib.error.HTTPError):
        return exc.code in RETRYABLE_HTTP_CODES
    return isinstance(exc, RETRYABLE_ERRORS)


def _describe(exc: BaseException) -> str:
    if isinstance(exc, urllib.error.HTTPError):
        return f"HTTP {exc.code} {exc.reason}"
    return f"{type(exc).__name__}: {exc}"


def read_url(
    url: str,
    *,
    headers: Mapping[str, str] | None = None,
    timeout: int = 60,
    attempts: int = DEFAULT_ATTEMPTS,
    label: str = "",
) -> tuple[bytes, dict[str, str]]:
    request = urllib.request.Request(url, headers=dict(headers or {}))
    return read_request(request, timeout=timeout, attempts=attempts, label=label or url)


def read_request(
    request: urllib.request.Request,
    *,
    timeout: int = 60,
    attempts: int = DEFAULT_ATTEMPTS,
    label: str = "",
) -> tuple[bytes, dict[str, str]]:
    attempts = max(1, attempts)
    resource = label or request.full_url
    last_error: BaseException | None = None

    for attempt in range(1, attempts + 1):
        try:
            with urllib.request.urlopen(request, timeout=timeout) as response:
                return response.read(), dict(response.headers.items())
        except BaseException as exc:
            if not _retryable(exc) or attempt >= attempts:
                raise
            last_error = exc
            delay = min(2 ** (attempt - 1), 8)
            print(
                f"[retry] {resource} attempt {attempt}/{attempts} failed: {_describe(exc)}; "
                f"sleep {delay}s",
                file=sys.stderr,
            )
            time.sleep(delay)

    raise RuntimeError(f"download failed for {resource}: {last_error}")
