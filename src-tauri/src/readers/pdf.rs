use crate::core::models::{AppSettings, PdfData};
use crate::core::normalizers::{normalize_date, normalize_phone, normalize_text};
use crate::readers::signature_text::parse_signature_text;
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageEncoder, ImageFormat};
use pdfium_auto::bind_bundled;
use pdfium_render::prelude::*;
use regex::Regex;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const PDF_SIGNATURE_BLOCK_FIELD_NAME: &str = "party_a_signature_block";
const PDF_CROP_RENDER_SCALE: f32 = 3.0;
const PDF_STAMP_RENDER_SCALE: f32 = 1.5;
const PDF_PARTY_A_FALLBACK_SIGNATURE_BLOCK_RATIOS: (f32, f32, f32, f32) = (0.04, 0.50, 0.50, 0.78);
const AI_PROMPT: &str = "请从这张甲方签字区图片中提取签字人姓名、电话、日期。只返回一个JSON对象，不要输出额外说明。字段固定为 signer_name、signer_phone、sign_date。如果某个字段无法确认，请返回空字符串。日期统一输出为 YYYY-MM-DD。";
const OCR_HTTP_PROMPT: &str = "请识别图片中的甲方签字区文字，优先保留姓名、电话、日期。";

static PDFIUM_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone)]
struct PdfCropImage {
    field_name: String,
    image_path: PathBuf,
}

#[derive(Debug, Clone, Copy)]
struct CropRect {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

pub fn read_pdf(path: &Path, settings: &AppSettings, detect_stamp: bool) -> Result<PdfData> {
    let text = read_pdf_text(path)?;
    let mut data = parse_signature_text(&text);

    if needs_ocr_enrichment(&data) {
        match extract_pdf_field_crops(path) {
            Ok(crops) => {
                let remote_data = collect_crop_image_remote_pdf_data(&crops, settings)?;
                data = merge_pdf_data(data, remote_data);
            }
            Err(error) => {
                if !is_pdfium_unavailable(&error) {
                    return Err(error);
                }
            }
        }
    }

    data.has_red_stamp = if detect_stamp {
        detect_red_stamp(path).unwrap_or(false)
    } else {
        false
    };
    Ok(data)
}

fn read_pdf_text(path: &Path) -> Result<String> {
    with_pdfium_document(path, |document| {
        let pages = document.pages();
        let mut text = String::new();
        for page_index in 0..pages.len() {
            let page = pages.get(page_index)?;
            let Ok(page_text) = page.text() else {
                continue;
            };
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&page_text.all());
        }
        Ok(text)
    })
}

fn extract_pdf_field_crops(path: &Path) -> Result<Vec<PdfCropImage>> {
    with_pdfium_document(path, |document| {
        let pages = document.pages();
        if pages.is_empty() {
            return Ok(Vec::new());
        }
        let page_index = pages.len() - 1;
        let page = pages.get(page_index)?;
        let crop_rect = detect_signature_crop_rect(&page)?;
        let crop_path = render_crop(path, &page, crop_rect)?;
        Ok(vec![PdfCropImage {
            field_name: PDF_SIGNATURE_BLOCK_FIELD_NAME.to_string(),
            image_path: crop_path,
        }])
    })
}

fn detect_red_stamp(path: &Path) -> Result<bool> {
    with_pdfium_document(path, |document| {
        let pages = document.pages();
        if pages.is_empty() {
            return Ok(false);
        }

        let mut indexes = Vec::new();
        indexes.push(pages.len() - 1);
        indexes.extend(0..pages.len().saturating_sub(1));
        for page_index in indexes {
            let page = pages.get(page_index)?;
            let image = render_page_image(&page, PDF_STAMP_RENDER_SCALE)?;
            if has_red_dominant_pixels(&image, 0.01) {
                return Ok(true);
            }
        }
        Ok(false)
    })
}

