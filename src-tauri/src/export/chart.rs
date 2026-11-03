//! 图表导出（§5）：csv（数据表）、xlsx（数据＋原生图表）、png（栅格折线）。
//!
//! PNG 为无文字依赖的纯 Rust 栅格（轴/网格/折线/图例色块；数值与图例文字
//! 由 CSV/XLSX 变体承载——本实现不内嵌字体，如实声明该限制）。

use super::cells::{csv_row, write_headers, write_text, DecimalTextLog};
use super::{decimal_of, text_of, ExportData, ExportError, ExportResult, SHEET_TREND};
use rust_xlsxwriter::{Chart, ChartType, Workbook};
use serde_json::Value;

/// 图表数据（一指标一序列）。
pub struct ChartSpec {
    pub chart_id: String,
    pub metric_key: String,
    pub metric_label: String,
    pub unit: String,
    pub buckets: Vec<String>,
    pub actual: Vec<Option<String>>,
    pub plan: Vec<Option<String>>,
}

impl ChartSpec {
    pub fn from_series(series: &Value) -> Option<Self> {
        let buckets: Vec<String> = series["buckets"].as_array()?.iter().map(text_of).collect();
        let pick = |key: &str| -> Vec<Option<String>> {
            series[key]
                .as_array()
                .map(|arr| arr.iter().map(decimal_of).collect())
                .unwrap_or_default()
        };
        Some(Self {
            chart_id: format!("chart:{}", text_of(&series["metric_key"])),
            metric_key: text_of(&series["metric_key"]),
            metric_label: text_of(&series["metric_label"]),
            unit: text_of(&series["unit"]),
            buckets,
            actual: pick("actual"),
            plan: pick("plan"),
        })
    }
}

fn find_series<'a>(data: &'a ExportData, chart_id: &str) -> Option<&'a Value> {
    // chart_id 形如 "chart:<metric_key>" 或 "trend:<metric_key>" 或裸 metric_key
    let key = chart_id
        .strip_prefix("chart:")
        .or_else(|| chart_id.strip_prefix("trend:"))
        .unwrap_or(chart_id);
    data.series
        .iter()
        .find(|s| s["metric_key"].as_str() == Some(key))
}

pub fn chart_csv(data: &ExportData, chart_id: &str) -> ExportResult<Vec<u8>> {
    let series = find_series(data, chart_id).ok_or_else(|| {
        ExportError(format!(
            "chart {chart_id} 不存在（可用：分析序列 metric_key）"
        ))
    })?;
    let spec = ChartSpec::from_series(series).ok_or_else(|| ExportError("序列形状非法".into()))?;
    let mut out = String::new();
    out.push_str(&csv_row(&[
        "chart_id".into(),
        spec.metric_key.clone(),
        spec.metric_label.clone(),
        spec.unit.clone(),
    ]));
    out.push('\n');
    out.push_str(&csv_row(&["期间".into(), "实绩".into(), "计划".into()]));
    out.push('\n');
    for (i, bucket) in spec.buckets.iter().enumerate() {
        out.push_str(&csv_row(&[
            bucket.clone(),
            spec.actual.get(i).cloned().flatten().unwrap_or_default(),
            spec.plan.get(i).cloned().flatten().unwrap_or_default(),
        ]));
        out.push('\n');
    }
    Ok(out.into_bytes())
}

pub fn chart_xlsx(
    data: &ExportData,
    chart_id: &str,
    log: &mut DecimalTextLog,
) -> ExportResult<Vec<u8>> {
    let series = find_series(data, chart_id).ok_or_else(|| {
        ExportError(format!(
            "chart {chart_id} 不存在（可用：分析序列 metric_key）"
        ))
    })?;
    let spec = ChartSpec::from_series(series).ok_or_else(|| ExportError("序列形状非法".into()))?;
    let mut book = Workbook::new();
    let ws = book.add_worksheet();
    ws.set_name(SHEET_TREND)
        .map_err(|e| ExportError(e.to_string()))?;
    super::workbook::write_metadata(ws, data)?;
    write_headers(ws, super::workbook::HEADER_ROW, &["期间", "实绩", "计划"])?;
    let start = super::workbook::DATA_ROW;
    for (i, bucket) in spec.buckets.iter().enumerate() {
        let row = start + i as u32;
        write_text(ws, row, 0, bucket)?;
        for (col, values) in [(1u16, &spec.actual), (2u16, &spec.plan)] {
            if let Some(v) = values.get(i).cloned().flatten() {
                super::cells::write_decimal(ws, row, col, SHEET_TREND, &v, log)?;
            }
        }
    }
    let end = start + spec.buckets.len().saturating_sub(1) as u32;
    if spec.buckets.len() >= 2 {
        let mut chart = Chart::new(ChartType::Line);
        for (col, name) in [(1u16, "实绩"), (2u16, "计划")] {
            chart
                .add_series()
                .set_name(name)
                .set_categories((SHEET_TREND, start, 0, end, 0))
                .set_values((SHEET_TREND, start, col, end, col));
        }
        ws.insert_chart(1, 5, &chart)
            .map_err(|e| ExportError(format!("图表插入失败: {e}")))?;
    }
    book.save_to_buffer()
        .map_err(|e| ExportError(e.to_string()))
}

// ---- PNG 栅格（无字体；纯几何） ----

const PNG_W: u32 = 960;
const PNG_H: u32 = 480;
const MARGIN: u32 = 48;
const BG: [u8; 4] = [255, 255, 255, 255];
const FG: [u8; 4] = [40, 40, 40, 255];
const GRID: [u8; 4] = [220, 220, 220, 255];
const ACTUAL: [u8; 4] = [31, 119, 180, 255];
const PLAN: [u8; 4] = [255, 127, 14, 255];

