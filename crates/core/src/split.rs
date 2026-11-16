//! Splitting a script into statements, the way the `mysql` command-line
//! client does: on the current delimiter, outside strings and comments, with
//! support for the `DELIMITER` command.

/// One statement and where it sits in the script (byte offsets).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Statement {
    /// The statement without its delimiter or surrounding whitespace.
    pub text: String,
    pub start: usize,
    pub end: usize,
}

fn starts_with_ignore_case(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.len() >= needle.len() && haystack[..needle.len()].eq_ignore_ascii_case(needle)
}

fn line_end(bytes: &[u8], from: usize) -> usize {
    bytes[from..]
        .iter()
        .position(|b| *b == b'\n')
        .map_or(bytes.len(), |p| from + p)
}

/// Splits `script` into statements. Empty statements and statements made
/// only of comments are dropped.
pub fn split(script: &str) -> Vec<Statement> {
    let bytes = script.as_bytes();
    let mut out = Vec::new();
    let mut delimiter: Vec<u8> = b";".to_vec();
    let mut i = 0;
    let mut stmt_start = 0;
    // True once the current statement contains something other than
    // whitespace and comments.
    let mut has_code = false;

    let push = |out: &mut Vec<Statement>, from: usize, to: usize| {
        let raw = &script[from..to];
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let start = from + (raw.len() - raw.trim_start().len());
            out.push(Statement {
                text: trimmed.to_string(),
                start,
                end: start + trimmed.len(),
            });
        }
    };

    while i < bytes.len() {
        let c = bytes[i];

        // DELIMITER is a client command, only recognised between statements.
        if !has_code && starts_with_ignore_case(&bytes[i..], b"delimiter") {
            let after = i + "delimiter".len();
            if matches!(bytes.get(after), Some(b' ') | Some(b'\t')) {
                let end = line_end(bytes, after);
                if let Some(token) = script[after..end].split_whitespace().next() {
                    delimiter = token.as_bytes().to_vec();
                }
                i = end;
                stmt_start = i;
                continue;
            }
        }

        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        if bytes[i..].starts_with(&delimiter) {
            if has_code {
                push(&mut out, stmt_start, i);
            }
            i += delimiter.len();
            stmt_start = i;
            has_code = false;
            continue;
        }

        match c {
            b'\'' | b'"' | b'`' => {
                has_code = true;
                let mut j = i + 1;
                while j < bytes.len() {
                    if bytes[j] == b'\\' && c != b'`' {
                        j += 2;
                    } else if bytes[j] == c {
                        if bytes.get(j + 1) == Some(&c) {
                            j += 2; // doubled quote is an escaped quote
                        } else {
                            j += 1;
                            break;
                        }
                    } else {
                        j += 1;
                    }
                }
                i = j.min(bytes.len());
            }
            b'#' => i = line_end(bytes, i),
            b'-' if bytes.get(i + 1) == Some(&b'-')
                && bytes.get(i + 2).is_none_or(|b| b.is_ascii_whitespace() || b.is_ascii_control()) =>
            {
                i = line_end(bytes, i);
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                // `/*! ... */` and `/*+ ... */` are executed by the server,
                // so they count as code; plain comments do not.
                if matches!(bytes.get(i + 2), Some(b'!') | Some(b'+')) {
                    has_code = true;
                }
                i = bytes[i + 2..]
                    .windows(2)
                    .position(|w| w == b"*/")
                    .map_or(bytes.len(), |p| i + 2 + p + 2);
            }
            _ => {
                has_code = true;
                i += 1;
            }
        }
    }
    if has_code {
        push(&mut out, stmt_start, bytes.len());
    }
    out
}

/// Converts an offset counted in UTF-16 code units (what a browser text
/// editor reports) into a byte offset into `script`.
pub fn utf16_to_byte_offset(script: &str, utf16_offset: usize) -> usize {
    let mut units = 0;
    for (byte_index, ch) in script.char_indices() {
        if units >= utf16_offset {
            return byte_index;
        }
        units += ch.len_utf16();
    }
    script.len()
}

/// Picks the statement the cursor is in. A cursor in the gap after a
/// statement (for example just after its `;`) selects that statement.
pub fn statement_at(statements: &[Statement], byte_offset: usize) -> Option<&Statement> {
    statements
        .iter()
        .rev()
        .find(|s| s.start <= byte_offset)
        .or_else(|| statements.first())
}

/// The first word of a statement, upper-cased, skipping leading comments.
pub fn first_keyword(statement: &str) -> String {
    let mut rest = statement.trim_start();
    loop {
        if let Some(after) = rest.strip_prefix("/*") {
            rest = after.split_once("*/").map_or("", |(_, r)| r).trim_start();
        } else if rest.starts_with("-- ") || rest.starts_with('#') {
            rest = rest.split_once('\n').map_or("", |(_, r)| r).trim_start();
        } else {
            break;
        }
    }
    rest.chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_uppercase()
}

