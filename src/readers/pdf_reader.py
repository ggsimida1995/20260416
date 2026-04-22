from __future__ import annotations

from pathlib import Path

import fitz

from src.config_store import AISettings
from src.models import PdfCropImage, PdfData
from src.normalizers import normalize_text
from src.readers.signature_ai import extract_signature_fields_with_remote_service
from src.readers.signature_text_parser import parse_signature_text as parse_shared_signature_text
PDF_CROP_RENDER_SCALE = 3.0
PDF_SIGNATURE_BLOCK_FIELD_NAME = "party_a_signature_block"
PDF_PARTY_A_LABELS = ("甲方：", "甲方:")
PDF_PARTY_A_FALLBACK_SIGNATURE_BLOCK_RATIOS = (0.04, 0.50, 0.50, 0.78)


def read_pdf_text(path: Path) -> str:
    return _extract_text_layer(path)


def parse_pdf_text(text: str) -> PdfData:
    return parse_shared_signature_text(text)


def read_pdf(path: Path, ai_settings: AISettings | None = None) -> PdfData:
    crop_images = extract_pdf_field_crops(path)
    parsed = parse_pdf_text(_extract_text_layer(path))
    if _needs_ocr_enrichment(parsed):
        parsed = _merge_pdf_data(parsed, _collect_crop_image_remote_pdf_data(crop_images, ai_settings))
    parsed.has_red_stamp = detect_red_stamp(path)
    parsed.crop_images = crop_images
    return parsed


def extract_pdf_field_crops(path: Path) -> list[PdfCropImage]:
    with fitz.open(path) as document:
        if document.page_count == 0:
            return []

        page_number = document.page_count
        page = document[page_number - 1]
        output_dir = path.parent / "debug" / "pdf_crops" / path.stem
        output_dir.mkdir(parents=True, exist_ok=True)

        crops: list[PdfCropImage] = []
        for field_name, rect in _detect_field_crop_rects(page):
            pixmap = page.get_pixmap(
                matrix=fitz.Matrix(PDF_CROP_RENDER_SCALE, PDF_CROP_RENDER_SCALE),
                clip=rect,
                alpha=False,
            )
            image_path = output_dir / f"{field_name}.png"
            pixmap.save(image_path)
            crops.append(
                PdfCropImage(
                    field_name=field_name,
                    page_number=page_number,
                    image_path=image_path,
                    rect=(rect.x0, rect.y0, rect.x1, rect.y1),
                )
            )
        return crops


def _extract_text_layer(path: Path) -> str:
    with fitz.open(path) as document:
        return _extract_text_from_document(document)


def _extract_text_from_document(document) -> str:
    return " ".join(normalize_text(page.get_text()) for page in document if normalize_text(page.get_text()))


def detect_red_stamp(path: Path, min_ratio: float = 0.01) -> bool:
    with fitz.open(path) as document:
        for page in document:
            pixmap = page.get_pixmap(matrix=fitz.Matrix(1.5, 1.5), alpha=False)
            pixel_count = pixmap.width * pixmap.height
            step = max(1, pixel_count // 5000)
            pixels = []
            for index in range(0, pixel_count, step):
                offset = index * pixmap.n
                pixels.append(tuple(pixmap.samples[offset : offset + 3]))
            if has_red_dominant_pixels(pixels, min_ratio=min_ratio):
                return True
    return False


def has_red_dominant_pixels(pixels, min_ratio: float = 0.01) -> bool:
    pixels = list(pixels)
    if not pixels:
        return False

    red_pixels = 0
    for pixel in pixels:
        if len(pixel) < 3:
            continue
        red, green, blue = pixel[:3]
        if red >= 180 and red > green + 40 and red > blue + 40:
            red_pixels += 1

    return (red_pixels / len(pixels)) >= min_ratio


def _needs_ocr_enrichment(parsed: PdfData) -> bool:
    return not (parsed.signer_name and parsed.signer_phone and parsed.sign_date)


def _collect_crop_image_remote_pdf_data(crop_images: list[PdfCropImage], ai_settings: AISettings | None) -> PdfData:
    for crop_image in crop_images:
        if crop_image.field_name != PDF_SIGNATURE_BLOCK_FIELD_NAME:
            continue
        parsed = extract_signature_fields_with_remote_service(crop_image.image_path, ai_settings)
        if parsed.signer_name or parsed.signer_phone or parsed.sign_date:
            return parsed
    return PdfData()


def _merge_pdf_data(base: PdfData, voted: PdfData) -> PdfData:
    return PdfData(
        signer_name=base.signer_name or voted.signer_name,
        signer_phone=base.signer_phone or voted.signer_phone,
        sign_date=base.sign_date or voted.sign_date,
        has_red_stamp=base.has_red_stamp or voted.has_red_stamp,
    )


def _crop_rect_from_ratios(page_rect: fitz.Rect, ratios: tuple[float, float, float, float]) -> fitz.Rect:
    x0_ratio, y0_ratio, x1_ratio, y1_ratio = ratios
    rect = fitz.Rect(
        page_rect.x0 + page_rect.width * x0_ratio,
        page_rect.y0 + page_rect.height * y0_ratio,
        page_rect.x0 + page_rect.width * x1_ratio,
        page_rect.y0 + page_rect.height * y1_ratio,
    )
    rect = rect & page_rect
    if rect.is_empty or rect.width <= 1 or rect.height <= 1:
        raise ValueError(f"Invalid crop rect for ratios: {ratios}")
    return rect


def _detect_field_crop_rects(page) -> list[tuple[str, fitz.Rect]]:
    anchor_rect = _find_party_a_anchor_rect(page)
    if anchor_rect is not None:
        return [(PDF_SIGNATURE_BLOCK_FIELD_NAME, _build_signature_block_rect_from_anchor(page, anchor_rect))]

    return [
        (
            PDF_SIGNATURE_BLOCK_FIELD_NAME,
            _crop_rect_from_ratios(page.rect, PDF_PARTY_A_FALLBACK_SIGNATURE_BLOCK_RATIOS),
        )
    ]


def _find_party_a_anchor_rect(page) -> fitz.Rect | None:
    for label in PDF_PARTY_A_LABELS:
        matches = page.search_for(label)
        if matches:
            return sorted(matches, key=lambda item: (item.y0, item.x0))[0]
    return None


def _build_signature_block_rect_from_anchor(page, anchor_rect: fitz.Rect) -> fitz.Rect:
    page_rect = page.rect
    x0 = max(page_rect.x0, anchor_rect.x0 - page_rect.width * 0.02)
    x1 = min(page_rect.x1, anchor_rect.x0 + page_rect.width * 0.46)
    y0 = max(page_rect.y0, anchor_rect.y0 - page_rect.height * 0.015)
    y1 = min(page_rect.y1, anchor_rect.y0 + page_rect.height * 0.18)
    rect = fitz.Rect(x0, y0, x1, y1) & page_rect
    if rect.is_empty or rect.width <= 1 or rect.height <= 1:
        raise ValueError("Invalid signature block rect from party A anchor")
    return rect
