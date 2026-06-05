use std::io::{Cursor, Write};

use anyhow::{Context, Result};

use crate::{
    db::{Batch, BatchOutcome, ControlEvent, SensorSampleRecord},
    number::round2,
};

pub(crate) fn build_audit_csv(events: &[ControlEvent]) -> String {
    let mut csv = String::from(
        "id,batch_id,event_type,target_temperature_c,target_stirrer_rpm,target_shake_speed_cpm,reason,created_at,previous_hash,event_hash\n",
    );
    for event in events {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{}\n",
            event.id,
            event
                .batch_id
                .map(|value| value.to_string())
                .unwrap_or_default(),
            csv_escape(&event.event_type),
            event
                .target_temperature_c
                .map(|value| value.to_string())
                .unwrap_or_default(),
            event
                .target_stirrer_rpm
                .map(|value| value.to_string())
                .unwrap_or_default(),
            event
                .target_shake_speed_cpm
                .map(|value| value.to_string())
                .unwrap_or_default(),
            csv_escape(&event.reason),
            event.created_at.to_rfc3339(),
            event.previous_hash.as_deref().unwrap_or_default(),
            event.event_hash.as_deref().unwrap_or_default()
        ));
    }
    csv
}

pub(crate) fn build_batches_csv(batches: &[Batch], outcomes: &[BatchOutcome]) -> String {
    let mut csv = String::from(
        "id,process_id,name,started_at,finished_at,target_temperature_c,target_stirrer_rpm,heating_minutes,stirring_minutes,yield_percent,product_ratio\n",
    );
    for batch in batches {
        let outcome = outcomes.iter().find(|outcome| outcome.batch_id == batch.id);
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{}\n",
            batch.id,
            batch
                .process_id
                .map(|value| value.to_string())
                .unwrap_or_default(),
            csv_escape(&batch.name),
            batch.started_at.to_rfc3339(),
            batch
                .finished_at
                .map(|value| value.to_rfc3339())
                .unwrap_or_default(),
            batch.target_temperature_c,
            batch.target_stirrer_rpm,
            batch.heating_minutes,
            batch.stirring_minutes,
            outcome
                .map(|value| value.yield_percent.to_string())
                .unwrap_or_default(),
            outcome
                .map(|value| value.product_ratio.to_string())
                .unwrap_or_default(),
        ));
    }
    csv
}

pub(crate) fn build_batches_xlsx(batches: &[Batch], outcomes: &[BatchOutcome]) -> Result<Vec<u8>> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    xlsx_zip_add(
        &mut zip,
        options,
        "[Content_Types].xml",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet3.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
</Types>"#
            .as_slice(),
    )?;
    xlsx_zip_add(
        &mut zip,
        options,
        "_rels/.rels",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#
            .as_slice(),
    )?;
    xlsx_zip_add(
        &mut zip,
        options,
        "xl/workbook.xml",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Batches" sheetId="1" r:id="rId1"/>
    <sheet name="Results" sheetId="2" r:id="rId2"/>
    <sheet name="Summary" sheetId="3" r:id="rId3"/>
  </sheets>
</workbook>"#
            .as_slice(),
    )?;
    xlsx_zip_add(
        &mut zip,
        options,
        "xl/_rels/workbook.xml.rels",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet3.xml"/>
  <Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>"#
            .as_slice(),
    )?;
    xlsx_zip_add(
        &mut zip,
        options,
        "xl/styles.xml",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts>
  <fills count="1"><fill><patternFill patternType="none"/></fill></fills>
  <borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
  <cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
  <cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs>
  <cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>
</styleSheet>"#
            .as_slice(),
    )?;

    let batch_sheet = worksheet_xml(&batch_rows(batches, outcomes));
    xlsx_zip_add(
        &mut zip,
        options,
        "xl/worksheets/sheet1.xml",
        batch_sheet.as_bytes(),
    )?;
    let result_sheet = worksheet_xml(&result_rows(outcomes));
    xlsx_zip_add(
        &mut zip,
        options,
        "xl/worksheets/sheet2.xml",
        result_sheet.as_bytes(),
    )?;
    let summary_sheet = worksheet_xml(&summary_rows(batches, outcomes));
    xlsx_zip_add(
        &mut zip,
        options,
        "xl/worksheets/sheet3.xml",
        summary_sheet.as_bytes(),
    )?;
    Ok(zip
        .finish()
        .context("failed to finish batches xlsx package")?
        .into_inner())
}

fn xlsx_zip_add<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    options: zip::write::SimpleFileOptions,
    name: &str,
    data: &[u8],
) -> Result<()> {
    zip.start_file(name, options)
        .with_context(|| format!("failed to start xlsx package entry {name}"))?;
    zip.write_all(data)
        .with_context(|| format!("failed to write xlsx package entry {name}"))?;
    Ok(())
}

