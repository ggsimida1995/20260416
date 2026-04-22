from datetime import date
from pathlib import Path

import fitz

from src.config_store import AISettings
from src.readers import pdf_reader
from src.readers.pdf_reader import has_red_dominant_pixels, parse_pdf_text, read_pdf_text


def test_parse_pdf_text_extracts_signer_fields():
    parsed = parse_pdf_text("签字人姓名 黄汉民 联系电话 14714691425 签字时间 2026-04-10")

    assert parsed.signer_name == "黄汉民"
    assert parsed.signer_phone == "14714691425"
    assert parsed.sign_date.isoformat() == "2026-04-10"


def test_parse_pdf_text_supports_signature_and_phone_labels():
    parsed = parse_pdf_text("签字/盖章：黄汉民 电话：14714691425 2026年4月10日")

    assert parsed.signer_name == "黄汉民"
    assert parsed.signer_phone == "14714691425"
    assert parsed.sign_date.isoformat() == "2026-04-10"


def test_parse_pdf_text_stops_at_second_signature_label():
    parsed = parse_pdf_text("签字/盖章：黄汉民 签字/盖章：（项目经理签字） 电话：14714691425 2026年4月10日")

    assert parsed.signer_name == "黄汉民"


def test_has_red_stamp_detects_red_dominant_pixels():
    pixels = [(255, 0, 0), (230, 20, 20), (240, 30, 30), (0, 0, 0)]

    assert has_red_dominant_pixels(pixels, min_ratio=0.5) is True


def test_read_pdf_text_returns_empty_when_text_layer_is_empty(tmp_path, monkeypatch):
    path = tmp_path / "image-only.pdf"
    document = fitz.open()
    document.new_page()
    document.save(path)
    document.close()

    monkeypatch.setattr(pdf_reader, "_extract_text_from_document", lambda document: "")

    assert read_pdf_text(path) == ""


def test_read_pdf_uses_remote_signature_result_when_text_layer_fields_are_missing(tmp_path, monkeypatch):
    path = tmp_path / "image-only.pdf"
    document = fitz.open()
    document.new_page()
    document.save(path)
    document.close()

    monkeypatch.setattr(pdf_reader, "_extract_text_layer", lambda path: "签字时间 2026-04-10")
    crop_path = tmp_path / "party_a_signature_block.png"
    crop_path.touch()
    crop_image = pdf_reader.PdfCropImage(
        field_name="party_a_signature_block",
        page_number=1,
        image_path=crop_path,
        rect=(0.0, 0.0, 100.0, 100.0),
    )
    monkeypatch.setattr(pdf_reader, "extract_pdf_field_crops", lambda path: [crop_image])
    monkeypatch.setattr(
        "src.readers.pdf_reader.extract_signature_fields_with_remote_service",
        lambda image_path, settings: pdf_reader.PdfData(
            signer_name="黄汉民",
            signer_phone="14714691425",
        ),
    )
    monkeypatch.setattr(pdf_reader, "detect_red_stamp", lambda path: False)

    parsed = pdf_reader.read_pdf(
        path,
        ai_settings=AISettings(
            enabled=True,
            ai_base_url="https://example.com/v1",
            ai_api_key="secret-key",
            ai_model="vision-model",
        ),
    )

    assert parsed.signer_name == "黄汉民"
    assert parsed.signer_phone == "14714691425"
    assert parsed.sign_date.isoformat() == "2026-04-10"


def test_extract_pdf_field_crops_uses_party_a_text_anchor_for_signature_block(tmp_path: Path):
    first = tmp_path / "signature-a.pdf"
    second = tmp_path / "signature-b.pdf"
    _build_signature_pdf(first, block_top=1750, body_line_count=8)
    _build_signature_pdf(second, block_top=1970, body_line_count=18)

    for path in (first, second):
        crops = pdf_reader.extract_pdf_field_crops(path)

        assert [crop.field_name for crop in crops] == ["party_a_signature_block"]
        for crop in crops:
            assert crop.image_path.exists() is True
            assert crop.image_path.parent == path.parent / "debug" / "pdf_crops" / path.stem
            assert _image_has_dark_pixels(crop.image_path) is True


