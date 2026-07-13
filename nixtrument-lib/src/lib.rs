use anyhow::{Context, Result, anyhow};
use rnix::ast;
use rowan::ast::AstNode;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, ExitStatus, Stdio};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

pub const TRACE_PREFIX: &str = "NIXCOV";

#[derive(Debug, Deserialize, Serialize)]
pub struct CoverageMap {
    pub trace_prefix: String,
    pub run_id: String,
    pub expressions: Vec<ExpressionMapping>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ExpressionMapping {
    pub id: usize,
    pub file: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub line_start: usize,
    pub column_start: usize,
    pub line_end: usize,
    pub column_end: usize,
    pub kind: String,
}

#[derive(Clone, Debug)]
struct CollectedExpression {
    id: usize,
    byte_start: usize,
    byte_end: usize,
    kind: String,
}

#[derive(Debug)]
pub struct InstrumentedFile {
    pub source: String,
    pub mappings: Vec<ExpressionMapping>,
}

#[derive(Debug, Deserialize)]
struct FlakeMetadata {
    path: PathBuf,
}

pub fn run_coverage(instrument_bin: &Path, flake_ref: &str) -> Result<()> {
    if !instrument_bin.starts_with("/nix/store") {
        return Err(anyhow!(
            "instrument binary must be a /nix/store path, got {}",
            instrument_bin.display()
        ));
    }

    let run_id = generate_run_id()?;
    let source = resolve_flake_source(flake_ref)?;
    println!("source: {}", source.display());
    let instrumented = build_instrumented_source(instrument_bin, &source, &run_id)?;
    let instrumented_source = instrumented.join("source");
    let coverage_map = instrumented.join("coverage-map.json");
    println!("instrumented source: {}", instrumented_source.display());
    println!("coverage map: {}", coverage_map.display());
    println!("run id: {run_id}");

    let (status, hits) = run_flake_check_collect_hits(&instrumented_source, &run_id)?;
    let coverage = coverage_summary(&coverage_map, &run_id, &hits)?;

    println!(
        "covered expressions: {} / {} ({:.2}%)",
        coverage.covered_expressions,
        coverage.total_expressions,
        coverage.expression_percent()
    );
    println!(
        "covered lines: {} / {} ({:.2}%)",
        coverage.covered_lines,
        coverage.total_lines,
        coverage.line_percent()
    );

    if !status.success() {
        return Err(anyhow!(
            "nix flake check failed for {}",
            instrumented_source.display()
        ));
    }

    Ok(())
}

fn generate_run_id() -> Result<String> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    Ok(format!("{now:x}-{:x}", std::process::id()))
}

fn run_flake_check_collect_hits(
    instrumented_source: &Path,
    run_id: &str,
) -> Result<(ExitStatus, BTreeSet<usize>)> {
    let mut child = ProcessCommand::new("nix")
        .args(["flake", "check"])
        .arg(instrumented_source)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to start nix flake check")?;

    let stdout = child
        .stdout
        .take()
        .context("failed to capture nix stdout")?;
    let stderr = child
        .stderr
        .take()
        .context("failed to capture nix stderr")?;
    let run_id = run_id.to_string();
    let stdout_run_id = run_id.clone();

    let stdout = thread::spawn(move || stream_lines(stdout, &stdout_run_id, false));
    let stderr = thread::spawn(move || stream_lines(stderr, &run_id, true));
    let status = child.wait()?;

    let mut hits = stdout
        .join()
        .map_err(|_| anyhow!("stdout reader thread panicked"))??;
    hits.extend(
        stderr
            .join()
            .map_err(|_| anyhow!("stderr reader thread panicked"))??,
    );

    Ok((status, hits))
}

fn stream_lines<R: std::io::Read>(
    reader: R,
    run_id: &str,
    stderr: bool,
) -> std::io::Result<BTreeSet<usize>> {
    let mut hits = BTreeSet::new();

    for line in BufReader::new(reader).lines() {
        let line = line?;
        let line_hits = parse_hits_from_text(run_id, &line);
        if line_hits.is_empty() {
            if stderr {
                eprintln!("{line}");
            } else {
                println!("{line}");
            }
        } else {
            hits.extend(line_hits);
        }
    }

    Ok(hits)
}

