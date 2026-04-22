from __future__ import annotations

from dataclasses import dataclass, field
from datetime import date
from pathlib import Path
from typing import Optional


@dataclass
class ProjectFiles:
    project_name: str
    project_dir: Path
    xlsx_path: Optional[Path] = None
    docx_path: Optional[Path] = None
    pdf_path: Optional[Path] = None
    txt_path: Optional[Path] = None
    missing_files: list[str] = field(default_factory=list)


@dataclass
class CompareFailure:
    field_name: str
    message: str
    values: dict[str, object] = field(default_factory=dict)


@dataclass
class CompareResult:
    passed: bool
    failures: list[CompareFailure] = field(default_factory=list)


@dataclass
class ProjectExtraction:
    raw_fields: dict[str, object] = field(default_factory=dict)
    normalized_fields: dict[str, object] = field(default_factory=dict)


@dataclass
class DocxData:
    project_code: str = ""
    project_name: str = ""
    contact_name: str = ""
    contact_phone: str = ""
    contact_names: list[str] = field(default_factory=list)
    contact_phones: list[str] = field(default_factory=list)
    acceptance_start: Optional[date] = None
    acceptance_end: Optional[date] = None
    has_invalid_acceptance_range: bool = False


@dataclass
class PdfData:
    signer_name: str = ""
    signer_phone: str = ""
    sign_date: Optional[date] = None
    has_red_stamp: bool = False
    crop_images: list["PdfCropImage"] = field(default_factory=list)


@dataclass(frozen=True)
class PdfCropImage:
    field_name: str
    page_number: int
    image_path: Path
    rect: tuple[float, float, float, float]


@dataclass
class AppendResult:
    status: str
    appended_row_index: Optional[int] = None


@dataclass
class WorkflowResult:
    appended_count: int = 0
    duplicate_count: int = 0
    failed_count: int = 0
    log_path: Optional[Path] = None
    success_project_names: list[str] = field(default_factory=list)
    success_project_codes: list[str] = field(default_factory=list)
    success_workbook_path: Optional[Path] = None
    error_report_paths: list[Path] = field(default_factory=list)


@dataclass
class WebPhaseResult:
    processed_projects: list[str] = field(default_factory=list)
    skipped_projects: list[str] = field(default_factory=list)


@dataclass
class BatchWorkflowResult:
    web_processed_count: int = 0
    skipped_processed_count: int = 0
    compare_appended_count: int = 0
    compare_duplicate_count: int = 0
    compare_failed_count: int = 0
    cleaned_count: int = 0
    log_path: Optional[Path] = None
    compare_success_project_codes: list[str] = field(default_factory=list)
    compare_success_workbook_path: Optional[Path] = None
    compare_error_report_paths: list[Path] = field(default_factory=list)