fn with_pdfium_document<T>(
    path: &Path,
    action: impl FnOnce(&PdfDocument<'_>) -> Result<T>,
) -> Result<T> {
    let lock = PDFIUM_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().map_err(|_| anyhow!("PDFium lock poisoned"))?;
    let pdfium = bind_bundled().map_err(|error| anyhow!("PDFium 初始化失败: {error}"))?;
    let document = pdfium
        .load_pdf_from_file(path, None)
        .with_context(|| format!("PDFium 无法读取 PDF: {}", path.display()))?;
    action(&document)
}

fn detect_signature_crop_rect(page: &PdfPage<'_>) -> Result<CropRect> {
    if let Some(anchor) = find_party_a_anchor_rect(page)? {
        return Ok(build_signature_block_rect_from_anchor(page, anchor));
    }
    Ok(crop_rect_from_ratios(
        page.width().value,
        page.height().value,
        PDF_PARTY_A_FALLBACK_SIGNATURE_BLOCK_RATIOS,
    ))
}

fn find_party_a_anchor_rect(page: &PdfPage<'_>) -> Result<Option<CropRect>> {
    let page_text = page.text()?;
    for label in ["甲方：", "甲方:"] {
        let search = page_text.search(label, &PdfSearchOptions::new())?;
        if let Some(segments) = search.find_next() {
            if let Ok(segment) = segments.get(0) {
                return Ok(Some(CropRect::from_pdf_rect(
                    segment.bounds(),
                    page.height().value,
                )));
            }
        }
    }
    Ok(None)
}

fn build_signature_block_rect_from_anchor(page: &PdfPage<'_>, anchor: CropRect) -> CropRect {
    let page_width = page.width().value;
    let page_height = page.height().value;
    CropRect {
        x0: (anchor.x0 - page_width * 0.02).max(0.0),
        y0: (anchor.y0 - page_height * 0.015).max(0.0),
        x1: (anchor.x0 + page_width * 0.46).min(page_width),
        y1: (anchor.y0 + page_height * 0.18).min(page_height),
    }
}

fn crop_rect_from_ratios(
    page_width: f32,
    page_height: f32,
    ratios: (f32, f32, f32, f32),
) -> CropRect {
    let (x0, y0, x1, y1) = ratios;
    CropRect {
        x0: page_width * x0,
        y0: page_height * y0,
        x1: page_width * x1,
        y1: page_height * y1,
    }
}

fn render_crop(path: &Path, page: &PdfPage<'_>, rect: CropRect) -> Result<PathBuf> {
    let rendered = render_page_image(page, PDF_CROP_RENDER_SCALE)?;
    let width_scale = rendered.width() as f32 / page.width().value.max(1.0);
    let height_scale = rendered.height() as f32 / page.height().value.max(1.0);
    let x = (rect.x0 * width_scale).round().max(0.0) as u32;
    let y = (rect.y0 * height_scale).round().max(0.0) as u32;
    let crop_width = ((rect.x1 - rect.x0) * width_scale).round().max(1.0) as u32;
    let crop_height = ((rect.y1 - rect.y0) * height_scale).round().max(1.0) as u32;
    let bounded_width = crop_width.min(rendered.width().saturating_sub(x).max(1));
    let bounded_height = crop_height.min(rendered.height().saturating_sub(y).max(1));
    let crop = rendered.crop_imm(x, y, bounded_width, bounded_height);

    let output_dir = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("debug")
        .join("pdf_crops")
        .join(path.file_stem().unwrap_or_default());
    fs::create_dir_all(&output_dir)?;
    let image_path = output_dir.join(format!("{PDF_SIGNATURE_BLOCK_FIELD_NAME}.png"));
    crop.save_with_format(&image_path, ImageFormat::Png)?;
    Ok(image_path)
}

fn render_page_image(page: &PdfPage<'_>, scale: f32) -> Result<DynamicImage> {
    let width = (page.width().value * scale).round().max(1.0) as i32;
    let height = (page.height().value * scale).round().max(1.0) as i32;
    Ok(page
        .render_with_config(&PdfRenderConfig::new().set_target_size(width, height))?
        .as_image()
        .into_rgb8()
        .into())
}

fn has_red_dominant_pixels(image: &DynamicImage, min_ratio: f32) -> bool {
    let rgb = image.to_rgb8();
    let pixel_count = rgb.width() as usize * rgb.height() as usize;
    if pixel_count == 0 {
        return false;
    }
    let step = (pixel_count / 5_000).max(1);
    let mut sampled = 0usize;
    let mut red_pixels = 0usize;
    for (index, pixel) in rgb.pixels().enumerate() {
        if index % step != 0 {
            continue;
        }
        sampled += 1;
        let [red, green, blue] = pixel.0;
        if red >= 180 && red > green.saturating_add(40) && red > blue.saturating_add(40) {
            red_pixels += 1;
        }
    }
    sampled > 0 && (red_pixels as f32 / sampled as f32) >= min_ratio
}

fn needs_ocr_enrichment(data: &PdfData) -> bool {
    data.signer_name.is_empty() || data.signer_phone.is_empty() || data.sign_date.is_none()
}

fn collect_crop_image_remote_pdf_data(
    crops: &[PdfCropImage],
    settings: &AppSettings,
) -> Result<PdfData> {
    for crop in crops {
        if crop.field_name != PDF_SIGNATURE_BLOCK_FIELD_NAME {
            continue;
        }
        let parsed = extract_signature_fields_with_remote_service(&crop.image_path, settings)?;
        if !parsed.signer_name.is_empty()
            || !parsed.signer_phone.is_empty()
            || parsed.sign_date.is_some()
        {
            return Ok(parsed);
        }
    }
    Ok(PdfData::default())
}

fn extract_signature_fields_with_remote_service(
    image_path: &Path,
    settings: &AppSettings,
) -> Result<PdfData> {
    if !settings.ai_enabled {
        return Ok(PdfData::default());
    }

    if is_ai_recognition_configured(settings) {
        let parsed = extract_signature_fields_with_chat_completions(image_path, settings)?;
        if !parsed.signer_name.is_empty()
            || !parsed.signer_phone.is_empty()
            || parsed.sign_date.is_some()
        {
            return Ok(parsed);
        }
    }

    if is_ocr_recognition_configured(settings) {
        return extract_signature_fields_with_ocr_http(image_path, settings);
    }

    Ok(PdfData::default())
}

fn is_ai_recognition_configured(settings: &AppSettings) -> bool {
    settings.ai_enabled
        && !normalize_text(&settings.ai_base_url).is_empty()
        && !normalize_text(&settings.ai_api_key).is_empty()
        && !normalize_text(&settings.ai_model).is_empty()
}

fn is_ocr_recognition_configured(settings: &AppSettings) -> bool {
    settings.ai_enabled
        && !normalize_text(&settings.ocr_base_url).is_empty()
        && !normalize_text(&settings.ocr_api_key).is_empty()
}

fn extract_signature_fields_with_chat_completions(
    image_path: &Path,
    settings: &AppSettings,
) -> Result<PdfData> {
    let payload = json!({
        "model": settings.ai_model,
        "messages": [
            {"role": "system", "content": "你是一个文档字段抽取助手。你只能返回JSON。"},
            {
                "role": "user",
                "content": [
                    {"type": "image_url", "image_url": {"url": build_image_data_url(image_path, settings)?}},
                    {"type": "text", "text": AI_PROMPT}
                ]
            }
        ]
    });
    let response = post_remote_json(
        &build_chat_completions_url(&settings.ai_base_url),
        &settings.ai_api_key,
        &payload,
        settings.request_timeout_seconds,
    )?;
    parse_ai_pdf_data(&response)
}

fn extract_signature_fields_with_ocr_http(
    image_path: &Path,
    settings: &AppSettings,
) -> Result<PdfData> {
    let payload = json!({
        "image_base64": build_image_base64(image_path, settings)?,
        "image_mime_type": "image/jpeg",
        "prompt": OCR_HTTP_PROMPT
    });
    let response = post_remote_json(
        &build_ocr_http_url(&settings.ocr_base_url),
        &settings.ocr_api_key,
        &payload,
        settings.request_timeout_seconds,
    )?;
    Ok(parse_ocr_http_pdf_data(&response))
}

fn post_remote_json(
    url: &str,
    api_key: &str,
    payload: &Value,
    timeout_seconds: i64,
) -> Result<Value> {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {api_key}"))?,
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Ok(Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_seconds.max(1) as u64))
        .default_headers(headers)
        .build()?
        .post(url)
        .json(payload)
        .send()?
        .error_for_status()?
        .json()?)
}