def test_extract_pdf_field_crops_falls_back_when_pdf_has_no_text_layer(tmp_path: Path):
    source = tmp_path / "source.pdf"
    image_only = tmp_path / "image-only.pdf"
    _build_signature_pdf(source, block_top=1880, body_line_count=14)
    _build_image_only_pdf_from_source(source, image_only)

    crops = pdf_reader.extract_pdf_field_crops(image_only)

    assert [crop.field_name for crop in crops] == ["party_a_signature_block"]
    assert crops[0].image_path.exists() is True
    assert _image_has_dark_pixels(crops[0].image_path) is True
    assert _rect_tuple_close(
        crops[0].rect,
        (73.12, 1314.0, 914.0, 2049.84),
        tolerance=6.0,
    )


def test_read_pdf_includes_saved_crop_images(tmp_path: Path, monkeypatch):
    path = tmp_path / "signature.pdf"
    _build_signature_pdf(path, block_top=1880, body_line_count=14)

    monkeypatch.setattr(pdf_reader, "_extract_text_layer", lambda path: "")
    monkeypatch.setattr(
        "src.readers.pdf_reader.extract_signature_fields_with_remote_service",
        lambda image_path, settings: pdf_reader.PdfData(),
    )
    monkeypatch.setattr(pdf_reader, "detect_red_stamp", lambda path: False)

    parsed = pdf_reader.read_pdf(path)

    assert [crop.field_name for crop in parsed.crop_images] == ["party_a_signature_block"]
    assert all(crop.image_path.exists() for crop in parsed.crop_images)


def test_read_pdf_returns_empty_signature_fields_when_remote_service_returns_empty(tmp_path: Path, monkeypatch):
    path = tmp_path / "image-only.pdf"
    document = fitz.open()
    document.new_page()
    document.save(path)
    document.close()

    crop_path = tmp_path / "party_a_signature_block.png"
    crop_path.touch()
    crop_image = pdf_reader.PdfCropImage(
        field_name="party_a_signature_block",
        page_number=1,
        image_path=crop_path,
        rect=(0.0, 0.0, 100.0, 100.0),
    )

    monkeypatch.setattr(pdf_reader, "_extract_text_layer", lambda path: "")
    monkeypatch.setattr(pdf_reader, "extract_pdf_field_crops", lambda path: [crop_image])
    monkeypatch.setattr(
        "src.readers.pdf_reader.extract_signature_fields_with_remote_service",
        lambda image_path, settings: pdf_reader.PdfData(),
    )
    monkeypatch.setattr(pdf_reader, "detect_red_stamp", lambda path: False)

    parsed = pdf_reader.read_pdf(
        path,
        ai_settings=AISettings(
            enabled=True,
            ai_base_url="https://example.com/v1",
            ai_api_key="secret-key",
            ai_model="vision-model",
        ),
    )

    assert parsed.signer_name == ""
    assert parsed.signer_phone == ""
    assert parsed.sign_date is None
    assert [crop.field_name for crop in parsed.crop_images] == ["party_a_signature_block"]


def test_read_pdf_returns_empty_signature_fields_when_remote_service_is_not_configured(tmp_path: Path, monkeypatch):
    path = tmp_path / "image-only.pdf"
    document = fitz.open()
    document.new_page()
    document.save(path)
    document.close()

    crop_path = tmp_path / "party_a_signature_block.png"
    crop_path.touch()
    crop_image = pdf_reader.PdfCropImage(
        field_name="party_a_signature_block",
        page_number=1,
        image_path=crop_path,
        rect=(0.0, 0.0, 100.0, 100.0),
    )

    monkeypatch.setattr(pdf_reader, "_extract_text_layer", lambda path: "")
    monkeypatch.setattr(pdf_reader, "extract_pdf_field_crops", lambda path: [crop_image])
    monkeypatch.setattr(pdf_reader, "detect_red_stamp", lambda path: False)

    parsed = pdf_reader.read_pdf(path)

    assert parsed.signer_name == ""
    assert parsed.signer_phone == ""
    assert parsed.sign_date is None
    assert [crop.field_name for crop in parsed.crop_images] == ["party_a_signature_block"]


