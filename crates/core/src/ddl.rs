//! Building SQL for schema changes.
//!
//! Everything here returns SQL text for the user to review before it runs.
//! Names are always quoted; character set and collation names, which cannot
//! be quoted, are checked against a strict pattern instead.

use serde::Deserialize;

use crate::error::Error;
use crate::split::quote_identifier;

/// The kinds of object that have a `SHOW CREATE` statement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObjectKind {
    Table,
    View,
    Procedure,
    Function,
    Trigger,
    Event,
}

impl ObjectKind {
    pub fn keyword(self) -> &'static str {
        match self {
            ObjectKind::Table => "TABLE",
            ObjectKind::View => "VIEW",
            ObjectKind::Procedure => "PROCEDURE",
            ObjectKind::Function => "FUNCTION",
            ObjectKind::Trigger => "TRIGGER",
            ObjectKind::Event => "EVENT",
        }
    }
}

fn check_name(name: &str) -> Result<(), Error> {
    if name.trim().is_empty() {
        return Err(Error::Other("Enter a name.".into()));
    }
    if name.chars().count() > 64 {
        return Err(Error::Other("Names can be at most 64 characters long.".into()));
    }
    Ok(())
}

/// Character set and collation names go into SQL unquoted, so only plain
/// words are accepted.
fn check_word(what: &str, value: &str) -> Result<(), Error> {
    if !value.is_empty() && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Ok(())
    } else {
        Err(Error::Other(format!("\"{value}\" is not a valid {what} name.")))
    }
}

fn charset_clause(charset: Option<&str>, collation: Option<&str>) -> Result<String, Error> {
    let mut clause = String::new();
    if let Some(charset) = charset.filter(|c| !c.is_empty()) {
        check_word("character set", charset)?;
        clause.push_str(&format!(" DEFAULT CHARACTER SET {charset}"));
    }
    if let Some(collation) = collation.filter(|c| !c.is_empty()) {
        check_word("collation", collation)?;
        clause.push_str(&format!(" DEFAULT COLLATE {collation}"));
    }
    Ok(clause)
}

pub fn create_schema(name: &str, charset: Option<&str>, collation: Option<&str>) -> Result<String, Error> {
    check_name(name)?;
    Ok(format!(
        "CREATE SCHEMA {}{};",
        quote_identifier(name.trim()),
        charset_clause(charset, collation)?
    ))
}

pub fn alter_schema(name: &str, charset: Option<&str>, collation: Option<&str>) -> Result<String, Error> {
    check_name(name)?;
    let clause = charset_clause(charset, collation)?;
    if clause.is_empty() {
        return Err(Error::Other("Choose a character set or collation to change.".into()));
    }
    Ok(format!("ALTER SCHEMA {}{};", quote_identifier(name.trim()), clause))
}

fn or_replace(create: &str) -> String {
    match create.get(..7) {
        Some(head) if head.eq_ignore_ascii_case("CREATE ") => format!("CREATE OR REPLACE {}", &create[7..]),
        _ => create.to_string(),
    }
}