fn batch_rows(batches: &[Batch], outcomes: &[BatchOutcome]) -> Vec<Vec<XlsxCell>> {
    let mut rows = vec![xlsx_row([
        "id",
        "process_id",
        "name",
        "started_at",
        "finished_at",
        "target_temperature_c",
        "target_stirrer_rpm",
        "heating_minutes",
        "stirring_minutes",
        "yield_percent",
        "product_ratio",
    ])];
    for batch in batches {
        let outcome = outcomes.iter().find(|outcome| outcome.batch_id == batch.id);
        rows.push(vec![
            XlsxCell::Number(batch.id as f64),
            optional_number(batch.process_id.map(|value| value as f64)),
            XlsxCell::Text(batch.name.clone()),
            XlsxCell::Text(batch.started_at.to_rfc3339()),
            optional_text(batch.finished_at.map(|value| value.to_rfc3339())),
            XlsxCell::Number(batch.target_temperature_c),
            XlsxCell::Number(batch.target_stirrer_rpm),
            XlsxCell::Number(batch.heating_minutes),
            XlsxCell::Number(batch.stirring_minutes),
            optional_number(outcome.map(|value| value.yield_percent)),
            optional_number(outcome.map(|value| value.product_ratio)),
        ]);
    }
    rows
}

fn result_rows(outcomes: &[BatchOutcome]) -> Vec<Vec<XlsxCell>> {
    let mut rows = vec![xlsx_row([
        "batch_id",
        "target_temperature_c",
        "target_stirrer_rpm",
        "heating_minutes",
        "stirring_minutes",
        "yield_percent",
        "product_ratio",
    ])];
    for outcome in outcomes {
        rows.push(vec![
            XlsxCell::Number(outcome.batch_id as f64),
            XlsxCell::Number(outcome.target_temperature_c),
            XlsxCell::Number(outcome.target_stirrer_rpm),
            XlsxCell::Number(outcome.heating_minutes),
            XlsxCell::Number(outcome.stirring_minutes),
            XlsxCell::Number(outcome.yield_percent),
            XlsxCell::Number(outcome.product_ratio),
        ]);
    }
    rows
}

fn summary_rows(batches: &[Batch], outcomes: &[BatchOutcome]) -> Vec<Vec<XlsxCell>> {
    let completed = batches
        .iter()
        .filter(|batch| batch.finished_at.is_some())
        .count();
    let avg_yield = if outcomes.is_empty() {
        None
    } else {
        Some(
            outcomes
                .iter()
                .map(|outcome| outcome.yield_percent)
                .sum::<f64>()
                / outcomes.len() as f64,
        )
    };
    let avg_ratio = if outcomes.is_empty() {
        None
    } else {
        Some(
            outcomes
                .iter()
                .map(|outcome| outcome.product_ratio)
                .sum::<f64>()
                / outcomes.len() as f64,
        )
    };
    vec![
        xlsx_row(["metric", "value"]),
        vec![
            XlsxCell::Text("total_batches".to_string()),
            XlsxCell::Number(batches.len() as f64),
        ],
        vec![
            XlsxCell::Text("completed_batches".to_string()),
            XlsxCell::Number(completed as f64),
        ],
        vec![
            XlsxCell::Text("recorded_results".to_string()),
            XlsxCell::Number(outcomes.len() as f64),
        ],
        vec![
            XlsxCell::Text("average_yield_percent".to_string()),
            optional_number(avg_yield),
        ],
        vec![
            XlsxCell::Text("average_product_ratio".to_string()),
            optional_number(avg_ratio),
        ],
    ]
}

#[derive(Clone)]
enum XlsxCell {
    Text(String),
    Number(f64),
    Blank,
}

fn xlsx_row<const N: usize>(values: [&str; N]) -> Vec<XlsxCell> {
    values
        .into_iter()
        .map(|value| XlsxCell::Text(value.to_string()))
        .collect()
}

fn optional_text(value: Option<String>) -> XlsxCell {
    value.map(XlsxCell::Text).unwrap_or(XlsxCell::Blank)
}

fn optional_number(value: Option<f64>) -> XlsxCell {
    value.map(XlsxCell::Number).unwrap_or(XlsxCell::Blank)
}

