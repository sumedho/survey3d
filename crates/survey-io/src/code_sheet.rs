//! Feature code sheets as CSV:
//! `code,description,layer,colour,linework,symbol,size,colour_by,line_style,weight`.
//!
//! Colours are `#rrggbb`; linework is `yes`/`no` (also `line`/`point`,
//! `true`/`false`, `1`/`0`). The style columns are optional (older five-column
//! sheets still load): `symbol` and `line_style` are keys such as `circle_cross`
//! or `dash_dot`, `colour_by` is `layer` or `custom`, sizes are screen points.
//! A first row starting with `code` is a header.

use survey_core::field_to_finish::FeatureCode;
use survey_core::style::{CodeStyle, LineStyle, Symbol};

use crate::IoError;

pub fn read_code_sheet(bytes: &[u8]) -> Result<(Vec<FeatureCode>, Vec<String>), IoError> {
    let text = std::str::from_utf8(bytes).map_err(|_| IoError::NotText)?;
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(text.as_bytes());
    let mut codes = Vec::new();
    let mut warnings = Vec::new();
    for (i, record) in reader.records().enumerate() {
        let line = i + 1;
        let record = match record {
            Ok(r) => r,
            Err(e) => {
                warnings.push(format!("line {line}: {e}"));
                continue;
            }
        };
        let field = |n: usize| record.get(n).unwrap_or("").to_string();
        let code = field(0);
        if code.is_empty() || (i == 0 && code.eq_ignore_ascii_case("code")) {
            continue;
        }
        let color = match parse_color(&field(3)) {
            Some(c) => c,
            None => {
                if !field(3).is_empty() {
                    warnings.push(format!("line {line}: colour '{}' is not #rrggbb", field(3)));
                }
                [200, 200, 200]
            }
        };
        let linework = match field(4).to_ascii_lowercase().as_str() {
            "" | "no" | "n" | "false" | "0" | "point" => false,
            "yes" | "y" | "true" | "1" | "line" => true,
            other => {
                warnings.push(format!("line {line}: linework '{other}' should be yes or no"));
                false
            }
        };
        let layer = match field(2) {
            l if l.is_empty() => code.clone(),
            l => l,
        };
        let mut style = CodeStyle::default();
        if !field(5).is_empty() {
            match Symbol::from_key(&field(5)) {
                Some(sym) => style.symbol = sym,
                None => warnings.push(format!("line {line}: unknown symbol '{}'", field(5))),
            }
        }
        if let Ok(v) = field(6).parse::<f32>()
            && v > 0.0
        {
            style.symbol_size = v;
        }
        match field(7).to_ascii_lowercase().as_str() {
            "" | "layer" => {}
            "custom" | "code" => style.by_layer = false,
            other => warnings.push(format!(
                "line {line}: colour_by '{other}' should be layer or custom"
            )),
        }
        if !field(8).is_empty() {
            match LineStyle::from_key(&field(8)) {
                Some(l) => style.line_style = l,
                None => warnings.push(format!("line {line}: unknown line style '{}'", field(8))),
            }
        }
        if let Ok(v) = field(9).parse::<f32>()
            && v > 0.0
        {
            style.line_weight = v;
        }
        codes.push(FeatureCode {
            code,
            description: field(1),
            layer,
            color,
            linework,
            style,
        });
    }
    if codes.is_empty() {
        return Err(IoError::NoPoints(" — no feature codes found".into()));
    }
    Ok((codes, warnings))
}

pub fn write_code_sheet(codes: &[FeatureCode]) -> String {
    let mut writer = csv::Writer::from_writer(Vec::new());
    let _ = writer.write_record([
        "code",
        "description",
        "layer",
        "colour",
        "linework",
        "symbol",
        "size",
        "colour_by",
        "line_style",
        "weight",
    ]);
    for c in codes {
        let [r, g, b] = c.color;
        let _ = writer.write_record([
            c.code.as_str(),
            c.description.as_str(),
            c.layer.as_str(),
            &format!("#{r:02x}{g:02x}{b:02x}"),
            if c.linework { "yes" } else { "no" },
            c.style.symbol.key(),
            &c.style.symbol_size.to_string(),
            if c.style.by_layer { "layer" } else { "custom" },
            c.style.line_style.key(),
            &c.style.line_weight.to_string(),
        ]);
    }
    String::from_utf8(writer.into_inner().unwrap_or_default()).unwrap_or_default()
}

fn parse_color(text: &str) -> Option<[u8; 3]> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(hex, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

#[cfg(test)]
mod tests {
    use super::*;
    use survey_core::field_to_finish::FeatureLibrary;

    #[test]
    fn round_trip() {
        let lib = FeatureLibrary::example();
        let text = write_code_sheet(&lib.codes);
        let (codes, warnings) = read_code_sheet(text.as_bytes()).expect("sheet");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(codes, lib.codes);
    }

    #[test]
    fn lenient_rows() {
        let (codes, warnings) =
            read_code_sheet(b"KB,\"Kerb, top\",,#zz0000,line\nTR,Tree,VEG,#22c55e,maybe\n").expect("sheet");
        assert_eq!(codes[0].description, "Kerb, top");
        assert_eq!(codes[0].layer, "KB");
        assert!(codes[0].linework && !codes[1].linework);
        assert_eq!(codes[1].color, [0x22, 0xc5, 0x5e]);
        assert_eq!(warnings.len(), 2);
    }
}