def test_read_pdf_prefers_remote_signature_result_before_old_local_ocr(tmp_path: Path, monkeypatch):
    path = tmp_path / "image-only.pdf"
    document = fitz.open()
    document.new_page()
    document.save(path)
    document.close()

    crop_path = tmp_path / "party_a_signature_block.png"
    crop_path.touch()
    crop_image = pdf_reader.PdfCropImage(
        field_name="party_a_signature_block",
        page_number=1,
        image_path=crop_path,
        rect=(0.0, 0.0, 100.0, 100.0),
    )

    monkeypatch.setattr(pdf_reader, "_extract_text_layer", lambda path: "")
    monkeypatch.setattr(pdf_reader, "extract_pdf_field_crops", lambda path: [crop_image])
    monkeypatch.setattr(
        "src.readers.pdf_reader.extract_signature_fields_with_remote_service",
        lambda image_path, settings: pdf_reader.PdfData(
            signer_name="AI姓名",
            signer_phone="13900000000",
            sign_date=date(2026, 4, 12),
        ),
    )
    monkeypatch.setattr(pdf_reader, "detect_red_stamp", lambda path: False)

    parsed = pdf_reader.read_pdf(
        path,
        ai_settings=AISettings(
            enabled=True,
            ai_base_url="https://example.com/v1",
            ai_api_key="secret-key",
            ai_model="vision-model",
        ),
    )

    assert parsed.signer_name == "AI姓名"
    assert parsed.signer_phone == "13900000000"
    assert parsed.sign_date.isoformat() == "2026-04-12"


def _build_signature_pdf(path: Path, block_top: float, body_line_count: int) -> None:
    document = fitz.open()
    page = document.new_page(width=1828, height=2628)
    for index in range(body_line_count):
        page.insert_text(
            fitz.Point(120, 260 + index * 70),
            f"正文内容第 {index + 1} 行",
            fontsize=24,
            color=(0, 0, 0),
        )

    page.insert_text(fitz.Point(120, block_top), "甲方：乳源东阳光氟有限公司", fontsize=28, color=(0, 0, 0))
    page.insert_text(fitz.Point(120, block_top + 100), "签字/盖章：黄汉成", fontsize=30, color=(0, 0, 0))
    page.insert_text(fitz.Point(120, block_top + 200), "电话：14714691425", fontsize=30, color=(0, 0, 0))
    page.insert_text(fitz.Point(220, block_top + 300), "2026年 4月 10日", fontsize=30, color=(0, 0, 0))
    page.insert_text(fitz.Point(120, 2520), "www.hollysys.com", fontsize=18, color=(0, 0, 0))
    document.save(path)
    document.close()


def _build_image_only_pdf_from_source(source_path: Path, target_path: Path) -> None:
    source_document = fitz.open(source_path)
    source_page = source_document[0]
    pixmap = source_page.get_pixmap(matrix=fitz.Matrix(2, 2), alpha=False)
    image_only_document = fitz.open()
    page = image_only_document.new_page(width=source_page.rect.width, height=source_page.rect.height)
    page.insert_image(page.rect, pixmap=pixmap)
    image_only_document.save(target_path)
    image_only_document.close()
    source_document.close()


def _image_has_dark_pixels(path: Path) -> bool:
    pixmap = fitz.Pixmap(str(path))
    pixel_count = pixmap.width * pixmap.height
    for index in range(0, pixel_count, max(1, pixel_count // 5000)):
        offset = index * pixmap.n
        red, green, blue = pixmap.samples[offset : offset + 3]
        if red < 220 or green < 220 or blue < 220:
            return True
    return False


def _rect_tuple_close(actual: tuple[float, float, float, float], expected: tuple[float, float, float, float], tolerance: float) -> bool:
    return all(abs(actual_value - expected_value) <= tolerance for actual_value, expected_value in zip(actual, expected))
