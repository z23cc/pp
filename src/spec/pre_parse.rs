//! Pre-deserialization tolerance rules for real-world OpenAPI documents.
//!
//! This module keeps lossy/string-level repairs separate from typed OpenAPI
//! normalization so each stage can report what changed before parsing.

use anyhow::Result;
use regex::Regex;

use super::normalization_rules::{self as rules, pre_parse};
use super::report::ReportEntry;

type DowngradeReport = Option<(String, usize)>;

pub(super) fn normalize_yaml(raw: &str) -> Result<(Option<String>, Vec<ReportEntry>)> {
    let (mut owned, downgraded) = downgrade_openapi_31(raw)?;
    let mut current = owned.as_deref().unwrap_or(raw).to_string();
    let mut changed = owned.is_some();
    let mut reports = Vec::new();

    if let Some((version, transforms)) = downgraded {
        reports.push(rules::pre_parse_warning(
            pre_parse::OPENAPI_31_DOWNGRADED,
            format!("downgraded OpenAPI {version} → 3.0.3 for parsing ({transforms} transforms applied)"),
            None,
        ));
    }

    let (clamped, clamp_count) = clamp_numeric_bounds(&current)?;
    if clamp_count > 0 {
        current = clamped;
        changed = true;
        reports.push(rules::pre_parse_warning(
            pre_parse::NUMERIC_BOUNDS_CLAMPED,
            format!("clamped {clamp_count} out-of-range numeric bounds"),
            None,
        ));
    }

    let (normalized_tags, tag_count) = normalize_top_level_tag_descriptions(&current);
    if tag_count > 0 {
        current = normalized_tags;
        changed = true;
        reports.push(rules::pre_parse_warning(
            pre_parse::TAG_DESCRIPTIONS_REPLACED,
            format!("replaced {tag_count} non-string top-level tag descriptions"),
            None,
        ));
    }

    let (inlined_refs, ref_count) = replace_ref_only_operations(&current)?;
    if ref_count > 0 {
        current = inlined_refs;
        changed = true;
        reports.push(rules::pre_parse_warning(
            pre_parse::REF_ONLY_OPERATIONS_REPLACED,
            format!("replaced {ref_count} ref-only operations with parseable placeholders"),
            None,
        ));
    }

    if changed {
        owned = Some(current);
    }

    Ok((owned, reports))
}

