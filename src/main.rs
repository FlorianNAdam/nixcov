use clap::{Parser, Subcommand};
use rnix::ast;
use rowan::ast::AstNode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

const TRACE_PREFIX: &str = "NIXCOV:";

#[derive(Debug, Serialize)]
struct CoverageMap {
    trace_prefix: &'static str,
    expressions: Vec<ExpressionMapping>,
}

#[derive(Debug, Serialize)]
struct ExpressionMapping {
    id: usize,
    file: String,
    byte_start: usize,
    byte_end: usize,
    line_start: usize,
    column_start: usize,
    line_end: usize,
    column_end: usize,
    kind: String,
}

#[derive(Clone, Debug)]
struct CollectedExpression {
    id: usize,
    byte_start: usize,
    byte_end: usize,
    kind: String,
}

#[derive(Debug)]
struct InstrumentedFile {
    source: String,
    mappings: Vec<ExpressionMapping>,
}

#[derive(Debug, Deserialize)]
struct FlakeMetadata {
    path: PathBuf,
}

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Instrument Nix files and write a coverage sidecar JSON file.
    Instrument {
        /// Flake reference to resolve and instrument, for example `.` or `github:owner/repo`.
        flake_ref: String,
        /// Output directory for the instrumented files.
        output_dir: PathBuf,
        /// JSON sidecar path for coverage ID source mappings.
        sidecar: PathBuf,
    },
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        Command::Instrument {
            flake_ref,
            output_dir,
            sidecar,
        } => instrument_flake(&flake_ref, &output_dir, &sidecar),
    }
}

fn instrument_flake(
    flake_ref: &str,
    output_dir: &Path,
    sidecar: &Path,
) -> Result<(), Box<dyn Error>> {
    let source = resolve_flake_source(flake_ref)?;
    instrument_path(&source, output_dir, sidecar)
}

fn resolve_flake_source(flake_ref: &str) -> Result<PathBuf, Box<dyn Error>> {
    let output = ProcessCommand::new("nix")
        .args(["flake", "metadata", "--json", flake_ref])
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "failed to resolve flake {flake_ref:?}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }

    flake_source_path_from_metadata(&output.stdout)
}

fn flake_source_path_from_metadata(metadata: &[u8]) -> Result<PathBuf, Box<dyn Error>> {
    let metadata: FlakeMetadata = serde_json::from_slice(metadata)?;
    Ok(metadata.path)
}

fn instrument_path(input: &Path, output_dir: &Path, sidecar: &Path) -> Result<(), Box<dyn Error>> {
    let files = input_files(input)?;
    let mut next_id = 0;
    let mut all_mappings = Vec::new();

    for file in files {
        let output_file = output_path(input, output_dir, &file)?;
        if let Some(parent) = output_file.parent() {
            fs::create_dir_all(parent)?;
        }

        if file.extension().is_some_and(|extension| extension == "nix") {
            let source = fs::read_to_string(&file)?;
            let instrumented = instrument_source(&file, &source, &mut next_id)?;
            fs::write(output_file, instrumented.source)?;
            all_mappings.extend(instrumented.mappings);
        } else {
            fs::copy(&file, output_file)?;
        }
    }

    if let Some(parent) = sidecar.parent() {
        fs::create_dir_all(parent)?;
    }
    let coverage_map = CoverageMap {
        trace_prefix: TRACE_PREFIX,
        expressions: all_mappings,
    };
    fs::write(sidecar, serde_json::to_string_pretty(&coverage_map)?)?;

    Ok(())
}

fn input_files(input: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut files = Vec::new();
    collect_input_files(input, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_input_files(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), Box<dyn Error>> {
    if path.is_file() {
        files.push(path.to_path_buf());
        return Ok(());
    }

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_input_files(&path, files)?;
        } else {
            files.push(path);
        }
    }

    Ok(())
}

