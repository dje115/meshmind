"""
Filesystem watcher for MeshMind ingestion agent.
Detects new, modified, and deleted files. Persists state for restart resilience.
Local-only; no cloud.
"""

import fnmatch
import hashlib
import json
import os
import time
from dataclasses import dataclass, field
from enum import Enum
from pathlib import Path
from typing import Callable

# Optional: watchdog for live watching
try:
    from watchdog.observers import Observer
    from watchdog.events import FileSystemEventHandler, FileSystemEvent

    HAS_WATCHDOG = True
except ImportError:
    Observer = None  # type: ignore[misc, assignment]
    FileSystemEventHandler = object  # type: ignore[misc, assignment]
    FileSystemEvent = object  # type: ignore[misc, assignment]
    HAS_WATCHDOG = False


class FileStatus(str, Enum):
    DISCOVERED = "discovered"
    UNCHANGED = "unchanged"
    QUEUED = "queued"
    PROCESSING = "processing"
    INGESTED = "ingested"
    SKIPPED_UNSUPPORTED = "skipped_unsupported"
    FAILED_EXTRACTION = "failed_extraction"
    FAILED_OCR = "failed_ocr"
    FAILED_UNKNOWN = "failed_unknown"
    DELETED = "deleted"


@dataclass
class FileState:
    path: str
    content_hash: str
    modified_at: float
    status: FileStatus = FileStatus.DISCOVERED


@dataclass
class WatchedSource:
    source_id: str
    root: Path
    recursion: bool = True
    include_patterns: list[str] = field(default_factory=lambda: ["*"])
    exclude_patterns: list[str] = field(default_factory=list)


def compute_content_hash(path: Path) -> str:
    """Compute SHA-256 hash of file content."""
    h = hashlib.sha256()
    try:
        with open(path, "rb") as f:
            for chunk in iter(lambda: f.read(65536), b""):
                h.update(chunk)
        return h.hexdigest()
    except OSError:
        return ""


def get_modified_time(path: Path) -> float:
    """Get file mtime. Returns 0 on error."""
    try:
        return os.path.getmtime(path)
    except OSError:
        return 0.0


def matches_patterns(path: Path, include: list[str], exclude: list[str]) -> bool:
    """Check if path matches include patterns and not exclude patterns."""
    name = path.name
    for pat in exclude:
        if fnmatch.fnmatch(name, pat):
            return False
    for pat in include:
        if fnmatch.fnmatch(name, pat):
            return True
    return False


# Document extensions (subset; extend as needed)
DOCUMENT_EXTENSIONS = {
    ".pdf", ".docx", ".doc", ".xls", ".xlsx", ".pptx", ".ppt",
    ".txt", ".md", ".rtf",
}


def is_document_file(path: Path) -> bool:
    return path.suffix.lower() in DOCUMENT_EXTENSIONS


def walk_files(root: Path, recursion: bool, include: list[str], exclude: list[str]) -> list[Path]:
    """Collect files under root matching patterns."""
    out: list[Path] = []
    if recursion:
        for dirpath, _dirnames, filenames in os.walk(root):
            for name in filenames:
                p = Path(dirpath) / name
                if matches_patterns(p, include, exclude) and is_document_file(p):
                    out.append(p)
    else:
        try:
            for entry in root.iterdir():
                if entry.is_file() and matches_patterns(entry, include, exclude) and is_document_file(entry):
                    out.append(entry)
        except OSError:
            pass
    return sorted(out)


class WatchStateStore:
    """Persist file states for restart resilience."""

    def __init__(self, state_path: Path) -> None:
        self.state_path = state_path
        self._states: dict[str, FileState] = {}
        self._load()

    def _load(self) -> None:
        if self.state_path.exists():
            try:
                with open(self.state_path, encoding="utf-8") as f:
                    data = json.load(f)
                for k, v in data.items():
                    self._states[k] = FileState(
                        path=k,
                        content_hash=v.get("content_hash", ""),
                        modified_at=v.get("modified_at", 0),
                        status=FileStatus(v.get("status", FileStatus.DISCOVERED.value)),
                    )
            except (json.JSONDecodeError, OSError):
                self._states = {}

    def save(self) -> None:
        data = {
            k: {
                "content_hash": s.content_hash,
                "modified_at": s.modified_at,
                "status": s.status.value,
            }
            for k, s in self._states.items()
        }
        try:
            self.state_path.parent.mkdir(parents=True, exist_ok=True)
            with open(self.state_path, "w", encoding="utf-8") as f:
                json.dump(data, f, indent=2)
        except OSError:
            pass

    def get(self, path: str) -> FileState | None:
        return self._states.get(path)

    def set(self, path: str, state: FileState) -> None:
        self._states[path] = state

    def mark_deleted(self, path: str) -> None:
        existing = self._states.get(path)
        if existing:
            self._states[path] = FileState(
                path=path,
                content_hash=existing.content_hash,
                modified_at=existing.modified_at,
                status=FileStatus.DELETED,
            )


