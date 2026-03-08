"""
Normalized ingestion contract models (JSON-serializable).
Matches Rust node_ingest_contract. Pipeline version: 1.
"""

from dataclasses import dataclass, field
from typing import Any, Optional

PIPELINE_VERSION = 1


@dataclass
class IngestedChunk:
    """A single chunk of extracted content."""

    chunk_index: int
    chunk_text: str
    page_number: int = 0

    def to_dict(self) -> dict:
        return {
            "chunk_index": self.chunk_index,
            "chunk_text": self.chunk_text,
            "page_number": self.page_number,
        }

    @classmethod
    def from_dict(cls, d: dict) -> "IngestedChunk":
        return cls(
            chunk_index=d["chunk_index"],
            chunk_text=d["chunk_text"],
            page_number=d.get("page_number", 0),
        )


@dataclass
class IngestedItem:
    """Normalized item from an ingestion agent, ready for core to store."""

    source_id: str
    source_type: str
    item_id: str
    source_display_name: str
    source_origin_label: str
    source_locator: str
    source_open_target: str
    path_or_external_key: str
    content_type: str
    extracted_text: str
    chunks: list[IngestedChunk]
    ocr_attempted: bool
    ocr_used: bool
    extraction_method: str
    ingest_status: str
    source_modified_at: int
    ingested_at: int
    content_hash: str
    pipeline_version: int = PIPELINE_VERSION
    source_parent: Optional[str] = None
    metadata: Optional[dict[str, Any]] = None
    warnings: Optional[list[str]] = None
    failure_reason: Optional[str] = None

    def __post_init__(self) -> None:
        if self.metadata is None:
            self.metadata = {}
        if self.warnings is None:
            self.warnings = []

    def to_dict(self) -> dict:
        d: dict[str, Any] = {
            "source_id": self.source_id,
            "source_type": self.source_type,
            "item_id": self.item_id,
            "source_display_name": self.source_display_name,
            "source_origin_label": self.source_origin_label,
            "source_locator": self.source_locator,
            "source_open_target": self.source_open_target,
            "path_or_external_key": self.path_or_external_key,
            "content_type": self.content_type,
            "extracted_text": self.extracted_text,
            "chunks": [c.to_dict() for c in self.chunks],
            "metadata": self.metadata,
            "ocr_attempted": self.ocr_attempted,
            "ocr_used": self.ocr_used,
            "extraction_method": self.extraction_method,
            "warnings": self.warnings,
            "ingest_status": self.ingest_status,
            "source_modified_at": self.source_modified_at,
            "ingested_at": self.ingested_at,
            "content_hash": self.content_hash,
            "pipeline_version": self.pipeline_version,
        }
        if self.source_parent is not None:
            d["source_parent"] = self.source_parent
        if self.failure_reason is not None:
            d["failure_reason"] = self.failure_reason
        return d
