"""
Ingestion-time LLM helper (optional).
Used for: document type classification, entity disambiguation, relationship inference.
Local-only (Ollama or similar). NOT for Q&A or answer generation.

When enabled in source config (llm_helper_enabled=True), the agent may call
these helpers and record llm_helper_used + llm_helper_steps in IngestedItem.
"""

from typing import Any


def classify_document_type(text: str, path: str) -> str:
    """
    Classify document type (invoice, contract, report, etc.) using local LLM.
    Returns a label. Stub: returns "document" until LLM backend is wired.
    """
    _ = text, path
    return "document"


def classify_entity(text: str, phrase: str, context: str | None) -> str:
    """
    Classify entity type (person, company, location, etc.) using local LLM.
    Returns entity_type. Stub: returns "unknown" until LLM backend is wired.
    """
    _ = text, phrase, context
    return "unknown"


def infer_relationships(
    text: str, entities: list[dict[str, Any]]
) -> list[dict[str, Any]]:
    """
    Infer relationships between entities using local LLM.
    Returns list of {from, to, type}. Stub: returns [] until LLM backend is wired.
    """
    _ = text, entities
    return []


def clean_ocr_text(text: str) -> str:
    """
    Post-process OCR output (fix common errors, structure) using local LLM.
    Stub: returns text unchanged until LLM backend is wired.
    """
    return text