fn parse_all(series: &[Option<String>]) -> Vec<Option<f64>> {
    series
        .iter()
        .map(|v| v.as_deref().and_then(|s| s.parse::<f64>().ok()))
        .collect()
}

pub fn chart_png(data: &ExportData, chart_id: &str) -> ExportResult<Vec<u8>> {
    let series = find_series(data, chart_id).ok_or_else(|| {
        ExportError(format!(
            "chart {chart_id} 不存在（可用：分析序列 metric_key）"
        ))
    })?;
    let spec = ChartSpec::from_series(series).ok_or_else(|| ExportError("序列形状非法".into()))?;
    if spec.buckets.len() < 2 {
        return Err(ExportError(
            "趋势至少 2 个有效桶；缺桶不造值（单桶请用 csv/xlsx 变体）".into(),
        ));
    }
    let actual = parse_all(&spec.actual);
    let plan = parse_all(&spec.plan);
    let values: Vec<f64> = actual
        .iter()
        .chain(plan.iter())
        .filter_map(|v| *v)
        .collect();
    if values.is_empty() {
        return Err(ExportError("序列无有效值".into()));
    }
    let mut min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let mut max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    if min == max {
        min -= 1.0;
        max += 1.0;
    }
    let plot_w = PNG_W - 2 * MARGIN;
    let plot_h = PNG_H - 2 * MARGIN;
    let n = spec.buckets.len() as f64;
    let x_of = |i: usize| -> f64 { MARGIN as f64 + (i as f64) * (plot_w as f64) / (n - 1.0) };
    let y_of =
        |v: f64| -> f64 { PNG_H as f64 - MARGIN as f64 - (v - min) / (max - min) * plot_h as f64 };
    let mut canvas = vec![BG[0]; (PNG_W * PNG_H) as usize * 4];
    // 网格（5 条横线）
    for g in 0..=5 {
        let y = MARGIN + g * plot_h / 5;
        draw_hline(&mut canvas, y);
    }
    // 轴
    draw_vline(&mut canvas, MARGIN);
    draw_hline_solid(&mut canvas, PNG_H - MARGIN);
    // 序列
    for (series_values, color) in [(&actual, ACTUAL), (&plan, PLAN)] {
        let mut prev: Option<(f64, f64)> = None;
        for (i, v) in series_values.iter().enumerate() {
            if let Some(v) = v {
                let point = (x_of(i), y_of(*v));
                if let Some(p) = prev {
                    draw_line(&mut canvas, p, point, color);
                }
                draw_dot(&mut canvas, point, color);
                prev = Some(point);
            } else {
                prev = None; // 缺桶断线
            }
        }
    }
    // 图例色块（左上）
    fill_rect(
        &mut canvas,
        MARGIN + 8,
        MARGIN - 28,
        MARGIN + 24,
        MARGIN - 20,
        ACTUAL,
    );
    fill_rect(
        &mut canvas,
        MARGIN + 96,
        MARGIN - 28,
        MARGIN + 112,
        MARGIN - 20,
        PLAN,
    );
    encode_png(&canvas)
}

fn put(canvas: &mut [u8], x: u32, y: u32, color: [u8; 4]) {
    if x >= PNG_W || y >= PNG_H {
        return;
    }
    let idx = ((y * PNG_W + x) * 4) as usize;
    canvas[idx..idx + 4].copy_from_slice(&color);
}

fn draw_hline(canvas: &mut [u8], y: u32) {
    for x in MARGIN..PNG_W - MARGIN {
        put(canvas, x, y, GRID);
    }
}

fn draw_hline_solid(canvas: &mut [u8], y: u32) {
    for x in MARGIN..PNG_W - MARGIN {
        put(canvas, x, y, FG);
    }
}

fn draw_vline(canvas: &mut [u8], x: u32) {
    for y in MARGIN..PNG_H - MARGIN {
        put(canvas, x, y, FG);
    }
}

fn fill_rect(canvas: &mut [u8], x0: u32, y0: u32, x1: u32, y1: u32, color: [u8; 4]) {
    for y in y0..=y1 {
        for x in x0..=x1 {
            put(canvas, x, y, color);
        }
    }
}

fn draw_dot(canvas: &mut [u8], p: (f64, f64), color: [u8; 4]) {
    let (cx, cy) = (p.0.round() as i64, p.1.round() as i64);
    for dy in -2i64..=2 {
        for dx in -2i64..=2 {
            if dx * dx + dy * dy <= 4 {
                put(
                    canvas,
                    (cx + dx).clamp(0, PNG_W as i64 - 1) as u32,
                    (cy + dy).clamp(0, PNG_H as i64 - 1) as u32,
                    color,
                );
            }
        }
    }
}

fn draw_line(canvas: &mut [u8], a: (f64, f64), b: (f64, f64), color: [u8; 4]) {
    let steps = ((b.0 - a.0).abs().max((b.1 - a.1).abs()).ceil() as usize).max(1);
    for s in 0..=steps {
        let t = s as f64 / steps as f64;
        let x = a.0 + (b.0 - a.0) * t;
        let y = a.1 + (b.1 - a.1) * t;
        put(canvas, x.round() as u32, y.round() as u32, color);
    }
}

fn encode_png(rgba: &[u8]) -> ExportResult<Vec<u8>> {
    let mut buffer = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut buffer, PNG_W, PNG_H);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| ExportError(format!("PNG 头失败: {e}")))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| ExportError(format!("PNG 数据失败: {e}")))?;
    }
    Ok(buffer)
}