#[derive(Debug)]
struct CoverageSummary {
    covered_expressions: usize,
    total_expressions: usize,
    covered_lines: usize,
    total_lines: usize,
}

impl CoverageSummary {
    fn expression_percent(&self) -> f64 {
        percent(self.covered_expressions, self.total_expressions)
    }

    fn line_percent(&self) -> f64 {
        percent(self.covered_lines, self.total_lines)
    }
}

fn percent(part: usize, total: usize) -> f64 {
    if total == 0 {
        100.0
    } else {
        part as f64 * 100.0 / total as f64
    }
}

fn parse_hits_from_text(run_id: &str, text: &str) -> BTreeSet<usize> {
    let marker = format!("{TRACE_PREFIX}:{run_id}:");
    let mut hits = BTreeSet::new();

    for (index, _) in text.match_indices(&marker) {
        let id = text[index + marker.len()..]
            .chars()
            .take_while(|character| character.is_ascii_digit())
            .collect::<String>();

        if let Ok(id) = id.parse() {
            hits.insert(id);
        }
    }

    hits
}

fn coverage_summary(
    coverage_map: &Path,
    run_id: &str,
    hits: &BTreeSet<usize>,
) -> Result<CoverageSummary> {
    let map: CoverageMap = serde_json::from_str(&fs::read_to_string(coverage_map)?)?;
    if map.run_id != run_id {
        return Err(anyhow!(
            "coverage map run id {} does not match current run id {run_id}",
            map.run_id
        ));
    }

    let mut all_lines = BTreeSet::new();
    let mut covered_lines = BTreeSet::new();
    let mut covered_expressions = BTreeSet::new();
    let mut sources = BTreeMap::new();

    for expression in &map.expressions {
        if hits.contains(&expression.id) {
            covered_expressions.insert(expression.id);
        }

        let source = sources
            .entry(expression.file.clone())
            .or_insert_with(|| fs::read_to_string(&expression.file));
        let Ok(source) = source else {
            continue;
        };

        for line in non_comment_lines(source, expression.line_start, expression.line_end) {
            all_lines.insert((expression.file.clone(), line));
            if hits.contains(&expression.id) {
                covered_lines.insert((expression.file.clone(), line));
            }
        }
    }

    Ok(CoverageSummary {
        covered_expressions: covered_expressions.len(),
        total_expressions: map.expressions.len(),
        covered_lines: covered_lines.len(),
        total_lines: all_lines.len(),
    })
}

fn non_comment_lines(source: &str, start: usize, end: usize) -> Vec<usize> {
    source
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let number = index + 1;
            let trimmed = line.trim_start();
            (number >= start && number <= end && !trimmed.is_empty() && !trimmed.starts_with('#'))
                .then_some(number)
        })
        .collect()
}

