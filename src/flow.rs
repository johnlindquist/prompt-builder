use std::path::Path;
use std::process::Command;

use anyhow::Context;
use serde::Deserialize;

/// Protocol version this client understands (mdflow's FLOW_UX_PROTOCOL_VERSION).
const SUPPORTED_PROTOCOL_VERSION: u64 = 1;

// External JSON from mdflow: every struct tolerates unknown fields so additive
// mdflow changes never break the TUI.

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowCatalog {
    #[serde(default)]
    pub protocol_version: Option<u64>,
    #[serde(default)]
    pub flows: Vec<FlowEntry>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct FlowEntry {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplainOutput {
    #[serde(default)]
    pub protocol_version: Option<u64>,
    #[serde(default)]
    pub inputs: Vec<ExplainInput>,
    /// All template vars in the body. Absent on older mdflow versions.
    #[serde(default)]
    pub template_vars: Vec<String>,
    /// Vars still unfilled after defaults. Absent on older mdflow versions;
    /// fall back to scanning `prompt` for [MISSING: …] markers.
    #[serde(default)]
    pub missing_template_vars: Vec<String>,
    #[serde(default)]
    pub prompt: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ExplainInput {
    /// Var name including the leading underscore, e.g. "_severity".
    pub name: String,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    /// Description/help text from the flow's _inputs definition.
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub default: Option<serde_json::Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Number,
    Select,
    Confirm,
}

impl FieldKind {
    fn parse(kind: Option<&str>) -> Self {
        match kind.unwrap_or("text") {
            "select" => Self::Select,
            "number" => Self::Number,
            "confirm" => Self::Confirm,
            _ => Self::Text,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldSpec {
    /// Var name without the leading underscore; launched as --_name=value.
    pub name: String,
    /// Human label: the input's message/description, else the var name.
    pub label: String,
    pub kind: FieldKind,
    pub options: Vec<String>,
    /// True when the flow cannot render without a value (no default).
    pub required: bool,
    pub default: Option<String>,
}

pub fn fetch_catalog(mdflow_bin: &str, cwd: &Path) -> anyhow::Result<FlowCatalog> {
    let output = run_json_command(mdflow_bin, &["catalog", "--json"], cwd)?;
    let catalog: FlowCatalog =
        serde_json::from_slice(&output).context("failed to parse mdflow catalog JSON")?;
    check_protocol(catalog.protocol_version)?;
    Ok(catalog)
}

pub fn explain_flow(
    mdflow_bin: &str,
    flow_path: &str,
    cwd: &Path,
) -> anyhow::Result<ExplainOutput> {
    let output = run_json_command(mdflow_bin, &["explain", flow_path, "--json"], cwd)?;
    let explain: ExplainOutput =
        serde_json::from_slice(&output).context("failed to parse mdflow explain JSON")?;
    check_protocol(explain.protocol_version)?;
    Ok(explain)
}

fn run_json_command(mdflow_bin: &str, args: &[&str], cwd: &Path) -> anyhow::Result<Vec<u8>> {
    let output = Command::new(mdflow_bin)
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => {
                anyhow::anyhow!("mdflow not found on PATH (bin: {mdflow_bin})")
            }
            _ => anyhow::Error::new(err).context(format!("failed to run {mdflow_bin}")),
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("no error output");
        anyhow::bail!(
            "{mdflow_bin} {} failed ({}): {detail}",
            args.join(" "),
            output.status
        );
    }
    Ok(output.stdout)
}

fn check_protocol(version: Option<u64>) -> anyhow::Result<()> {
    if let Some(version) = version {
        if version != SUPPORTED_PROTOCOL_VERSION {
            anyhow::bail!(
                "mdflow protocol v{version} is not supported (expected v{SUPPORTED_PROTOCOL_VERSION}); update prompt-builder"
            );
        }
    }
    Ok(())
}

/// Var names extracted from `[MISSING: name]` markers, leading `_` intact.
pub fn missing_var_names(prompt: &str) -> Vec<String> {
    const MARKER: &str = "[MISSING: ";
    let mut names = Vec::new();
    let mut rest = prompt;
    while let Some(start) = rest.find(MARKER) {
        rest = &rest[start + MARKER.len()..];
        let Some(end) = rest.find(']') else { break };
        let name = rest[..end].trim();
        if !name.is_empty() && !names.iter().any(|existing| existing == name) {
            names.push(name.to_string());
        }
        rest = &rest[end + 1..];
    }
    names
}

fn strip_var(name: &str) -> &str {
    name.strip_prefix('_').unwrap_or(name)
}

/// True for vars the composed prompt itself fills (first positional).
fn is_prompt_var(bare_name: &str) -> bool {
    bare_name == "1" || bare_name == "prompt"
}

/// Form fields for a flow plus whether the flow consumes the composed prompt.
///
/// Fields = declared `_inputs` (declaration order) plus any still-missing
/// template vars not covered by an input. `_1`/`_prompt` are excluded — the
/// composed prompt fills them — and drive `prompt_capable`.
pub fn extract_fields(explain: &ExplainOutput) -> (Vec<FieldSpec>, bool) {
    let missing: Vec<String> = if explain.missing_template_vars.is_empty() {
        explain
            .prompt
            .as_deref()
            .map(missing_var_names)
            .unwrap_or_default()
    } else {
        explain.missing_template_vars.clone()
    };
    let missing_bare: Vec<&str> = missing.iter().map(|name| strip_var(name)).collect();

    let prompt_capable = missing_bare.iter().copied().any(is_prompt_var)
        || explain
            .template_vars
            .iter()
            .any(|name| is_prompt_var(strip_var(name)));

    let mut fields = Vec::new();
    for input in &explain.inputs {
        let name = strip_var(&input.name).to_string();
        if is_prompt_var(&name) || fields.iter().any(|field: &FieldSpec| field.name == name) {
            continue;
        }
        let default = input.default.as_ref().and_then(json_value_text);
        fields.push(FieldSpec {
            label: input
                .message
                .clone()
                .filter(|message| !message.trim().is_empty())
                .unwrap_or_else(|| name.clone()),
            kind: FieldKind::parse(input.kind.as_deref()),
            options: input.options.clone(),
            required: default.is_none() && missing_bare.contains(&name.as_str()),
            default,
            name,
        });
    }
    for bare in &missing_bare {
        if is_prompt_var(bare) || fields.iter().any(|field| field.name == *bare) {
            continue;
        }
        fields.push(FieldSpec {
            name: (*bare).to_string(),
            label: (*bare).to_string(),
            kind: FieldKind::Text,
            options: Vec::new(),
            required: true,
            default: None,
        });
    }

    (fields, prompt_capable)
}

fn json_value_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Bool(flag) => Some(flag.to_string()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        other => Some(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_tolerates_unknown_fields_and_missing_optionals() {
        let json = r#"{
            "type": "mdflow.catalog",
            "protocolVersion": 1,
            "cwd": "/x",
            "projectRoot": "/x",
            "counts": {"project": 1},
            "flows": [
                {"name": "review.md", "path": "/x/flows/review.md",
                 "provenance": "PROJECT", "frecency": 3.5, "extraField": true},
                {"id": "global:notes", "name": "notes.md", "path": "/g/notes.md",
                 "description": "Take notes", "engine": "claude", "scope": "global"}
            ]
        }"#;

        let catalog: FlowCatalog = serde_json::from_str(json).expect("parse");

        assert_eq!(catalog.protocol_version, Some(1));
        assert_eq!(catalog.flows.len(), 2);
        assert_eq!(catalog.flows[0].name, "review.md");
        assert_eq!(catalog.flows[0].description, None);
        assert_eq!(catalog.flows[1].engine.as_deref(), Some("claude"));
    }

    #[test]
    fn unsupported_protocol_version_is_rejected() {
        let err = check_protocol(Some(2)).expect_err("v2 should fail");

        assert!(err.to_string().contains("protocol v2"));
        assert!(check_protocol(None).is_ok());
        assert!(check_protocol(Some(1)).is_ok());
    }

    #[test]
    fn missing_markers_are_scanned_and_deduped() {
        let names =
            missing_var_names("Review [MISSING: _focus] and [MISSING: _1]; also [MISSING: _focus]");

        assert_eq!(names, vec!["_focus".to_string(), "_1".to_string()]);
    }

    // Shape captured from a real `mdflow explain --json` run: inputs keep the
    // `_` name prefix, defaults are already substituted into `prompt`, and
    // only default-less vars appear as [MISSING: …].
    fn real_explain() -> ExplainOutput {
        serde_json::from_str(
            r#"{
                "protocolVersion": 1,
                "prompt": "Review [MISSING: _focus] at high severity, max 10 files, deep=false.\n\nTask: [MISSING: _1]",
                "inputs": [
                    {"name": "_severity", "type": "select", "message": null,
                     "default": "high", "options": ["low", "high"]},
                    {"name": "_max_files", "type": "number", "message": null, "default": 10},
                    {"name": "_focus", "type": "text", "message": "Area to focus on", "default": null},
                    {"name": "_deep", "type": "confirm", "message": null, "default": false}
                ]
            }"#,
        )
        .expect("parse")
    }

    #[test]
    fn fields_come_from_inputs_with_defaults_prefilled() {
        let (fields, prompt_capable) = extract_fields(&real_explain());

        assert!(prompt_capable, "flow references _1");
        assert_eq!(
            fields.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
            vec!["severity", "max_files", "focus", "deep"]
        );
        let severity = &fields[0];
        assert_eq!(severity.kind, FieldKind::Select);
        assert_eq!(severity.options, vec!["low", "high"]);
        assert_eq!(severity.default.as_deref(), Some("high"));
        assert!(!severity.required);
        let focus = &fields[2];
        assert_eq!(focus.kind, FieldKind::Text);
        assert_eq!(focus.label, "Area to focus on");
        assert!(focus.required);
        let deep = &fields[3];
        assert_eq!(deep.kind, FieldKind::Confirm);
        assert_eq!(deep.default.as_deref(), Some("false"));
    }

    #[test]
    fn bare_template_vars_become_required_text_fields() {
        let explain: ExplainOutput = serde_json::from_str(
            r#"{"prompt": "Translate [MISSING: _1] to [MISSING: _2].", "inputs": []}"#,
        )
        .expect("parse");

        let (fields, prompt_capable) = extract_fields(&explain);

        assert!(prompt_capable);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "2");
        assert_eq!(fields[0].kind, FieldKind::Text);
        assert!(fields[0].required);
    }

    #[test]
    fn flow_without_prompt_var_is_flagged() {
        let explain: ExplainOutput =
            serde_json::from_str(r#"{"prompt": "Say hello.", "inputs": []}"#).expect("parse");

        let (fields, prompt_capable) = extract_fields(&explain);

        assert!(fields.is_empty());
        assert!(!prompt_capable, "flow ignores the composed prompt");
    }

    #[test]
    fn explicit_missing_template_vars_take_priority_over_markers() {
        let explain: ExplainOutput = serde_json::from_str(
            r#"{
                "prompt": "irrelevant [MISSING: _stale]",
                "templateVars": ["_prompt", "_focus"],
                "missingTemplateVars": ["_focus"],
                "inputs": []
            }"#,
        )
        .expect("parse");

        let (fields, prompt_capable) = extract_fields(&explain);

        assert!(prompt_capable, "templateVars mentions _prompt");
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "focus");
    }

    #[test]
    fn unknown_input_type_degrades_to_text() {
        let explain: ExplainOutput = serde_json::from_str(
            r#"{"prompt": "x [MISSING: _thing]",
                "inputs": [{"name": "_thing", "type": "slider", "default": null}]}"#,
        )
        .expect("parse");

        let (fields, _) = extract_fields(&explain);

        assert_eq!(fields[0].kind, FieldKind::Text);
        assert!(fields[0].required);
    }
}