fn output_path(input: &Path, output_dir: &Path, file: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let relative = if input.is_file() {
        file.file_name()
            .map(PathBuf::from)
            .ok_or("input file has no file name")?
    } else {
        file.strip_prefix(input)?.to_path_buf()
    };
    Ok(output_dir.join(relative))
}

fn instrument_source(
    file: &Path,
    source: &str,
    next_id: &mut usize,
) -> Result<InstrumentedFile, Box<dyn Error>> {
    let parsed = rnix::Root::parse(source);
    let errors = parsed.errors();
    if !errors.is_empty() {
        return Err(format!("failed to parse {}: {errors:?}", file.display()).into());
    }

    let root = parsed.tree();
    let is_flake = file.file_name().is_some_and(|name| name == "flake.nix");
    let flake_outputs_range = is_flake.then(|| flake_outputs_range(&root)).flatten();
    let mut expressions = root
        .syntax()
        .descendants()
        .filter_map(ast::Expr::cast)
        .filter(|expr| should_instrument_expr(expr, source, flake_outputs_range))
        .map(|expr| {
            let range = expr.syntax().text_range();
            let byte_start = u32::from(range.start()) as usize;
            let byte_end = u32::from(range.end()) as usize;
            let id = *next_id;
            *next_id += 1;

            CollectedExpression {
                id,
                byte_start,
                byte_end,
                kind: format!("{:?}", expr.syntax().kind()),
            }
        })
        .collect::<Vec<_>>();

    expressions.sort_by_key(|expr| (expr.byte_start, expr.byte_end, expr.id));

    let mappings = expressions
        .iter()
        .map(|expr| expression_mapping(file, source, expr))
        .collect();
    let source = rewrite_source(source, &expressions);

    Ok(InstrumentedFile { source, mappings })
}

fn should_instrument_expr(
    expr: &ast::Expr,
    source: &str,
    flake_outputs_range: Option<(usize, usize)>,
) -> bool {
    let expr_range = expr.syntax().text_range();
    let expr_start = u32::from(expr_range.start()) as usize;
    let expr_end = u32::from(expr_range.end()) as usize;

    if let Some((outputs_start, outputs_end)) = flake_outputs_range {
        if expr_start < outputs_start
            || expr_end > outputs_end
            || (expr_start == outputs_start && expr_end == outputs_end)
        {
            return false;
        }
    }

    expr.syntax().ancestors().skip(1).all(|ancestor| {
        let kind = format!("{:?}", ancestor.kind());
        if matches!(
            kind.as_str(),
            "NODE_ATTRPATH" | "NODE_INHERIT" | "NODE_PAT_BIND" | "NODE_PAT_ENTRY" | "NODE_PATTERN"
        ) {
            return false;
        }

        if kind == "NODE_LAMBDA" {
            let lambda_range = ancestor.text_range();
            let lambda_start = u32::from(lambda_range.start()) as usize;
            let lambda_end = u32::from(lambda_range.end()) as usize;

            if let Some(colon) = source[lambda_start..lambda_end].find(':') {
                return expr_end > lambda_start + colon;
            }
        }

        true
    })
}

fn flake_outputs_range(root: &rnix::Root) -> Option<(usize, usize)> {
    root.syntax()
        .descendants()
        .filter_map(ast::AttrpathValue::cast)
        .find_map(|attrpath_value| {
            let attrpath = attrpath_value.attrpath()?;
            if attrpath.syntax().to_string().trim() != "outputs" {
                return None;
            }

            let value = attrpath_value.value()?;
            let range = value.syntax().text_range();
            Some((
                u32::from(range.start()) as usize,
                u32::from(range.end()) as usize,
            ))
        })
}

