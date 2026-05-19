use crate::core::models::{AppSettings, PdfData, PdfRecognitionContext};
use crate::core::normalizers::{
    names_match_by_loose_pinyin, normalize_compact_text, normalize_date, normalize_phone,
    normalize_text,
};
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

pub fn read_pdf(
    path: &Path,
    settings: &AppSettings,
    detect_stamp: bool,
    context: &PdfRecognitionContext,
) -> Result<PdfData> {
    if is_image_report(path) {
        let mut data = extract_signature_fields_with_remote_service(path, settings, context)?;
        data.has_red_stamp = if detect_stamp {
            image::open(path)
                .map(|image| has_red_dominant_pixels(&image, 0.01))
                .unwrap_or(false)
        } else {
            false
        };
        return Ok(data);
    }

    let text = read_pdf_text(path)?;
    let text_data = parse_signature_text(&text);
    let mut data = text_data.clone();

    if settings.ai_enabled || needs_ocr_enrichment(&data) {
        match extract_pdf_field_crops(path) {
            Ok(crops) => {
                let remote_data = collect_crop_image_remote_pdf_data(&crops, settings, context)?;
                data = merge_pdf_data(remote_data, text_data);
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

pub fn pdf_file_fingerprint(path: &Path) -> Result<String> {
    let metadata = fs::metadata(path)?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    Ok(format!("{}:{modified}", metadata.len()))
}

fn is_image_report(path: &Path) -> bool {
    path.extension()
        .and_then(|item| item.to_str())
        .map(|extension| matches!(extension.to_lowercase().as_str(), "jpg" | "jpeg" | "png"))
        .unwrap_or(false)
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
    context: &PdfRecognitionContext,
) -> Result<PdfData> {
    for crop in crops {
        if crop.field_name != PDF_SIGNATURE_BLOCK_FIELD_NAME {
            continue;
        }
        let parsed =
            extract_signature_fields_with_remote_service(&crop.image_path, settings, context)?;
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
    context: &PdfRecognitionContext,
) -> Result<PdfData> {
    if !settings.ai_enabled {
        return Ok(PdfData::default());
    }

    if !is_ai_recognition_configured(settings) {
        return Err(anyhow!("AI 识别未配置"));
    }

    let ai_data = extract_signature_fields_with_chat_completions(image_path, settings, context)
        .context("AI 识别不可用")?;
    let mut merged = apply_candidate_confirmation(ai_data, context);

    if is_ocr_recognition_configured(settings) {
        if let Ok(ocr_data) = extract_signature_fields_with_ocr_http(image_path, settings) {
            merged = merge_pdf_data(merged, ocr_data);
        }
    }

    if has_pdf_signature_fields(&merged) {
        Ok(apply_candidate_confirmation(merged, context))
    } else {
        Ok(PdfData::default())
    }
}

fn is_ai_recognition_configured(settings: &AppSettings) -> bool {
    settings.ai_enabled
        && !normalize_text(&settings.ai_base_url).is_empty()
        && !normalize_text(&settings.ai_api_key).is_empty()
        && !normalize_text(&settings.ai_model).is_empty()
}

fn is_ocr_recognition_configured(settings: &AppSettings) -> bool {
    settings.ai_enabled && !normalize_text(&settings.ocr_base_url).is_empty()
}

fn has_pdf_signature_fields(data: &PdfData) -> bool {
    !data.signer_name.is_empty() || !data.signer_phone.is_empty() || data.sign_date.is_some()
}

fn build_ai_prompt(context: &PdfRecognitionContext) -> String {
    let names = json_array_text(&context.candidate_names);
    let phones = json_array_text(&context.candidate_phones);
    let excel_date = context
        .excel_acceptance_date
        .map(|date| date.to_string())
        .unwrap_or_default();
    let start = context
        .acceptance_start
        .map(|date| date.to_string())
        .unwrap_or_default();
    let end = context
        .acceptance_end
        .map(|date| date.to_string())
        .unwrap_or_default();

    format!(
        r#"请识别图片中的甲方手写签字区，并结合候选值做确认。

候选姓名: {names}
候选电话: {phones}
Excel验收日期: "{excel_date}"
Word开始时间: "{start}"
Word完成时间: "{end}"

规则:
1. 姓名优先从候选姓名中选择最像手写签名的一个；相似度 >= 0.90 时返回候选姓名，否则返回你实际识别到的手写姓名。
2. 电话优先从候选电话中选择最像手写电话的一个；只保留数字；相似度 >= 0.90 或完整数字一致时返回候选电话，否则返回你实际识别到的手写电话数字。
3. 日期识别为 YYYY-MM-DD。能确定与 Excel验收日期 或 Word时间区间匹配时返回匹配日期，否则返回你实际识别到的手写日期；完全无法识别才返回空字符串。
4. 不要输出 Markdown，不要输出解释，只返回一个 JSON 对象。

JSON 字段固定:
{{
  "signer_name": "",
  "signer_name_confidence": 0.0,
  "signer_phone": "",
  "signer_phone_confidence": 0.0,
  "sign_date": "",
  "sign_date_confidence": 0.0
}}"#
    )
}

fn json_array_text(values: &[String]) -> String {
    serde_json::to_string(
        &values
            .iter()
            .map(|value| normalize_text(value))
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| "[]".to_string())
}

fn extract_signature_fields_with_chat_completions(
    image_path: &Path,
    settings: &AppSettings,
    context: &PdfRecognitionContext,
) -> Result<PdfData> {
    let prompt = build_ai_prompt(context);
    let payload = json!({
        "model": settings.ai_model,
        "messages": [
            {"role": "system", "content": "你是一个文档字段抽取助手。你只能返回JSON。"},
            {
                "role": "user",
                "content": [
                    {"type": "image_url", "image_url": {"url": build_image_data_url(image_path, settings)?}},
                    {"type": "text", "text": prompt}
                ]
            }
        ],
        "temperature": 0
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
    parse_ocr_http_pdf_data(&response)
}

fn post_remote_json(
    url: &str,
    api_key: &str,
    payload: &Value,
    timeout_seconds: i64,
) -> Result<Value> {
    let mut headers = HeaderMap::new();
    if !normalize_text(api_key).is_empty() {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {api_key}"))?,
        );
    }
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

fn parse_ocr_http_pdf_data(payload: &Value) -> Result<PdfData> {
    if payload
        .get("success")
        .and_then(Value::as_bool)
        .is_some_and(|success| !success)
    {
        let message = payload
            .get("error")
            .and_then(Value::as_str)
            .map(normalize_text)
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| "OCR 服务返回失败".to_string());
        return Err(anyhow!(message));
    }
    if let Some(data) = parse_json_payload_to_pdf_data(payload) {
        return Ok(data);
    }
    let text = extract_ocr_http_text(payload);
    if text.is_empty() {
        Ok(PdfData::default())
    } else {
        Ok(parse_signature_text(&text))
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
        || object.contains_key("matched_name")
        || object.contains_key("matched_phone")
        || object.contains_key("matched_date")
    {
        return Some(pdf_data_from_mapping(payload));
    }
    let data = payload.get("data")?;
    let data_object = data.as_object()?;
    if data_object.contains_key("signer_name")
        || data_object.contains_key("signer_phone")
        || data_object.contains_key("sign_date")
        || data_object.contains_key("matched_name")
        || data_object.contains_key("matched_phone")
        || data_object.contains_key("matched_date")
    {
        Some(pdf_data_from_mapping(data))
    } else {
        None
    }
}

fn pdf_data_from_mapping(data: &Value) -> PdfData {
    PdfData {
        project_code: json_string_field(data, &["project_code"]),
        signer_name: json_string_field(data, &["signer_name", "matched_name"]),
        signer_phone: normalize_phone(&json_string_field(data, &["signer_phone", "matched_phone"])),
        sign_date: json_date_field(data, &["sign_date", "matched_date"]),
        has_red_stamp: false,
        signer_name_confidence: json_confidence_field(
            data,
            &[
                "signer_name_confidence",
                "name_confidence",
                "matched_name_confidence",
            ],
        ),
        signer_phone_confidence: json_confidence_field(
            data,
            &[
                "signer_phone_confidence",
                "phone_confidence",
                "matched_phone_confidence",
            ],
        ),
        sign_date_confidence: json_confidence_field(
            data,
            &[
                "sign_date_confidence",
                "date_confidence",
                "matched_date_confidence",
            ],
        ),
    }
}

fn json_string_field(data: &Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(value) = data.get(*key) {
            if let Some(text) = value.as_str() {
                let normalized = normalize_text(text);
                if !normalized.is_empty() {
                    return normalized;
                }
            }
            if value.is_number() || value.is_boolean() {
                let normalized = normalize_text(&value.to_string());
                if !normalized.is_empty() {
                    return normalized;
                }
            }
        }
    }
    String::new()
}

fn json_date_field(data: &Value, keys: &[&str]) -> Option<chrono::NaiveDate> {
    let text = json_string_field(data, keys);
    normalize_date(&text)
}

fn json_confidence_field(data: &Value, keys: &[&str]) -> Option<f64> {
    for key in keys {
        let Some(value) = data.get(*key) else {
            continue;
        };
        if let Some(confidence) = normalize_confidence_value(value) {
            return Some(confidence);
        }
    }
    None
}

fn normalize_confidence_value(value: &Value) -> Option<f64> {
    let number = match value {
        Value::Number(number) => number.as_f64()?,
        Value::String(text) => normalize_text(text)
            .trim_end_matches('%')
            .trim()
            .parse::<f64>()
            .ok()?,
        _ => return None,
    };
    let normalized = if number > 1.0 { number / 100.0 } else { number };
    (0.0..=1.0).contains(&normalized).then_some(normalized)
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
        vec!["data"],
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

fn apply_candidate_confirmation(mut data: PdfData, context: &PdfRecognitionContext) -> PdfData {
    data.signer_name = confirm_name_candidate(
        &data.signer_name,
        data.signer_name_confidence,
        &context.candidate_names,
    );
    data.signer_phone = confirm_phone_candidate(
        &data.signer_phone,
        data.signer_phone_confidence,
        &context.candidate_phones,
    );
    data.sign_date = confirm_date_candidate(data.sign_date, data.sign_date_confidence);
    data
}

fn confirm_name_candidate(value: &str, _confidence: Option<f64>, candidates: &[String]) -> String {
    let normalized = normalize_text(value);
    if normalized.is_empty() || candidates.is_empty() {
        return normalized;
    }
    if let Some(candidate) = find_name_candidate(&normalized, candidates) {
        return candidate;
    }
    normalized
}

fn find_name_candidate(value: &str, candidates: &[String]) -> Option<String> {
    let normalized = normalize_compact_text(value);
    candidates
        .iter()
        .map(|candidate| normalize_text(candidate))
        .filter(|candidate| !candidate.is_empty())
        .find(|candidate| {
            normalize_compact_text(candidate) == normalized
                || names_match_by_loose_pinyin(candidate, value)
        })
}

fn confirm_phone_candidate(value: &str, _confidence: Option<f64>, candidates: &[String]) -> String {
    let normalized = normalize_phone(value);
    if normalized.is_empty() || candidates.is_empty() {
        return normalized;
    }
    if let Some(candidate) = find_phone_candidate(&normalized, candidates) {
        return candidate;
    }
    normalized
}

fn find_phone_candidate(value: &str, candidates: &[String]) -> Option<String> {
    candidates
        .iter()
        .map(|candidate| normalize_phone(candidate))
        .filter(|candidate| !candidate.is_empty())
        .find(|candidate| candidate == value)
}

fn confirm_date_candidate(
    value: Option<chrono::NaiveDate>,
    _confidence: Option<f64>,
) -> Option<chrono::NaiveDate> {
    value
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
        signer_name_confidence: base
            .signer_name_confidence
            .or(remote.signer_name_confidence),
        signer_phone_confidence: base
            .signer_phone_confidence
            .or(remote.signer_phone_confidence),
        sign_date_confidence: base.sign_date_confidence.or(remote.sign_date_confidence),
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

#[cfg(test)]
mod tests {
    use super::{
        apply_candidate_confirmation, normalize_confidence_value, parse_structured_or_empty,
    };
    use crate::core::models::{PdfData, PdfRecognitionContext};
    use chrono::NaiveDate;
    use serde_json::json;

    #[test]
    fn parses_ai_confidence_fields() {
        let data = parse_structured_or_empty(
            r#"{
                "signer_name":"张三",
                "signer_name_confidence":0.94,
                "signer_phone":"138 0013 8000",
                "signer_phone_confidence":"95%",
                "sign_date":"2026-04-27",
                "sign_date_confidence":92
            }"#,
        );

        assert_eq!(data.signer_name, "张三");
        assert_eq!(data.signer_name_confidence, Some(0.94));
        assert_eq!(data.signer_phone, "13800138000");
        assert_eq!(data.signer_phone_confidence, Some(0.95));
        assert_eq!(
            data.sign_date,
            Some(NaiveDate::from_ymd_opt(2026, 4, 27).unwrap())
        );
        assert_eq!(data.sign_date_confidence, Some(0.92));
    }

    #[test]
    fn candidate_confirmation_keeps_exact_candidates() {
        let context = PdfRecognitionContext {
            candidate_names: vec!["张三".to_string()],
            candidate_phones: vec!["13800138000".to_string()],
            ..PdfRecognitionContext::default()
        };
        let data = PdfData {
            signer_name: "张三".to_string(),
            signer_phone: "138 0013 8000".to_string(),
            signer_name_confidence: Some(0.91),
            signer_phone_confidence: Some(0.91),
            ..PdfData::default()
        };

        let confirmed = apply_candidate_confirmation(data, &context);

        assert_eq!(confirmed.signer_name, "张三");
        assert_eq!(confirmed.signer_phone, "13800138000");
    }

    #[test]
    fn candidate_confirmation_keeps_low_confidence_ai_name_when_no_candidate_match() {
        let context = PdfRecognitionContext {
            candidate_names: vec!["张三".to_string()],
            ..PdfRecognitionContext::default()
        };
        let data = PdfData {
            signer_name: "李四".to_string(),
            signer_name_confidence: Some(0.7),
            ..PdfData::default()
        };

        let confirmed = apply_candidate_confirmation(data, &context);

        assert_eq!(confirmed.signer_name, "李四");
    }

    #[test]
    fn candidate_confirmation_keeps_low_confidence_ai_date() {
        let data = PdfData {
            sign_date: Some(NaiveDate::from_ymd_opt(2026, 4, 27).unwrap()),
            sign_date_confidence: Some(0.7),
            ..PdfData::default()
        };

        let confirmed = apply_candidate_confirmation(data, &PdfRecognitionContext::default());

        assert_eq!(
            confirmed.sign_date,
            Some(NaiveDate::from_ymd_opt(2026, 4, 27).unwrap())
        );
    }

    #[test]
    fn normalizes_percent_confidence() {
        assert_eq!(normalize_confidence_value(&json!("90%")), Some(0.9));
        assert_eq!(normalize_confidence_value(&json!(91)), Some(0.91));
    }
}
