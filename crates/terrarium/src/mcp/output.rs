use serde::Deserialize;

const MAX_DIAGNOSTICS: usize = 50;
const MAX_MSG_LEN: usize = 200;

#[derive(Debug)]
pub struct Diagnostic {
    pub level: String,
    pub message: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub code: Option<String>,
}

#[derive(Deserialize)]
struct CargoMessage {
    reason: String,
    message: Option<CompilerMessage>,
}

#[derive(Deserialize)]
struct CompilerMessage {
    level: String,
    message: String,
    spans: Vec<Span>,
    code: Option<DiagCode>,
}

#[derive(Deserialize)]
struct Span {
    file_name: String,
    line_start: u32,
}

#[derive(Deserialize)]
struct DiagCode {
    code: String,
}

/// Parses cargo's `--message-format=json` NDJSON output, returning up to 50 diagnostics.
pub fn parse_diagnostics(ndjson: &[u8]) -> Vec<Diagnostic> {
    let text = String::from_utf8_lossy(ndjson);
    let mut out = Vec::new();

    for line in text.lines() {
        if out.len() >= MAX_DIAGNOSTICS {
            break;
        }
        let Ok(msg) = serde_json::from_str::<CargoMessage>(line) else {
            continue;
        };
        if msg.reason != "compiler-message" {
            continue;
        }
        let Some(cm) = msg.message else { continue };

        let message = if cm.message.len() > MAX_MSG_LEN {
            cm.message[..MAX_MSG_LEN].to_string()
        } else {
            cm.message
        };

        let (file, line) = cm
            .spans
            .first()
            .map(|s| (Some(s.file_name.clone()), Some(s.line_start)))
            .unwrap_or((None, None));

        out.push(Diagnostic {
            level: cm.level,
            message,
            file,
            line,
            code: cm.code.map(|c| c.code),
        });
    }

    out
}

/// Parse `cargo fmt --check` stdout/stderr for files that need formatting.
/// Lines starting with "Diff in " contain the file path.
pub fn parse_fmt_files(output: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(output);
    text.lines()
        .filter_map(|l| l.strip_prefix("Diff in "))
        .filter_map(|rest| rest.split(" at line").next())
        .map(|s| s.trim().to_string())
        .collect()
}