fn expression_mapping(file: &Path, source: &str, expr: &CollectedExpression) -> ExpressionMapping {
    let (line_start, column_start) = line_column(source, expr.byte_start);
    let (line_end, column_end) = line_column(source, expr.byte_end);

    ExpressionMapping {
        id: expr.id,
        file: file.display().to_string(),
        byte_start: expr.byte_start,
        byte_end: expr.byte_end,
        line_start,
        column_start,
        line_end,
        column_end,
        kind: expr.kind.clone(),
    }
}

fn line_column(source: &str, byte: usize) -> (usize, usize) {
    let mut line = 1;
    let mut column = 1;

    for (index, character) in source.char_indices() {
        if index >= byte {
            break;
        }

        if character == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }

    (line, column)
}

fn rewrite_source(source: &str, expressions: &[CollectedExpression]) -> String {
    let mut insertions: BTreeMap<usize, Vec<Insertion>> = BTreeMap::new();

    for expr in expressions {
        insertions
            .entry(expr.byte_start)
            .or_default()
            .push(Insertion::Open {
                id: expr.id,
                byte_end: expr.byte_end,
            });
        insertions
            .entry(expr.byte_end)
            .or_default()
            .push(Insertion::Close {
                byte_start: expr.byte_start,
                byte_end: expr.byte_end,
            });
    }

    let mut output = String::with_capacity(source.len() + expressions.len() * 32);
    let mut previous = 0;

    for (byte, mut insertions) in insertions {
        output.push_str(&source[previous..byte]);
        insertions.sort_by(insertion_order);
        for insertion in insertions {
            match insertion {
                Insertion::Open { id, .. } => {
                    output.push_str("(builtins.trace \"");
                    output.push_str(TRACE_PREFIX);
                    output.push_str(&id.to_string());
                    output.push_str("\" (");
                }
                Insertion::Close { .. } => output.push_str("))"),
            }
        }
        previous = byte;
    }

    output.push_str(&source[previous..]);
    output
}

#[derive(Debug)]
enum Insertion {
    Open { id: usize, byte_end: usize },
    Close { byte_start: usize, byte_end: usize },
}