fn build_chat_completions_url(base_url: &str) -> String {
    let normalized = normalize_text(base_url).trim_end_matches('/').to_string();
    if normalized.ends_with("/chat/completions") {
        normalized
    } else {
        format!("{normalized}/chat/completions")
    }
}

fn build_ocr_http_url(base_url: &str) -> String {
    normalize_text(base_url).trim_end_matches('/').to_string()
}

fn build_image_data_url(image_path: &Path, settings: &AppSettings) -> Result<String> {
    Ok(format!(
        "data:image/jpeg;base64,{}",
        build_image_base64(image_path, settings)?
    ))
}

fn build_image_base64(image_path: &Path, settings: &AppSettings) -> Result<String> {
    let bytes = compress_image_for_remote_service(image_path, settings.image_max_kb)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn compress_image_for_remote_service(image_path: &Path, max_kb: i64) -> Result<Vec<u8>> {
    let limit_bytes = max_kb.max(1) as usize * 1024;
    let image = image::open(image_path)?.into_rgb8();
    let dynamic = DynamicImage::ImageRgb8(image);
    let mut best = encode_jpeg(&dynamic, 85)?;
    if best.len() <= limit_bytes {
        return Ok(best);
    }
    for scale in [1.0_f32, 0.9, 0.8, 0.7, 0.6, 0.5] {
        let candidate_image = if (scale - 1.0).abs() < f32::EPSILON {
            dynamic.clone()
        } else {
            let width = (dynamic.width() as f32 * scale).round().max(1.0) as u32;
            let height = (dynamic.height() as f32 * scale).round().max(1.0) as u32;
            dynamic.resize(width, height, image::imageops::FilterType::Lanczos3)
        };
        for quality in [80_u8, 70, 60, 50, 40, 32, 24] {
            let candidate = encode_jpeg(&candidate_image, quality)?;
            if candidate.len() < best.len() {
                best = candidate.clone();
            }
            if candidate.len() <= limit_bytes {
                return Ok(candidate);
            }
        }
    }
    Ok(best)
}

fn encode_jpeg(image: &DynamicImage, quality: u8) -> Result<Vec<u8>> {
    let rgb = image.to_rgb8();
    let mut output = Vec::new();
    JpegEncoder::new_with_quality(&mut output, quality).write_image(
        rgb.as_raw(),
        rgb.width(),
        rgb.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(output)
}

fn parse_ai_pdf_data(payload: &Value) -> Result<PdfData> {
    let text = extract_ai_message_text(payload);
    if text.is_empty() {
        return Ok(PdfData::default());
    }
    Ok(parse_structured_or_empty(&text))
}

fn parse_ocr_http_pdf_data(payload: &Value) -> PdfData {
    if let Some(data) = parse_json_payload_to_pdf_data(payload) {
        return data;
    }
    let text = extract_ocr_http_text(payload);
    if text.is_empty() {
        PdfData::default()
    } else {
        parse_signature_text(&text)
    }
}

fn parse_structured_or_empty(text: &str) -> PdfData {
    extract_json_object(text)
        .as_ref()
        .map(pdf_data_from_mapping)
        .unwrap_or_default()
}

fn parse_json_payload_to_pdf_data(payload: &Value) -> Option<PdfData> {
    let object = payload.as_object()?;
    if object.contains_key("signer_name")
        || object.contains_key("signer_phone")
        || object.contains_key("sign_date")
    {
        return Some(pdf_data_from_mapping(payload));
    }
    let data = payload.get("data")?;
    let data_object = data.as_object()?;
    if data_object.contains_key("signer_name")
        || data_object.contains_key("signer_phone")
        || data_object.contains_key("sign_date")
    {
        Some(pdf_data_from_mapping(data))
    } else {
        None
    }
}

fn pdf_data_from_mapping(data: &Value) -> PdfData {
    PdfData {
        project_code: normalize_text(
            data.get("project_code")
                .and_then(Value::as_str)
                .unwrap_or(""),
        ),
        signer_name: normalize_text(
            data.get("signer_name")
                .and_then(Value::as_str)
                .unwrap_or(""),
        ),
        signer_phone: normalize_phone(
            data.get("signer_phone")
                .and_then(Value::as_str)
                .unwrap_or(""),
        ),
        sign_date: data
            .get("sign_date")
            .and_then(Value::as_str)
            .and_then(normalize_date),
        has_red_stamp: false,
    }
}

fn extract_ai_message_text(payload: &Value) -> String {
    let Some(first_choice) = payload
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
    else {
        return String::new();
    };
    let Some(content) = first_choice
        .get("message")
        .and_then(|message| message.get("content"))
    else {
        return String::new();
    };
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    content
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn extract_ocr_http_text(payload: &Value) -> String {
    for path in [
        vec!["text"],
        vec!["result"],
        vec!["result", "text"],
        vec!["data", "text"],
    ] {
        if let Some(text) = value_at_path(payload, &path).and_then(Value::as_str) {
            return normalize_text(text);
        }
    }
    String::new()
}

fn value_at_path<'a>(payload: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = payload;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

fn extract_json_object(text: &str) -> Option<Value> {
    let normalized = normalize_text(text);
    if normalized.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(&normalized) {
        if value.is_object() {
            return Some(value);
        }
    }
    let pattern = Regex::new(r"\{[\s\S]*\}").ok()?;
    let found = pattern.find(text)?;
    let value = serde_json::from_str::<Value>(found.as_str()).ok()?;
    value.is_object().then_some(value)
}

fn merge_pdf_data(base: PdfData, remote: PdfData) -> PdfData {
    PdfData {
        project_code: if base.project_code.is_empty() {
            remote.project_code
        } else {
            base.project_code
        },
        signer_name: if base.signer_name.is_empty() {
            remote.signer_name
        } else {
            base.signer_name
        },
        signer_phone: if base.signer_phone.is_empty() {
            remote.signer_phone
        } else {
            base.signer_phone
        },
        sign_date: base.sign_date.or(remote.sign_date),
        has_red_stamp: base.has_red_stamp || remote.has_red_stamp,
    }
}

fn is_pdfium_unavailable(error: &anyhow::Error) -> bool {
    error.to_string().contains("PDFium 初始化失败")
}

impl CropRect {
    fn from_pdf_rect(rect: PdfRect, page_height: f32) -> Self {
        Self {
            x0: rect.left().value,
            y0: page_height - rect.top().value,
            x1: rect.right().value,
            y1: page_height - rect.bottom().value,
        }
    }
}