/// A script that changes an existing object, starting from the statement the
/// server reports for it.
///
/// Routines, triggers and events cannot be altered in place. Where the server
/// can replace one in a single statement (`supports_or_replace`: MariaDB),
/// the script does that, so a failed change leaves the original untouched.
/// MySQL has no such statement for them, so there the script drops and
/// recreates, and says so at the top.
pub fn alter_script(
    kind: ObjectKind,
    schema: &str,
    name: &str,
    create_statement: &str,
    supports_or_replace: bool,
) -> String {
    let schema_q = quote_identifier(schema);
    let name_q = quote_identifier(name);
    let create = create_statement.trim().trim_end_matches(';');
    const DEFINER_NOTE: &str = "-- Keeping a DEFINER other than your own account needs a privilege for that\n\
                                -- (SET USER or SUPER). Remove the DEFINER clause to create it as yourself.\n";
    match kind {
        ObjectKind::Table => {
            let reference: String = create.lines().map(|line| format!("--   {line}\n")).collect();
            format!(
                "-- Current definition of {schema_q}.{name_q}, for reference:\n{reference}\n\
                 ALTER TABLE {schema_q}.{name_q}\n  -- for example: ADD COLUMN `new_column` VARCHAR(100) NULL\n;\n"
            )
        }
        ObjectKind::View => format!("USE {schema_q};\n\n{};\n", or_replace(create)),
        _ if supports_or_replace => format!(
            "{DEFINER_NOTE}USE {schema_q};\n\nDELIMITER $$\n{}$$\nDELIMITER ;\n",
            or_replace(create)
        ),
        _ => format!(
            "-- This server cannot replace a {what} in one statement, so this script drops it\n\
             -- and creates it again. If the CREATE fails, the {what} is gone until you fix\n\
             -- the statement and run it again; keep this tab open until it has succeeded.\n\
             {DEFINER_NOTE}USE {schema_q};\n\nDROP {keyword} IF EXISTS {name_q};\n\nDELIMITER $$\n{create}$$\nDELIMITER ;\n",
            what = kind.keyword().to_ascii_lowercase(),
            keyword = kind.keyword(),
        ),
    }
}