fn worksheet_xml(rows: &[Vec<XlsxCell>]) -> String {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
    );
    for (row_index, row) in rows.iter().enumerate() {
        let row_number = row_index + 1;
        xml.push_str(&format!(r#"<row r="{row_number}">"#));
        for (col_index, cell) in row.iter().enumerate() {
            let reference = cell_reference(col_index, row_number);
            match cell {
                XlsxCell::Text(value) => xml.push_str(&format!(
                    r#"<c r="{reference}" t="inlineStr"><is><t>{}</t></is></c>"#,
                    xml_escape(value)
                )),
                XlsxCell::Number(value) if value.is_finite() => {
                    xml.push_str(&format!(r#"<c r="{reference}"><v>{value}</v></c>"#));
                }
                _ => xml.push_str(&format!(r#"<c r="{reference}"/>"#)),
            }
        }
        xml.push_str("</row>");
    }
    xml.push_str("</sheetData></worksheet>");
    xml
}

fn cell_reference(mut col_index: usize, row_number: usize) -> String {
    let mut letters = Vec::new();
    loop {
        letters.push((b'A' + (col_index % 26) as u8) as char);
        col_index /= 26;
        if col_index == 0 {
            break;
        }
        col_index -= 1;
    }
    letters.iter().rev().collect::<String>() + &row_number.to_string()
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub(crate) fn build_batch_report_markdown(
    batch: &Batch,
    outcome: Option<&BatchOutcome>,
    samples: &[SensorSampleRecord],
    events: &[ControlEvent],
) -> String {
    let mut report = String::new();
    report.push_str(&format!("# Experiment Report - Batch {}\n\n", batch.id));
    report.push_str("## Summary\n\n");
    report.push_str(&format!("- Name: {}\n", markdown_escape(&batch.name)));
    report.push_str(&format!(
        "- Process ID: {}\n",
        batch
            .process_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string())
    ));
    report.push_str(&format!("- Started: {}\n", batch.started_at.to_rfc3339()));
    report.push_str(&format!(
        "- Finished: {}\n",
        batch
            .finished_at
            .map(|value| value.to_rfc3339())
            .unwrap_or_else(|| "running or not finished".to_string())
    ));
    report.push_str(&format!(
        "- Target temperature: {:.2} C\n",
        batch.target_temperature_c
    ));
    report.push_str(&format!(
        "- Target stirrer speed: {:.2} RPM\n",
        batch.target_stirrer_rpm
    ));
    report.push_str(&format!(
        "- Heating / stirring: {:.2} min / {:.2} min\n\n",
        batch.heating_minutes, batch.stirring_minutes
    ));

    report.push_str("## Product Result\n\n");
    if let Some(outcome) = outcome {
        report.push_str(&format!("- Yield: {:.2}%\n", outcome.yield_percent));
        report.push_str(&format!(
            "- Product ratio: {:.3}\n\n",
            outcome.product_ratio
        ));
    } else {
        report.push_str("- Product result has not been recorded.\n\n");
    }

    report.push_str("## Sensor Statistics\n\n");
    report.push_str("| Metric | Min | Avg | Max |\n");
    report.push_str("|---|---:|---:|---:|\n");
    for (label, stats) in [
        (
            "Temperature C",
            sample_stats(samples, |sample| sample.sample.temperature_c),
        ),
        (
            "Pressure MPa",
            sample_stats(samples, |sample| sample.sample.pressure_mpa),
        ),
        (
            "Stirrer RPM",
            sample_stats(samples, |sample| sample.sample.stirrer_rpm),
        ),
        (
            "Shake CPM",
            sample_stats(samples, |sample| sample.sample.shake_speed_cpm),
        ),
        (
            "Flow L/min",
            sample_stats(samples, |sample| sample.sample.flow_rate_l_min),
        ),
        (
            "Concentration %",
            sample_stats(samples, |sample| {
                sample.sample.product_concentration_percent
            }),
        ),
        ("pH", sample_stats(samples, |sample| sample.sample.ph)),
    ] {
        report.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            label,
            stat_value(stats.map(|value| value.min)),
            stat_value(stats.map(|value| value.avg)),
            stat_value(stats.map(|value| value.max)),
        ));
    }
    report.push('\n');

    report.push_str("## Audit Events\n\n");
    if events.is_empty() {
        report.push_str("No audit events are linked to this batch.\n");
    } else {
        for event in events.iter().rev().take(50) {
            report.push_str(&format!(
                "- {} [{}] {}\n",
                event.created_at.to_rfc3339(),
                markdown_escape(&event.event_type),
                markdown_escape(&event.reason)
            ));
        }
    }
    report
}

#[derive(Clone, Copy)]
struct NumericStats {
    min: f64,
    avg: f64,
    max: f64,
}

fn sample_stats(
    samples: &[SensorSampleRecord],
    mut value: impl FnMut(&SensorSampleRecord) -> f64,
) -> Option<NumericStats> {
    let mut count = 0.0;
    let mut sum = 0.0;
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for sample in samples {
        let current = value(sample);
        if !current.is_finite() {
            continue;
        }
        count += 1.0;
        sum += current;
        min = min.min(current);
        max = max.max(current);
    }
    if count == 0.0 {
        None
    } else {
        Some(NumericStats {
            min: round2(min),
            avg: round2(sum / count),
            max: round2(max),
        })
    }
}

fn stat_value(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.2}"))
        .unwrap_or_else(|| "--".to_string())
}

fn markdown_escape(value: &str) -> String {
    value.replace('\n', " ").replace('\r', " ")
}

fn csv_escape(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}