fn insertion_order(left: &Insertion, right: &Insertion) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    match (left, right) {
        (
            Insertion::Open {
                byte_end: left_end,
                id: left_id,
            },
            Insertion::Open {
                byte_end: right_end,
                id: right_id,
            },
        ) => right_end.cmp(left_end).then_with(|| left_id.cmp(right_id)),
        (
            Insertion::Close {
                byte_start: left_start,
                byte_end: left_end,
            },
            Insertion::Close {
                byte_start: right_start,
                byte_end: right_end,
            },
        ) => right_start
            .cmp(left_start)
            .then_with(|| left_end.cmp(right_end)),
        (Insertion::Close { .. }, Insertion::Open { .. }) => Ordering::Less,
        (Insertion::Open { .. }, Insertion::Close { .. }) => Ordering::Greater,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instruments_nested_expressions() {
        let mut next_id = 0;
        let instrumented = instrument_source(Path::new("test.nix"), "{ x = 1 + 2; }", &mut next_id)
            .expect("instrumentation succeeds");

        assert!(instrumented.source.contains("NIXCOV:0"));
        assert!(instrumented.source.contains("NIXCOV:1"));
        assert!(instrumented.source.contains("NIXCOV:2"));
        assert!(instrumented.mappings.len() >= 4);
        assert_eq!(instrumented.mappings[0].byte_start, 0);
    }

    #[test]
    fn reads_flake_source_path_from_metadata() {
        let source = flake_source_path_from_metadata(
            br#"{
                "description": "test flake",
                "path": "/nix/store/abc123-source"
            }"#,
        )
        .expect("metadata parses");

        assert_eq!(source, PathBuf::from("/nix/store/abc123-source"));
    }

    #[test]
    fn instruments_lambda_value_and_body() {
        let mut next_id = 0;
        let instrumented = instrument_source(Path::new("test.nix"), "x: x + 1", &mut next_id)
            .expect("instrumentation succeeds");

        assert!(
            instrumented
                .source
                .starts_with("(builtins.trace \"NIXCOV:0\" (")
        );
        assert!(instrumented.source.contains("NIXCOV:1"));
        assert!(
            instrumented
                .mappings
                .iter()
                .any(|mapping| mapping.kind == "NODE_LAMBDA")
        );
        assert!(
            instrumented
                .mappings
                .iter()
                .any(|mapping| mapping.kind == "NODE_BIN_OP")
        );
    }

    #[test]
    fn does_not_rewrite_lambda_parameter() {
        let mut next_id = 0;
        let instrumented = instrument_source(
            Path::new("test.nix"),
            "system: let x = 1; in x",
            &mut next_id,
        )
        .expect("instrumentation succeeds");

        assert!(instrumented.source.contains("system:"));
        assert!(!instrumented.source.contains("(system):"));
        assert!(rnix::Root::parse(&instrumented.source).errors().is_empty());
    }

    #[test]
    fn does_not_wrap_flake_root_attrset() {
        let mut next_id = 0;
        let instrumented = instrument_source(
            Path::new("flake.nix"),
            "{ description = \"test\"; inputs.nixpkgs.url = \"github:NixOS/nixpkgs\"; outputs = inputs: { packages.x86_64-linux.default = 1; }; }",
            &mut next_id,
        )
        .expect("instrumentation succeeds");

        assert!(instrumented.source.starts_with('{'));
        assert!(instrumented.source.contains("description = \"test\""));
        assert!(
            instrumented
                .source
                .contains("inputs.nixpkgs.url = \"github:NixOS/nixpkgs\"")
        );
        assert!(instrumented.source.contains("outputs = inputs:"));
        assert!(!instrumented.source.contains("outputs = (builtins.trace"));
        assert!(instrumented.source.contains("NIXCOV:"));
        assert!(rnix::Root::parse(&instrumented.source).errors().is_empty());
    }

    #[test]
    fn does_not_wrap_flake_root_attrset_with_leading_whitespace() {
        let mut next_id = 0;
        let instrumented = instrument_source(
            Path::new("flake.nix"),
            "\n  {\n    description = \"test\";\n    outputs = inputs: { };\n  }\n",
            &mut next_id,
        )
        .expect("instrumentation succeeds");

        assert!(instrumented.source.starts_with("\n  {"));
        assert!(instrumented.source.contains("outputs = inputs:"));
        assert!(!instrumented.source.contains("outputs = (builtins.trace"));
        assert!(
            !instrumented
                .source
                .trim_start()
                .starts_with("(builtins.trace")
        );
        assert!(instrumented.source.contains("NIXCOV:"));
        assert!(rnix::Root::parse(&instrumented.source).errors().is_empty());
    }

    #[test]
    fn does_not_rewrite_static_attrpath_syntax() {
        let mut next_id = 0;
        let instrumented = instrument_source(
            Path::new("test.nix"),
            "{ inputs.foo = config.bar; }",
            &mut next_id,
        )
        .expect("instrumentation succeeds");

        assert!(instrumented.source.contains("inputs.foo"));
        assert!(instrumented.source.contains(".bar"));
        assert!(!instrumented.source.contains(".(builtins.trace"));
        assert!(rnix::Root::parse(&instrumented.source).errors().is_empty());
    }

    #[test]
    fn reports_one_based_line_and_column_ranges() {
        let mut next_id = 0;
        let instrumented =
            instrument_source(Path::new("test.nix"), "let\n  x = 1;\nin x", &mut next_id)
                .expect("instrumentation succeeds");
        let one = instrumented
            .mappings
            .iter()
            .find(|mapping| mapping.byte_start == 10 && mapping.byte_end == 11)
            .expect("integer expression is mapped");

        assert_eq!((one.line_start, one.column_start), (2, 7));
        assert_eq!((one.line_end, one.column_end), (2, 8));
    }
}