/// A starting point for creating a new object, to be edited before running.
pub fn create_template(kind: ObjectKind, schema: Option<&str>) -> String {
    let prefix = schema.map(|s| format!("{}.", quote_identifier(s))).unwrap_or_default();
    let use_schema = schema.map(|s| format!("USE {};\n\n", quote_identifier(s))).unwrap_or_default();
    match kind {
        ObjectKind::Table => format!(
            "CREATE TABLE {prefix}`new_table` (\n  `id` INT NOT NULL AUTO_INCREMENT,\n  `name` VARCHAR(100) NOT NULL,\n  \
             `created_at` TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,\n  PRIMARY KEY (`id`)\n);\n"
        ),
        ObjectKind::View => {
            format!("CREATE VIEW {prefix}`new_view` AS\nSELECT *\nFROM {prefix}`table_name`;\n")
        }
        ObjectKind::Procedure => format!(
            "{use_schema}DELIMITER $$\nCREATE PROCEDURE `new_procedure`()\nBEGIN\n  SELECT 1;\nEND$$\nDELIMITER ;\n"
        ),
        ObjectKind::Function => format!(
            "{use_schema}DELIMITER $$\nCREATE FUNCTION `new_function`() RETURNS INT\nDETERMINISTIC\nBEGIN\n  RETURN 1;\nEND$$\nDELIMITER ;\n"
        ),
        ObjectKind::Trigger => format!(
            "{use_schema}DELIMITER $$\nCREATE TRIGGER `new_trigger` BEFORE INSERT ON `table_name`\nFOR EACH ROW\nBEGIN\n  \
             SET NEW.`column_name` = NEW.`column_name`;\nEND$$\nDELIMITER ;\n"
        ),
        ObjectKind::Event => format!(
            "{use_schema}DELIMITER $$\nCREATE EVENT `new_event`\nON SCHEDULE EVERY 1 DAY\nDO\nBEGIN\n  SELECT 1;\nEND$$\nDELIMITER ;\n"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::split::split;

    #[test]
    fn create_schema_quotes_the_name_and_adds_the_charset() {
        assert_eq!(create_schema("shop", None, None).unwrap(), "CREATE SCHEMA `shop`;");
        assert_eq!(
            create_schema(" we`ird ", Some("utf8mb4"), Some("utf8mb4_0900_ai_ci")).unwrap(),
            "CREATE SCHEMA `we``ird` DEFAULT CHARACTER SET utf8mb4 DEFAULT COLLATE utf8mb4_0900_ai_ci;"
        );
    }

    #[test]
    fn charset_and_collation_cannot_carry_sql() {
        assert!(create_schema("shop", Some("utf8mb4; DROP SCHEMA x"), None).is_err());
        assert!(create_schema("shop", None, Some("a b")).is_err());
        assert!(alter_schema("shop", Some("utf8mb4`"), None).is_err());
    }

    #[test]
    fn names_must_be_present_and_short_enough() {
        assert!(create_schema("  ", None, None).is_err());
        assert!(create_schema(&"x".repeat(65), None, None).is_err());
        assert!(create_schema(&"x".repeat(64), None, None).is_ok());
    }

    #[test]
    fn alter_schema_needs_something_to_change() {
        assert!(alter_schema("shop", None, None).is_err());
        assert_eq!(
            alter_schema("shop", Some("latin1"), None).unwrap(),
            "ALTER SCHEMA `shop` DEFAULT CHARACTER SET latin1;"
        );
    }

    #[test]
    fn routine_alter_script_drops_and_recreates_and_splits_correctly() {
        let create = "CREATE DEFINER=`root`@`%` PROCEDURE `two_sets`()\nBEGIN\n  SELECT 1;\n  SELECT 2;\nEND";
        let script = alter_script(ObjectKind::Procedure, "shop", "two_sets", create, false);
        let statements: Vec<String> = split(&script).into_iter().map(|s| s.text).collect();
        assert_eq!(statements.len(), 3, "{statements:?}");
        assert!(statements[0].ends_with("USE `shop`"), "leading comments stay with the first statement");
        assert_eq!(statements[1], "DROP PROCEDURE IF EXISTS `two_sets`");
        assert_eq!(statements[2], create, "the body's semicolons must not split the CREATE");
        assert!(script.contains("drops it"), "the script warns that it drops first");
    }

    #[test]
    fn routine_alter_script_replaces_in_place_where_the_server_can() {
        let create = "CREATE DEFINER=`root`@`%` PROCEDURE `two_sets`()\nBEGIN\n  SELECT 1;\nEND";
        let script = alter_script(ObjectKind::Procedure, "shop", "two_sets", create, true);
        let statements: Vec<String> = split(&script).into_iter().map(|s| s.text).collect();
        assert_eq!(statements.len(), 2, "{statements:?}");
        assert!(statements[1].starts_with("CREATE OR REPLACE DEFINER=`root`@`%` PROCEDURE `two_sets`()"));
        assert!(!script.contains("DROP "), "nothing is dropped");
    }

    #[test]
    fn view_alter_script_becomes_create_or_replace() {
        let script = alter_script(ObjectKind::View, "shop", "v", "CREATE ALGORITHM=UNDEFINED VIEW `v` AS select 1", false);
        assert!(script.contains("CREATE OR REPLACE ALGORITHM=UNDEFINED VIEW `v` AS select 1;"));
        assert!(script.starts_with("USE `shop`;"));
    }

    #[test]
    fn table_alter_script_keeps_the_definition_as_comments_only() {
        let script = alter_script(ObjectKind::Table, "shop", "orders", "CREATE TABLE `orders` (\n  `id` int\n)", false);
        let statements = split(&script);
        assert_eq!(statements.len(), 1, "only the ALTER is a statement: {statements:?}");
        assert!(statements[0].text.contains("ALTER TABLE `shop`.`orders`"));
        assert!(script.contains("--   CREATE TABLE `orders` ("));
    }

    #[test]
    fn templates_split_into_runnable_statements() {
        for kind in [ObjectKind::Procedure, ObjectKind::Function, ObjectKind::Trigger, ObjectKind::Event] {
            let statements = split(&create_template(kind, Some("shop")));
            assert_eq!(statements.len(), 2, "{kind:?}: USE plus one CREATE");
            assert!(statements[1].text.starts_with("CREATE "));
            assert!(statements[1].text.ends_with("END"));
        }
        assert_eq!(split(&create_template(ObjectKind::Table, None)).len(), 1);
        assert!(create_template(ObjectKind::View, Some("shop")).contains("`shop`.`new_view`"));
    }
}