pub fn build_instrumented_source(
    instrument_bin: &Path,
    source: &Path,
    run_id: &str,
) -> Result<PathBuf> {
    let expr = instrumentation_derivation_expr(instrument_bin, source, run_id)?;
    let output = ProcessCommand::new("nix")
        .args([
            "build",
            "--impure",
            "--no-link",
            "--print-out-paths",
            "--expr",
        ])
        .arg(expr)
        .output()
        .context("failed to run nix build for instrumentation derivation")?;

    if !output.status.success() {
        return Err(anyhow!(
            "failed to build instrumentation derivation: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let path = String::from_utf8(output.stdout)?;
    Ok(PathBuf::from(path.trim()))
}

pub fn instrumentation_derivation_expr(
    instrument_bin: &Path,
    source: &Path,
    run_id: &str,
) -> Result<String> {
    let (instrument_package, instrument_bin_relative) =
        store_package_and_relative_path(instrument_bin)?;
    let instrument_package = nix_store_path_expr(&instrument_package)?;
    let instrument_bin_relative = nix_string_literal(&instrument_bin_relative)?;
    let source = nix_store_path_expr(source)?;
    let run_id = nix_string_literal(run_id)?;

    Ok(format!(
        r#"
        let
          pkgs = (builtins.getFlake "nixpkgs").legacyPackages.${{builtins.currentSystem}};
          instrumentPackage = {instrument_package};
          instrumentBin = "${{instrumentPackage}}/${{instrumentBinRelative}}";
          instrumentBinRelative = {instrument_bin_relative};
          runId = {run_id};
          source = {source};
        in
        pkgs.runCommand "nixtrument-instrumented-source" {{ }} ''
          mkdir -p "$out"
          ${{instrumentBin}} instrument-source --run-id '${{runId}}' ${{source}} "$out/source" "$out/coverage-map.json"
        ''
        "#
    ))
}

fn store_package_and_relative_path(path: &Path) -> Result<(PathBuf, String)> {
    let mut components = path.components();

    if components.next() != Some(std::path::Component::RootDir)
        || components.next() != Some(std::path::Component::Normal("nix".as_ref()))
        || components.next() != Some(std::path::Component::Normal("store".as_ref()))
    {
        return Err(anyhow!("path must be in /nix/store: {}", path.display()));
    }

    let store_entry = components
        .next()
        .with_context(|| format!("path must include a store entry: {}", path.display()))?;
    let package = Path::new("/nix/store").join(store_entry.as_os_str());
    let relative = path.strip_prefix(&package)?;

    if relative.as_os_str().is_empty() {
        return Err(anyhow!(
            "path must point to a binary inside a store package: {}",
            path.display()
        ));
    }

    let relative = relative
        .to_str()
        .with_context(|| format!("path is not valid UTF-8: {}", path.display()))?
        .to_string();

    Ok((package, relative))
}

fn nix_string_literal(value: &str) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

fn nix_store_path_expr(path: &Path) -> Result<String> {
    let path = path
        .to_str()
        .with_context(|| format!("path is not valid UTF-8: {}", path.display()))?;

    if !path.starts_with("/nix/store/") {
        return Err(anyhow!("path must be in /nix/store: {path}"));
    }

    Ok(format!("builtins.storePath {}", nix_string_literal(path)?))
}

pub fn instrument_flake(
    flake_ref: &str,
    output_dir: &Path,
    sidecar: &Path,
    run_id: &str,
) -> Result<()> {
    let source = resolve_flake_source(flake_ref)?;
    instrument_path(&source, output_dir, sidecar, run_id)
}

pub fn resolve_flake_source(flake_ref: &str) -> Result<PathBuf> {
    let output = ProcessCommand::new("nix")
        .args(["flake", "metadata", "--json", flake_ref])
        .output()
        .with_context(|| format!("failed to run nix flake metadata for {flake_ref:?}"))?;

    if !output.status.success() {
        return Err(anyhow!(
            "failed to resolve flake {flake_ref:?}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    flake_source_path_from_metadata(&output.stdout)
}

fn flake_source_path_from_metadata(metadata: &[u8]) -> Result<PathBuf> {
    let metadata: FlakeMetadata = serde_json::from_slice(metadata)?;
    Ok(metadata.path)
}

pub fn instrument_path(
    input: &Path,
    output_dir: &Path,
    sidecar: &Path,
    run_id: &str,
) -> Result<()> {
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
            let instrumented = instrument_source(&file, &source, &mut next_id, run_id)?;
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
        trace_prefix: TRACE_PREFIX.to_string(),
        run_id: run_id.to_string(),
        expressions: all_mappings,
    };
    fs::write(sidecar, serde_json::to_string_pretty(&coverage_map)?)?;

    Ok(())
}

fn input_files(input: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_input_files(input, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_input_files(path: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
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

fn output_path(input: &Path, output_dir: &Path, file: &Path) -> Result<PathBuf> {
    let relative = if input.is_file() {
        file.file_name()
            .map(PathBuf::from)
            .context("input file has no file name")?
    } else {
        file.strip_prefix(input)?.to_path_buf()
    };
    Ok(output_dir.join(relative))
}

fn instrument_source(
    file: &Path,
    source: &str,
    next_id: &mut usize,
    run_id: &str,
) -> Result<InstrumentedFile> {
    let parsed = rnix::Root::parse(source);
    let errors = parsed.errors();
    if !errors.is_empty() {
        return Err(anyhow!("failed to parse {}: {errors:?}", file.display()));
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
    let source = rewrite_source(source, &expressions, run_id);

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

fn rewrite_source(source: &str, expressions: &[CollectedExpression], run_id: &str) -> String {
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
                    output.push(':');
                    output.push_str(run_id);
                    output.push(':');
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

    const RUN_ID: &str = "test-run";

    #[test]
    fn instruments_nested_expressions() {
        let mut next_id = 0;
        let instrumented = instrument_source(
            Path::new("test.nix"),
            "{ x = 1 + 2; }",
            &mut next_id,
            RUN_ID,
        )
        .expect("instrumentation succeeds");

        assert!(instrumented.source.contains("NIXCOV:test-run:0"));
        assert!(instrumented.source.contains("NIXCOV:test-run:1"));
        assert!(instrumented.source.contains("NIXCOV:test-run:2"));
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
    fn builds_instrumentation_derivation_expression() {
        let expr = instrumentation_derivation_expr(
            Path::new("/nix/store/abc123-nixtrument-instrument/bin/nixtrument-instrument"),
            Path::new("/nix/store/def456-source"),
            RUN_ID,
        )
        .expect("expression builds");

        assert!(expr.contains("pkgs.runCommand \"nixtrument-instrumented-source\""));
        assert!(expr.contains(
            "instrumentPackage = builtins.storePath \"/nix/store/abc123-nixtrument-instrument\";"
        ));
        assert!(expr.contains("instrumentBinRelative = \"bin/nixtrument-instrument\";"));
        assert!(
            expr.contains("instrumentBin = \"${instrumentPackage}/${instrumentBinRelative}\";")
        );
        assert!(expr.contains("runId = \"test-run\";"));
        assert!(expr.contains("source = builtins.storePath \"/nix/store/def456-source\";"));
        assert!(expr.contains("${instrumentBin} instrument-source --run-id '${runId}' ${source}"));
        assert!(expr.contains("$out/source"));
        assert!(expr.contains("$out/coverage-map.json"));
    }

    #[test]
    fn instruments_lambda_value_and_body() {
        let mut next_id = 0;
        let instrumented =
            instrument_source(Path::new("test.nix"), "x: x + 1", &mut next_id, RUN_ID)
                .expect("instrumentation succeeds");

        assert!(
            instrumented
                .source
                .starts_with("(builtins.trace \"NIXCOV:test-run:0\" (")
        );
        assert!(instrumented.source.contains("NIXCOV:test-run:1"));
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
            RUN_ID,
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
            RUN_ID,
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
        assert!(instrumented.source.contains("NIXCOV:test-run:"));
        assert!(rnix::Root::parse(&instrumented.source).errors().is_empty());
    }

    #[test]
    fn does_not_wrap_flake_root_attrset_with_leading_whitespace() {
        let mut next_id = 0;
        let instrumented = instrument_source(
            Path::new("flake.nix"),
            "\n  {\n    description = \"test\";\n    outputs = inputs: { };\n  }\n",
            &mut next_id,
            RUN_ID,
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
        assert!(instrumented.source.contains("NIXCOV:test-run:"));
        assert!(rnix::Root::parse(&instrumented.source).errors().is_empty());
    }

    #[test]
    fn does_not_rewrite_static_attrpath_syntax() {
        let mut next_id = 0;
        let instrumented = instrument_source(
            Path::new("test.nix"),
            "{ inputs.foo = config.bar; }",
            &mut next_id,
            RUN_ID,
        )
        .expect("instrumentation succeeds");

        assert!(instrumented.source.contains("inputs.foo"));
        assert!(instrumented.source.contains(".bar"));
        assert!(!instrumented.source.contains(".(builtins.trace"));
        assert!(rnix::Root::parse(&instrumented.source).errors().is_empty());
    }

    #[test]
    fn parses_only_matching_run_hits() {
        let hits = parse_hits_from_text(
            RUN_ID,
            "trace: NIXCOV:test-run:1\ntrace: NIXCOV:other:2\ntrace: NIXCOV:test-run:42 extra",
        );

        assert_eq!(hits, BTreeSet::from([1, 42]));
    }

    #[test]
    fn reports_one_based_line_and_column_ranges() {
        let mut next_id = 0;
        let instrumented = instrument_source(
            Path::new("test.nix"),
            "let\n  x = 1;\nin x",
            &mut next_id,
            RUN_ID,
        )
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