/// Quotes an identifier for use in SQL.
pub fn quote_identifier(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(script: &str) -> Vec<String> {
        split(script).into_iter().map(|s| s.text).collect()
    }

    #[test]
    fn splits_on_semicolons_and_drops_empty_statements() {
        assert_eq!(texts("SELECT 1; SELECT 2;;\n ; SELECT 3"), ["SELECT 1", "SELECT 2", "SELECT 3"]);
    }

    #[test]
    fn semicolons_inside_strings_and_identifiers_do_not_split() {
        assert_eq!(
            texts(r#"SELECT 'a;b', "c;d", `e;f`; SELECT 2"#),
            [r#"SELECT 'a;b', "c;d", `e;f`"#, "SELECT 2"]
        );
    }

    #[test]
    fn escaped_and_doubled_quotes_stay_inside_the_string() {
        assert_eq!(
            texts(r"SELECT 'it\'s; here', 'it''s; there'; SELECT 2"),
            [r"SELECT 'it\'s; here', 'it''s; there'", "SELECT 2"]
        );
    }

    #[test]
    fn semicolons_inside_comments_do_not_split() {
        assert_eq!(
            texts("SELECT 1 -- not; here\n, 2 # nor; here\n, 3 /* nor; here */; SELECT 4"),
            ["SELECT 1 -- not; here\n, 2 # nor; here\n, 3 /* nor; here */", "SELECT 4"]
        );
    }

    #[test]
    fn double_dash_without_a_space_is_not_a_comment() {
        assert_eq!(texts("SELECT 5--3; SELECT 2"), ["SELECT 5--3", "SELECT 2"]);
    }

    #[test]
    fn comment_only_statements_are_dropped_but_version_comments_are_kept() {
        assert_eq!(
            texts("-- just a note\n/* another */;\n/*!40101 SET NAMES utf8mb4 */;"),
            ["/*!40101 SET NAMES utf8mb4 */"]
        );
    }

    #[test]
    fn delimiter_command_allows_procedure_bodies() {
        let script = "DELIMITER $$\nCREATE PROCEDURE p()\nBEGIN\n  SELECT 1;\n  SELECT 2;\nEND$$\nDELIMITER ;\nCALL p();";
        assert_eq!(
            texts(script),
            ["CREATE PROCEDURE p()\nBEGIN\n  SELECT 1;\n  SELECT 2;\nEND", "CALL p()"]
        );
    }

    #[test]
    fn delimiter_is_case_insensitive_and_can_be_slashes() {
        assert_eq!(texts("delimiter //\nSELECT 1; SELECT 2//\nSELECT 3//"), ["SELECT 1; SELECT 2", "SELECT 3"]);
    }

    #[test]
    fn a_column_named_delimiter_is_not_the_command() {
        assert_eq!(texts("SELECT delimiter FROM t; SELECT 2"), ["SELECT delimiter FROM t", "SELECT 2"]);
    }

    #[test]
    fn offsets_point_at_the_trimmed_statement() {
        let script = "  SELECT 1 ;\n\nSELECT 'é';";
        let parts = split(script);
        assert_eq!(&script[parts[0].start..parts[0].end], "SELECT 1");
        assert_eq!(&script[parts[1].start..parts[1].end], "SELECT 'é'");
    }

    #[test]
    fn unterminated_string_runs_to_the_end_without_panicking() {
        assert_eq!(texts("SELECT 'oops; SELECT 2"), ["SELECT 'oops; SELECT 2"]);
        assert_eq!(texts("SELECT 1 /* open"), ["SELECT 1 /* open"]);
        assert_eq!(texts("SELECT '\\"), ["SELECT '\\"]);
    }

    #[test]
    fn statement_at_cursor() {
        let script = "SELECT 1;\n\nSELECT 2;\n";
        let parts = split(script);
        let at = |offset| statement_at(&parts, offset).map(|s| s.text.as_str());
        assert_eq!(at(0), Some("SELECT 1"));
        assert_eq!(at(9), Some("SELECT 1"), "just after the semicolon");
        assert_eq!(at(10), Some("SELECT 1"), "blank line between statements");
        assert_eq!(at(11), Some("SELECT 2"));
        assert_eq!(at(script.len()), Some("SELECT 2"));
        assert_eq!(statement_at(&[], 0), None);
    }

    #[test]
    fn utf16_offsets_convert_to_byte_offsets() {
        let script = "SELECT 'é😀'; SELECT 2";
        // 'é' is 1 UTF-16 unit and 2 bytes; the emoji is 2 units and 4 bytes.
        assert_eq!(utf16_to_byte_offset(script, 0), 0);
        assert_eq!(utf16_to_byte_offset(script, 9), 10);
        assert_eq!(utf16_to_byte_offset(script, 11), 14);
        assert_eq!(utf16_to_byte_offset(script, 999), script.len());
    }

    #[test]
    fn first_keyword_skips_comments() {
        assert_eq!(first_keyword("  select 1"), "SELECT");
        assert_eq!(first_keyword("/* hint */ -- x\n# y\nUse shop"), "USE");
        assert_eq!(first_keyword("(SELECT 1)"), "");
    }

    #[test]
    fn identifiers_are_quoted_with_backticks() {
        assert_eq!(quote_identifier("order"), "`order`");
        assert_eq!(quote_identifier("we`ird"), "`we``ird`");
    }
}
