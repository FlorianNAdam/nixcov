use crate::ansi::{ansi_len, lpad_ansi};

pub fn print_table(lines: Vec<String>) {
    print!("{}", format_table(lines));
}

pub fn format_table(lines: Vec<String>) -> String {
    let split_lines: Vec<Vec<&str>> = lines
        .iter()
        .map(|line| line.split('\t').collect())
        .collect();

    let num_cols = split_lines.iter().map(|cols| cols.len()).max().unwrap_or(0);

    let mut col_widths = vec![0; num_cols];
    for cols in &split_lines {
        for (i, col) in cols.iter().enumerate() {
            let visible_len = ansi_len(col);
            if visible_len > col_widths[i] {
                col_widths[i] = visible_len;
            }
        }
    }

    let mut output = String::new();
    for cols in split_lines {
        for (i, col) in cols.iter().enumerate() {
            if i < cols.len() - 1 {
                let padded = lpad_ansi(col, col_widths[i]);
                output.push_str(&padded);
                output.push_str("  ");
            } else {
                output.push_str(col)
            }
        }
        output.push('\n');
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use console::style;

    #[test]
    fn format_table_aligns_tab_separated_columns() {
        let output = format_table(vec![
            "name\tstate\timage".to_string(),
            "web\trunning\tnginx".to_string(),
            "database\tstopped\tpostgres".to_string(),
        ]);

        assert_eq!(
            output,
            "name      state    image\nweb       running  nginx\ndatabase  stopped  postgres\n"
        );
    }

    #[test]
    fn format_table_uses_visible_width_for_ansi_columns() {
        let green_ok = style("ok").green().to_string();
        let output = format_table(vec![format!("{}\tweb", green_ok), "failed\tdb".to_string()]);

        assert_eq!(output, format!("{}      web\nfailed  db\n", green_ok));
    }

    #[test]
    fn format_table_handles_empty_input() {
        assert_eq!(format_table(vec![]), "");
    }
}