def scan_source(
    source: WatchedSource,
    state_store: WatchStateStore,
    on_queued: Callable[[Path, FileState | None], None],
) -> list[tuple[Path, FileStatus]]:
    """
    Scan a watched source. Returns list of (path, status).
    Calls on_queued(path, prior_state) for files that need processing (new or changed).
    """
    root = source.root.resolve()
    if not root.exists() or not root.is_dir():
        return []

    files = walk_files(root, source.recursion, source.include_patterns, source.exclude_patterns)
    results: list[tuple[Path, FileStatus]] = []

    for path in files:
        path_str = str(path)
        mtime = get_modified_time(path)
        content_hash = compute_content_hash(path)
        prior = state_store.get(path_str)

        if prior is None:
            # New file
            state = FileState(path=path_str, content_hash=content_hash, modified_at=mtime, status=FileStatus.QUEUED)
            state_store.set(path_str, state)
            on_queued(path, None)
            results.append((path, FileStatus.QUEUED))
        elif prior.content_hash == content_hash and prior.modified_at == mtime:
            # Unchanged
            results.append((path, FileStatus.UNCHANGED))
        else:
            # Changed
            state = FileState(path=path_str, content_hash=content_hash, modified_at=mtime, status=FileStatus.QUEUED)
            state_store.set(path_str, state)
            on_queued(path, prior)
            results.append((path, FileStatus.QUEUED))

    state_store.save()
    return results


class FileWatcherHandler(FileSystemEventHandler if HAS_WATCHDOG else object):  # type: ignore
    """Watchdog event handler for real-time file changes."""

    def __init__(
        self,
        state_store: WatchStateStore,
        on_queued: Callable[[Path, FileState | None], None],
        include_patterns: list[str],
        exclude_patterns: list[str],
    ) -> None:
        self.state_store = state_store
        self.on_queued = on_queued
        self.include_patterns = include_patterns
        self.exclude_patterns = exclude_patterns

    def _should_process(self, path: Path) -> bool:
        return matches_patterns(path, self.include_patterns, self.exclude_patterns) and is_document_file(path)

    def _handle_file(self, path: Path, deleted: bool = False) -> None:
        path_str = str(path)
        if deleted:
            self.state_store.mark_deleted(path_str)
            self.state_store.save()
            return
        if not self._should_process(path):
            return
        if not path.exists():
            return
        mtime = get_modified_time(path)
        content_hash = compute_content_hash(path)
        prior = self.state_store.get(path_str)
        state = FileState(path=path_str, content_hash=content_hash, modified_at=mtime, status=FileStatus.QUEUED)
        self.state_store.set(path_str, state)
        self.state_store.save()
        self.on_queued(path, prior)

    def on_created(self, event: "FileSystemEvent") -> None:
        if event.is_directory:
            return
        self._handle_file(Path(event.src_path))

    def on_modified(self, event: "FileSystemEvent") -> None:
        if event.is_directory:
            return
        self._handle_file(Path(event.src_path))

    def on_deleted(self, event: "FileSystemEvent") -> None:
        if event.is_directory:
            return
        path_str = event.src_path
        self.state_store.mark_deleted(path_str)
        self.state_store.save()


def run_watcher(
    sources: list[WatchedSource],
    state_path: Path,
    on_queued: Callable[[Path, FileState | None], None],
    poll_interval: float = 5.0,
) -> None:
    """
    Run the filesystem watcher. Uses watchdog if available; otherwise polls.
    """
    state_store = WatchStateStore(state_path)

    if HAS_WATCHDOG:
        observer = Observer()
        for src in sources:
            if not src.root.exists():
                continue
            handler = FileWatcherHandler(state_store, on_queued, src.include_patterns, src.exclude_patterns)
            observer.schedule(handler, str(src.root), recursive=src.recursion)
        observer.start()
        try:
            while True:
                time.sleep(poll_interval)
        except KeyboardInterrupt:
            observer.stop()
            observer.join()
    else:
        # Fallback: polling scan
        while True:
            for src in sources:
                scan_source(src, state_store, on_queued)
            time.sleep(poll_interval)