fn downgrade_openapi_31(raw: &str) -> Result<(Option<String>, DowngradeReport)> {
    let Some(version) = detect_openapi_31(raw) else {
        return Ok((None, None));
    };

    let mut transforms = 0;
    let mut out = raw.to_string();

    out = replace_count(
        &out,
        &Regex::new(r#"(?m)^(\s*openapi:\s*)['\"]?3\.1(?:\.\d+)?['\"]?\s*$"#)?,
        "${1}3.0.3",
        &mut transforms,
    );
    out = replace_count(
        &out,
        &Regex::new(r#"\"openapi\"\s*:\s*\"3\.1(?:\.\d+)?\""#)?,
        r#""openapi":"3.0.3""#,
        &mut transforms,
    );
    out = replace_count(
        &out,
        &Regex::new(
            r#"(?m)^(\s*)type:\s*\[\s*['\"]?(string|integer|number|boolean|array|object)['\"]?\s*,\s*['\"]?null['\"]?\s*\]\s*$"#,
        )?,
        "${1}type: $2\n${1}nullable: true",
        &mut transforms,
    );
    out = replace_count(
        &out,
        &Regex::new(
            r#"(?m)^(\s*)type:\s*\[\s*['\"]?null['\"]?\s*,\s*['\"]?(string|integer|number|boolean|array|object)['\"]?\s*\]\s*$"#,
        )?,
        "${1}type: $2\n${1}nullable: true",
        &mut transforms,
    );
    out = replace_count(
        &out,
        &Regex::new(
            r#"(?m)^(\s*)type:\s*\n\s*-\s*['\"]?(string|integer|number|boolean|array|object)['\"]?\s*\n\s*-\s*['\"]?null['\"]?\s*$"#,
        )?,
        "${1}type: $2\n${1}nullable: true",
        &mut transforms,
    );
    out = replace_count(
        &out,
        &Regex::new(
            r#"(?m)^(\s*)type:\s*\n\s*-\s*['\"]?null['\"]?\s*\n\s*-\s*['\"]?(string|integer|number|boolean|array|object)['\"]?\s*$"#,
        )?,
        "${1}type: $2\n${1}nullable: true",
        &mut transforms,
    );
    out = replace_count(
        &out,
        &Regex::new(r#"\"exclusiveMinimum\"\s*:\s*(-?\d+(?:\.\d+)?)"#)?,
        r#""exclusiveMinimum": true, "minimum": $1"#,
        &mut transforms,
    );
    out = replace_count(
        &out,
        &Regex::new(r#"\"exclusiveMaximum\"\s*:\s*(-?\d+(?:\.\d+)?)"#)?,
        r#""exclusiveMaximum": true, "maximum": $1"#,
        &mut transforms,
    );
    out = strip_top_level_block(&out, "webhooks", &mut transforms);
    out = strip_top_level_block(&out, "$defs", &mut transforms);

    Ok((Some(out), Some((version, transforms))))
}

fn replace_count(input: &str, re: &Regex, replacement: &str, transforms: &mut usize) -> String {
    let count = re.find_iter(input).count();
    if count > 0 {
        *transforms += count;
        re.replace_all(input, replacement).into_owned()
    } else {
        input.to_string()
    }
}

fn strip_top_level_block(input: &str, key: &str, transforms: &mut usize) -> String {
    let mut out = Vec::new();
    let mut skipping = false;
    let header = format!("{key}:");

    for line in input.lines() {
        let is_top_level = !line.starts_with(char::is_whitespace);
        if is_top_level && line.trim_end() == header {
            skipping = true;
            *transforms += 1;
            continue;
        }
        if skipping && is_top_level && !line.trim().is_empty() {
            skipping = false;
        }
        if !skipping {
            out.push(line);
        }
    }

    join_lines(&out, input.ends_with('\n'))
}

pub(super) fn clamp_numeric_bounds(input: &str) -> Result<(String, usize)> {
    let yaml_re = Regex::new(
        r#"(?m)^(\s*(?:minimum|maximum|exclusiveMinimum|exclusiveMaximum):\s*)(-?\d+)(\s*)$"#,
    )?;
    let json_re =
        Regex::new(r#"(\"(?:minimum|maximum|exclusiveMinimum|exclusiveMaximum)\"\s*:\s*)(-?\d+)"#)?;
    let mut count = 0;

    let out = yaml_re
        .replace_all(input, |caps: &regex::Captures<'_>| {
            clamp_replacement(
                &caps[1],
                &caps[2],
                caps.get(3).map_or("", |m| m.as_str()),
                &mut count,
            )
        })
        .into_owned();
    let out = json_re
        .replace_all(&out, |caps: &regex::Captures<'_>| {
            clamp_replacement(&caps[1], &caps[2], "", &mut count)
        })
        .into_owned();

    Ok((out, count))
}

fn clamp_replacement(prefix: &str, literal: &str, suffix: &str, count: &mut usize) -> String {
    if let Some(clamped) = clamped_i64_literal(literal) {
        *count += 1;
        format!("{prefix}{clamped}{suffix}")
    } else {
        format!("{prefix}{literal}{suffix}")
    }
}

fn clamped_i64_literal(literal: &str) -> Option<&'static str> {
    const I64_MAX: &str = "9223372036854775807";
    const I64_MIN_ABS: &str = "9223372036854775808";

    if let Some(rest) = literal.strip_prefix('-') {
        integer_exceeds(rest, I64_MIN_ABS).then_some("-9223372036854775808")
    } else {
        integer_exceeds(literal, I64_MAX).then_some("9223372036854775807")
    }
}

fn integer_exceeds(value: &str, max: &str) -> bool {
    let normalized = value.trim_start_matches('0');
    let normalized = if normalized.is_empty() {
        "0"
    } else {
        normalized
    };
    normalized.len() > max.len() || (normalized.len() == max.len() && normalized > max)
}

pub(super) fn normalize_top_level_tag_descriptions(input: &str) -> (String, usize) {
    let lines: Vec<&str> = input.lines().collect();
    let mut out = Vec::with_capacity(lines.len());
    let mut i = 0;
    let mut in_tags = false;
    let mut count = 0;

    while i < lines.len() {
        let line = lines[i];
        if !line.starts_with(char::is_whitespace) && !line.trim().is_empty() {
            in_tags = line.trim_end() == "tags:";
        }

        if in_tags && line.trim_end() == "    description:" && child_map_follows(&lines, i + 1) {
            out.push("    description: \"\"");
            count += 1;
            i += 1;
            while i < lines.len() && is_description_child(lines[i]) {
                i += 1;
            }
            continue;
        }

        out.push(line);
        i += 1;
    }

    (join_lines(&out, input.ends_with('\n')), count)
}

fn child_map_follows(lines: &[&str], start: usize) -> bool {
    for line in &lines[start..] {
        if line.trim().is_empty() {
            continue;
        }
        return is_description_child(line);
    }
    false
}

fn is_description_child(line: &str) -> bool {
    (line.starts_with("      ") && !line.starts_with("    - ")) || line.trim().is_empty()
}

pub(super) fn replace_ref_only_operations(input: &str) -> Result<(String, usize)> {
    let method_re = Regex::new(r#"^    (get|put|post|delete|patch|options|head|trace):\s*$"#)?;
    let ref_re = Regex::new(r#"^      \$ref:\s*['\"]([^'\"]+)['\"]\s*$"#)?;
    let path_re = Regex::new(r#"^  ([^\s].*):\s*$"#)?;
    let path_param_re = Regex::new(r#"\{([^}/]+)\}"#)?;
    let lines: Vec<&str> = input.lines().collect();
    let mut out = Vec::with_capacity(lines.len());
    let mut current_path = String::new();
    let mut count = 0;
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        if let Some(caps) = path_re.captures(line) {
            current_path = caps[1].trim_matches('"').to_string();
        }

        if let Some(method_caps) = method_re.captures(line) {
            if let Some(next) = lines.get(i + 1) {
                if let Some(ref_caps) = ref_re.captures(next) {
                    count += 1;
                    let method = &method_caps[1];
                    let operation_id = operation_id_from_ref(method, &ref_caps[1]);
                    out.push(format!("    {method}:"));
                    out.push(format!("      operationId: {operation_id}"));
                    let path_params: Vec<String> = path_param_re
                        .captures_iter(&current_path)
                        .map(|caps| caps[1].to_string())
                        .collect();
                    if !path_params.is_empty() {
                        out.push("      parameters:".to_string());
                        for param in path_params {
                            out.push("        - in: path".to_string());
                            out.push(format!("          name: {param}"));
                            out.push("          required: true".to_string());
                            out.push("          schema:".to_string());
                            out.push("            type: string".to_string());
                        }
                    }
                    out.push("      responses:".to_string());
                    out.push("        '200':".to_string());
                    out.push("          description: ok".to_string());
                    i += 2;
                    continue;
                }
            }
        }

        out.push(line.to_string());
        i += 1;
    }

    let out_refs: Vec<&str> = out.iter().map(String::as_str).collect();
    Ok((join_lines(&out_refs, input.ends_with('\n')), count))
}

fn operation_id_from_ref(method: &str, reference: &str) -> String {
    let mut out = String::from(method);
    for ch in reference.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

fn join_lines(lines: &[&str], trailing_newline: bool) -> String {
    let mut joined = lines.join("\n");
    if trailing_newline {
        joined.push('\n');
    }
    joined
}

pub(super) fn detect_openapi_31(raw: &str) -> Option<String> {
    for line in raw.lines() {
        let line = line.trim_start();
        let Some(value) = line.strip_prefix("openapi:") else {
            continue;
        };
        let version = value.trim().trim_matches(['\'', '"']);
        if version.starts_with("3.1") {
            return Some(version.to_string());
        }
    }

    let compact: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    let after_key = compact
        .split_once("\"openapi\":\"")
        .map(|(_, value)| value)?;
    let version = after_key.split('"').next().unwrap_or_default();
    version.starts_with("3.1").then(|| version.to_string())
}
