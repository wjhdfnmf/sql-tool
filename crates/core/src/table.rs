//! The table designer's model, and the SQL it turns into.
//!
//! A [`TableSpec`] describes a table the way the designer shows it. Creating
//! a table writes the whole spec out. Altering compares the spec read from
//! the server with the edited one and writes only the differences, so
//! anything the designer does not model (checks, generated columns,
//! partitioning) is left alone as long as it is not touched.

use serde::{Deserialize, Serialize};

use crate::ddl::{check_name, check_word, ObjectKind};
use crate::error::Error;
use crate::export::sql_literal;
use crate::split::quote_identifier;
use crate::store::ColumnKind;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ColumnSpec {
    pub name: String,
    /// As written in SQL, for example `VARCHAR(100)` or `INT UNSIGNED`.
    pub data_type: String,
    pub nullable: bool,
    /// The text after `DEFAULT`, exactly as SQL: `0`, `'new'`, `CURRENT_TIMESTAMP`.
    pub default: Option<String>,
    /// The text after `ON UPDATE`, for timestamp columns.
    pub on_update: Option<String>,
    pub auto_increment: bool,
    pub comment: String,
    /// Set only when it differs from the table's collation, so that
    /// modifying the column does not silently reset it.
    pub collation: Option<String>,
    /// The column's name on the server; `None` for a column being added.
    pub original_name: Option<String>,
    /// A column the designer cannot describe fully (a generated column).
    /// It is shown but never modified.
    pub locked: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IndexSpec {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    pub original_name: Option<String>,
    /// An index the designer cannot describe fully (prefix lengths,
    /// full-text, spatial, expressions). Shown but never modified.
    pub locked: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ForeignKeySpec {
    /// The constraint's name. May be left empty for a new one; the server
    /// then names it.
    pub name: String,
    pub columns: Vec<String>,
    /// Empty means the table's own schema.
    pub referenced_schema: String,
    pub referenced_table: String,
    pub referenced_columns: Vec<String>,
    /// `RESTRICT`, `CASCADE`, `SET NULL`, `NO ACTION` or `SET DEFAULT`;
    /// empty leaves it to the server's default.
    pub on_update: String,
    pub on_delete: String,
    /// The constraint's name on the server; `None` for one being added.
    pub original_name: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TableSpec {
    pub schema: String,
    pub name: String,
    pub columns: Vec<ColumnSpec>,
    pub primary_key: Vec<String>,
    pub indexes: Vec<IndexSpec>,
    pub foreign_keys: Vec<ForeignKeySpec>,
    pub engine: Option<String>,
    pub collation: Option<String>,
    pub comment: String,
}

fn fail(message: impl Into<String>) -> Error {
    Error::Other(message.into())
}

fn text_literal(text: &str) -> String {
    sql_literal(Some(text.as_bytes()), ColumnKind::Text)
}

fn column_list(names: &[String]) -> String {
    names.iter().map(|n| quote_identifier(n)).collect::<Vec<_>>().join(", ")
}

fn column_definition(column: &ColumnSpec) -> String {
    let mut sql = format!("{} {}", quote_identifier(column.name.trim()), column.data_type.trim());
    if let Some(collation) = column.collation.as_deref().filter(|c| !c.is_empty()) {
        sql.push_str(&format!(" COLLATE {collation}"));
    }
    sql.push_str(if column.nullable { " NULL" } else { " NOT NULL" });
    if let Some(default) = column.default.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        sql.push_str(&format!(" DEFAULT {default}"));
    }
    if let Some(on_update) = column.on_update.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        sql.push_str(&format!(" ON UPDATE {on_update}"));
    }
    if column.auto_increment {
        sql.push_str(" AUTO_INCREMENT");
    }
    if !column.comment.is_empty() {
        sql.push_str(&format!(" COMMENT {}", text_literal(&column.comment)));
    }
    sql
}

fn index_definition(index: &IndexSpec) -> String {
    format!("{}INDEX {} ({})", if index.unique { "UNIQUE " } else { "" }, quote_identifier(index.name.trim()), column_list(&index.columns))
}

const REFERENCE_ACTIONS: [&str; 5] = ["RESTRICT", "CASCADE", "SET NULL", "NO ACTION", "SET DEFAULT"];

fn action(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_uppercase()
}

fn foreign_key_definition(key: &ForeignKeySpec, own_schema: &str) -> String {
    let mut sql = String::new();
    if !key.name.trim().is_empty() {
        sql.push_str(&format!("CONSTRAINT {} ", quote_identifier(key.name.trim())));
    }
    let schema = if key.referenced_schema.trim().is_empty() { own_schema } else { key.referenced_schema.trim() };
    sql.push_str(&format!(
        "FOREIGN KEY ({}) REFERENCES {} ({})",
        column_list(&key.columns),
        qualified(schema, &key.referenced_table),
        column_list(&key.referenced_columns)
    ));
    if !action(&key.on_delete).is_empty() {
        sql.push_str(&format!(" ON DELETE {}", action(&key.on_delete)));
    }
    if !action(&key.on_update).is_empty() {
        sql.push_str(&format!(" ON UPDATE {}", action(&key.on_update)));
    }
    sql
}

fn validate(spec: &TableSpec) -> Result<(), Error> {
    check_name(&spec.name)?;
    if spec.columns.is_empty() {
        return Err(fail("A table needs at least one column."));
    }
    let mut seen = std::collections::HashSet::new();
    for column in &spec.columns {
        check_name(&column.name).map_err(|_| fail("Every column needs a name of at most 64 characters."))?;
        if !seen.insert(column.name.trim().to_lowercase()) {
            return Err(fail(format!("There are two columns named {}.", column.name.trim())));
        }
        let data_type = column.data_type.trim();
        if data_type.is_empty() {
            return Err(fail(format!("Column {} needs a type.", column.name.trim())));
        }
        if data_type.contains([';', '\n']) {
            return Err(fail(format!("The type of column {} is not valid.", column.name.trim())));
        }
        if let Some(collation) = column.collation.as_deref().filter(|c| !c.is_empty()) {
            check_word("collation", collation)?;
        }
    }
    let known = |name: &String| spec.columns.iter().any(|c| c.name.trim() == name.trim());
    if let Some(missing) = spec.primary_key.iter().find(|name| !known(name)) {
        return Err(fail(format!("The primary key refers to a column that does not exist: {missing}.")));
    }
    let mut index_names = std::collections::HashSet::new();
    for index in &spec.indexes {
        check_name(&index.name).map_err(|_| fail("Every index needs a name of at most 64 characters."))?;
        if !index_names.insert(index.name.trim().to_lowercase()) {
            return Err(fail(format!("There are two indexes named {}.", index.name.trim())));
        }
        if index.columns.is_empty() {
            return Err(fail(format!("Index {} needs at least one column.", index.name.trim())));
        }
        if let Some(missing) = index.columns.iter().find(|name| !known(name)) {
            return Err(fail(format!("Index {} refers to a column that does not exist: {missing}.", index.name.trim())));
        }
    }
    let mut key_names = std::collections::HashSet::new();
    for key in &spec.foreign_keys {
        let label = if key.name.trim().is_empty() { "A foreign key".to_string() } else { format!("Foreign key {}", key.name.trim()) };
        if !key.name.trim().is_empty() {
            check_name(&key.name).map_err(|_| fail("A foreign key's name can be at most 64 characters."))?;
            if !key_names.insert(key.name.trim().to_lowercase()) {
                return Err(fail(format!("There are two foreign keys named {}.", key.name.trim())));
            }
        }
        if key.columns.is_empty() {
            return Err(fail(format!("{label} needs at least one column.")));
        }
        if let Some(missing) = key.columns.iter().find(|name| !known(name)) {
            return Err(fail(format!("{label} refers to a column that does not exist: {missing}.")));
        }
        check_name(&key.referenced_table).map_err(|_| fail(format!("{label} needs the table it refers to.")))?;
        if key.referenced_columns.len() != key.columns.len() || key.referenced_columns.iter().any(|c| c.trim().is_empty()) {
            return Err(fail(format!("{label} needs one column in {} for each of its own columns.", key.referenced_table.trim())));
        }
        for rule in [&key.on_update, &key.on_delete] {
            let rule = action(rule);
            if !rule.is_empty() && !REFERENCE_ACTIONS.contains(&rule.as_str()) {
                return Err(fail(format!("{rule} is not something a foreign key can do on update or delete.")));
            }
        }
    }
    if let Some(engine) = spec.engine.as_deref().filter(|e| !e.is_empty()) {
        check_word("engine", engine)?;
    }
    if let Some(collation) = spec.collation.as_deref().filter(|c| !c.is_empty()) {
        check_word("collation", collation)?;
    }
    Ok(())
}

fn qualified(schema: &str, name: &str) -> String {
    if schema.is_empty() {
        quote_identifier(name.trim())
    } else {
        format!("{}.{}", quote_identifier(schema), quote_identifier(name.trim()))
    }
}

pub fn create_table(spec: &TableSpec) -> Result<String, Error> {
    validate(spec)?;
    let mut lines: Vec<String> = spec.columns.iter().map(column_definition).collect();
    if !spec.primary_key.is_empty() {
        lines.push(format!("PRIMARY KEY ({})", column_list(&spec.primary_key)));
    }
    lines.extend(spec.indexes.iter().map(index_definition));
    lines.extend(spec.foreign_keys.iter().map(|key| foreign_key_definition(key, &spec.schema)));

    let mut sql = format!("CREATE TABLE {} (\n  {}\n)", qualified(&spec.schema, &spec.name), lines.join(",\n  "));
    if let Some(engine) = spec.engine.as_deref().filter(|e| !e.is_empty()) {
        sql.push_str(&format!(" ENGINE = {engine}"));
    }
    if let Some(collation) = spec.collation.as_deref().filter(|c| !c.is_empty()) {
        sql.push_str(&format!(" COLLATE = {collation}"));
    }
    if !spec.comment.is_empty() {
        sql.push_str(&format!(" COMMENT = {}", text_literal(&spec.comment)));
    }
    sql.push(';');
    Ok(sql)
}

/// The SQL that turns `original` into `updated`. Fails if nothing changed.
///
/// This is one `ALTER TABLE`, except when foreign keys are removed or
/// changed: those are dropped in a statement of their own first. Servers do
/// not reliably accept dropping a foreign key and adding one of the same
/// name, or dropping a column a foreign key uses, in a single statement.
pub fn alter_table(original: &TableSpec, updated: &TableSpec) -> Result<String, Error> {
    validate(updated)?;
    // Renamed columns are compared by the name they have on the server.
    let server_name = |name: &String| -> String {
        updated
            .columns
            .iter()
            .find(|c| c.name.trim() == name.trim())
            .and_then(|c| c.original_name.clone())
            .unwrap_or_else(|| name.trim().to_string())
    };
    let on_server = |names: &[String]| names.iter().map(server_name).collect::<Vec<_>>();
    let involves_new_column = |names: &[String]| {
        names.iter().any(|name| updated.columns.iter().any(|c| c.name.trim() == name.trim() && c.original_name.is_none()))
    };

    let mut clauses = Vec::new();

    // Foreign keys that go away or change.
    let mut key_drops = Vec::new();
    let mut key_adds = Vec::new();
    let own_schema = original.schema.as_str();
    let referenced = |key: &ForeignKeySpec| {
        let schema = if key.referenced_schema.trim().is_empty() { own_schema } else { key.referenced_schema.trim() };
        (
            schema.to_string(),
            key.referenced_table.trim().to_string(),
            key.referenced_columns.iter().map(|c| c.trim().to_string()).collect::<Vec<_>>(),
        )
    };
    for key in &original.foreign_keys {
        match updated.foreign_keys.iter().find(|k| k.original_name.as_deref() == Some(key.name.as_str())) {
            None => key_drops.push(format!("DROP FOREIGN KEY {}", quote_identifier(&key.name))),
            Some(now) => {
                let same = now.name.trim() == key.name
                    && !involves_new_column(&now.columns)
                    && on_server(&now.columns) == key.columns
                    && referenced(now) == referenced(key)
                    && action(&now.on_update) == action(&key.on_update)
                    && action(&now.on_delete) == action(&key.on_delete);
                if !same {
                    key_drops.push(format!("DROP FOREIGN KEY {}", quote_identifier(&key.name)));
                    key_adds.push(format!("ADD {}", foreign_key_definition(now, own_schema)));
                }
            }
        }
    }
    key_adds.extend(
        updated.foreign_keys.iter().filter(|k| k.original_name.is_none()).map(|k| format!("ADD {}", foreign_key_definition(k, own_schema))),
    );

    // Indexes that go away or change are dropped first.
    let mut index_adds = Vec::new();
    for index in &original.indexes {
        if index.locked {
            continue;
        }
        match updated.indexes.iter().find(|i| i.original_name.as_deref() == Some(index.name.as_str())) {
            None => clauses.push(format!("DROP INDEX {}", quote_identifier(&index.name))),
            Some(now) => {
                let same = now.name.trim() == index.name
                    && now.unique == index.unique
                    && !involves_new_column(&now.columns)
                    && on_server(&now.columns) == index.columns;
                if !same {
                    clauses.push(format!("DROP INDEX {}", quote_identifier(&index.name)));
                    index_adds.push(format!("ADD {}", index_definition(now)));
                }
            }
        }
    }
    index_adds.extend(updated.indexes.iter().filter(|i| i.original_name.is_none()).map(|i| format!("ADD {}", index_definition(i))));

    let primary_key_changed = involves_new_column(&updated.primary_key) || on_server(&updated.primary_key) != original.primary_key;
    if primary_key_changed && !original.primary_key.is_empty() {
        clauses.push("DROP PRIMARY KEY".to_string());
    }

    for column in &original.columns {
        let kept = updated.columns.iter().any(|c| c.original_name.as_deref() == Some(column.name.as_str()));
        if !kept {
            clauses.push(format!("DROP COLUMN {}", quote_identifier(&column.name)));
        }
    }

    let mut previous: Option<&str> = None;
    for column in &updated.columns {
        match &column.original_name {
            None => {
                let position = match previous {
                    None => " FIRST".to_string(),
                    Some(name) => format!(" AFTER {}", quote_identifier(name)),
                };
                clauses.push(format!("ADD COLUMN {}{position}", column_definition(column)));
            }
            Some(old) if !column.locked => {
                if let Some(before) = original.columns.iter().find(|c| &c.name == old) {
                    let mut comparable = column.clone();
                    comparable.name = before.name.clone();
                    comparable.original_name = before.original_name.clone();
                    if column.name.trim() != old {
                        clauses.push(format!("CHANGE COLUMN {} {}", quote_identifier(old), column_definition(column)));
                    } else if &comparable != before {
                        clauses.push(format!("MODIFY COLUMN {}", column_definition(column)));
                    }
                }
            }
            Some(_) => {}
        }
        previous = Some(column.name.trim());
    }

    if primary_key_changed && !updated.primary_key.is_empty() {
        clauses.push(format!("ADD PRIMARY KEY ({})", column_list(&updated.primary_key)));
    }
    clauses.extend(index_adds);
    clauses.extend(key_adds);

    if updated.engine != original.engine {
        if let Some(engine) = updated.engine.as_deref().filter(|e| !e.is_empty()) {
            clauses.push(format!("ENGINE = {engine}"));
        }
    }
    if updated.collation != original.collation {
        if let Some(collation) = updated.collation.as_deref().filter(|c| !c.is_empty()) {
            clauses.push(format!("COLLATE = {collation}"));
        }
    }
    if updated.comment != original.comment {
        clauses.push(format!("COMMENT = {}", text_literal(&updated.comment)));
    }
    if updated.name.trim() != original.name {
        clauses.push(format!("RENAME TO {}", qualified(&updated.schema, &updated.name)));
    }

    if clauses.is_empty() && key_drops.is_empty() {
        return Err(fail("Nothing has been changed."));
    }
    let table = qualified(&original.schema, &original.name);
    let mut statements = Vec::new();
    if !key_drops.is_empty() {
        statements.push(format!("ALTER TABLE {table}\n  {};", key_drops.join(",\n  ")));
    }
    if !clauses.is_empty() {
        statements.push(format!("ALTER TABLE {table}\n  {};", clauses.join(",\n  ")));
    }
    Ok(statements.join("\n\n"))
}

pub fn drop_object(kind: ObjectKind, schema: &str, name: &str) -> String {
    format!("DROP {} {};", kind.keyword(), qualified(schema, name))
}

pub fn drop_schema(name: &str) -> String {
    format!("DROP SCHEMA {};", quote_identifier(name))
}

pub fn truncate_table(schema: &str, name: &str) -> String {
    format!("TRUNCATE TABLE {};", qualified(schema, name))
}

pub fn rename_table(schema: &str, name: &str, new_name: &str) -> Result<String, Error> {
    check_name(new_name)?;
    Ok(format!("RENAME TABLE {} TO {};", qualified(schema, name), qualified(schema, new_name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(name: &str, data_type: &str) -> ColumnSpec {
        ColumnSpec { name: name.into(), data_type: data_type.into(), original_name: Some(name.into()), ..ColumnSpec::default() }
    }

    fn orders() -> TableSpec {
        TableSpec {
            schema: "shop".into(),
            name: "orders".into(),
            columns: vec![
                ColumnSpec { auto_increment: true, ..column("id", "INT") },
                ColumnSpec { nullable: true, ..column("note", "VARCHAR(100)") },
                ColumnSpec { default: Some("'open'".into()), ..column("status", "VARCHAR(20)") },
            ],
            primary_key: vec!["id".into()],
            indexes: vec![IndexSpec {
                name: "idx_status".into(),
                columns: vec!["status".into()],
                unique: false,
                original_name: Some("idx_status".into()),
                locked: false,
            }],
            foreign_keys: Vec::new(),
            engine: Some("InnoDB".into()),
            collation: Some("utf8mb4_bin".into()),
            comment: String::new(),
        }
    }

    fn customer_key() -> ForeignKeySpec {
        ForeignKeySpec {
            name: "fk_customer".into(),
            columns: vec!["note".into()],
            referenced_schema: "shop".into(),
            referenced_table: "customers".into(),
            referenced_columns: vec!["id".into()],
            on_update: "NO ACTION".into(),
            on_delete: "CASCADE".into(),
            original_name: Some("fk_customer".into()),
        }
    }

    #[test]
    fn create_table_writes_foreign_keys() {
        let mut spec = orders();
        spec.foreign_keys = vec![
            customer_key(),
            // No name, same schema, no rules: the server fills those in.
            ForeignKeySpec {
                columns: vec!["status".into()],
                referenced_table: "statuses".into(),
                referenced_columns: vec!["code".into()],
                ..ForeignKeySpec::default()
            },
        ];
        let sql = create_table(&spec).unwrap();
        assert!(sql.contains(
            "CONSTRAINT `fk_customer` FOREIGN KEY (`note`) REFERENCES `shop`.`customers` (`id`) ON DELETE CASCADE ON UPDATE NO ACTION,\n"
        ));
        assert!(sql.contains("  FOREIGN KEY (`status`) REFERENCES `shop`.`statuses` (`code`)\n)"));
    }

    #[test]
    fn foreign_keys_are_checked() {
        let broken = |change: fn(&mut ForeignKeySpec)| {
            let mut spec = orders();
            let mut key = customer_key();
            change(&mut key);
            spec.foreign_keys = vec![key];
            create_table(&spec).unwrap_err().to_string()
        };
        assert!(broken(|k| k.columns.clear()).contains("at least one column"));
        assert!(broken(|k| k.columns = vec!["nope".into()]).contains("does not exist"));
        assert!(broken(|k| k.referenced_table = " ".into()).contains("needs the table it refers to"));
        assert!(broken(|k| k.referenced_columns.clear()).contains("one column in customers for each"));
        assert!(broken(|k| k.on_delete = "CASCADE; DROP TABLE x".into()).contains("is not something a foreign key can do"));
    }

    #[test]
    fn an_untouched_foreign_key_is_left_alone() {
        let mut original = orders();
        original.foreign_keys = vec![customer_key()];
        let mut updated = original.clone();
        // The designer may hand rules back in another case or with the schema left out.
        updated.foreign_keys[0].on_delete = "cascade".into();
        updated.foreign_keys[0].referenced_schema = String::new();
        updated.comment = "x".into();
        assert_eq!(alter_table(&original, &updated).unwrap(), "ALTER TABLE `shop`.`orders`\n  COMMENT = 'x';");

        // Renaming a column the key uses does not rebuild the key; the server carries it over.
        let mut renamed = original.clone();
        renamed.columns[1].name = "customer_id".into();
        renamed.foreign_keys[0].columns = vec!["customer_id".into()];
        assert_eq!(
            alter_table(&original, &renamed).unwrap(),
            "ALTER TABLE `shop`.`orders`\n  CHANGE COLUMN `note` `customer_id` VARCHAR(100) NULL;"
        );
    }

    #[test]
    fn changed_and_removed_foreign_keys_are_dropped_in_their_own_statement_first() {
        let mut original = orders();
        original.foreign_keys =
            vec![customer_key(), ForeignKeySpec { name: "fk_old".into(), original_name: Some("fk_old".into()), ..customer_key() }];
        let mut updated = original.clone();
        updated.foreign_keys.remove(1);
        updated.foreign_keys[0].on_delete = "SET NULL".into();
        updated.foreign_keys.push(ForeignKeySpec {
            columns: vec!["status".into()],
            referenced_table: "statuses".into(),
            referenced_columns: vec!["code".into()],
            ..ForeignKeySpec::default()
        });
        assert_eq!(
            alter_table(&original, &updated).unwrap(),
            "ALTER TABLE `shop`.`orders`\n  \
             DROP FOREIGN KEY `fk_customer`,\n  \
             DROP FOREIGN KEY `fk_old`;\n\n\
             ALTER TABLE `shop`.`orders`\n  \
             ADD CONSTRAINT `fk_customer` FOREIGN KEY (`note`) REFERENCES `shop`.`customers` (`id`) ON DELETE SET NULL ON UPDATE NO ACTION,\n  \
             ADD FOREIGN KEY (`status`) REFERENCES `shop`.`statuses` (`code`);"
        );

        // Only removing: a single statement.
        let mut removed = original.clone();
        removed.foreign_keys.clear();
        assert_eq!(
            alter_table(&original, &removed).unwrap(),
            "ALTER TABLE `shop`.`orders`\n  DROP FOREIGN KEY `fk_customer`,\n  DROP FOREIGN KEY `fk_old`;"
        );
    }

    #[test]
    fn create_table_writes_columns_keys_and_options() {
        let mut spec = orders();
        spec.comment = "Customer's orders".into();
        assert_eq!(
            create_table(&spec).unwrap(),
            "CREATE TABLE `shop`.`orders` (\n  \
             `id` INT NOT NULL AUTO_INCREMENT,\n  \
             `note` VARCHAR(100) NULL,\n  \
             `status` VARCHAR(20) NOT NULL DEFAULT 'open',\n  \
             PRIMARY KEY (`id`),\n  \
             INDEX `idx_status` (`status`)\n\
             ) ENGINE = InnoDB COLLATE = utf8mb4_bin COMMENT = 'Customer''s orders';"
        );
    }

    #[test]
    fn create_table_checks_the_spec() {
        let broken = |change: fn(&mut TableSpec)| {
            let mut spec = orders();
            change(&mut spec);
            create_table(&spec).unwrap_err().to_string()
        };
        assert!(broken(|s| s.columns.clear()).contains("at least one column"));
        assert!(broken(|s| s.columns[1].name = "ID".into()).contains("two columns named"));
        assert!(broken(|s| s.columns[1].data_type = " ".into()).contains("needs a type"));
        assert!(broken(|s| s.columns[1].data_type = "INT; DROP TABLE x".into()).contains("not valid"));
        assert!(broken(|s| s.primary_key = vec!["nope".into()]).contains("does not exist"));
        assert!(broken(|s| s.indexes[0].columns.clear()).contains("at least one column"));
        assert!(broken(|s| s.engine = Some("Inno DB".into())).contains("not a valid engine"));
        assert!(broken(|s| s.name = String::new()).contains("Enter a name"));
    }

    #[test]
    fn unchanged_spec_is_refused() {
        assert!(alter_table(&orders(), &orders()).unwrap_err().to_string().contains("Nothing has been changed"));
    }

    #[test]
    fn alter_adds_drops_renames_and_modifies_columns() {
        let original = orders();
        let mut updated = orders();
        updated.columns.remove(1); // drop note
        updated.columns[1].name = "state".into(); // rename status
        updated.columns[0].data_type = "BIGINT".into(); // modify id
        updated.columns.insert(1, ColumnSpec { name: "total".into(), data_type: "DECIMAL(10,2)".into(), ..ColumnSpec::default() });
        updated.indexes[0].columns = vec!["state".into()];

        assert_eq!(
            alter_table(&original, &updated).unwrap(),
            "ALTER TABLE `shop`.`orders`\n  \
             DROP COLUMN `note`,\n  \
             MODIFY COLUMN `id` BIGINT NOT NULL AUTO_INCREMENT,\n  \
             ADD COLUMN `total` DECIMAL(10,2) NOT NULL AFTER `id`,\n  \
             CHANGE COLUMN `status` `state` VARCHAR(20) NOT NULL DEFAULT 'open';",
            "the index follows the renamed column, so it is not rebuilt"
        );
    }

    #[test]
    fn alter_changes_keys_indexes_and_options() {
        let original = orders();
        let mut updated = orders();
        updated.primary_key = vec!["id".into(), "status".into()];
        updated.indexes[0].unique = true;
        updated.indexes.push(IndexSpec { name: "idx_note".into(), columns: vec!["note".into()], ..IndexSpec::default() });
        updated.engine = Some("MyISAM".into());
        updated.comment = "x".into();
        updated.name = "orders_old".into();

        assert_eq!(
            alter_table(&original, &updated).unwrap(),
            "ALTER TABLE `shop`.`orders`\n  \
             DROP INDEX `idx_status`,\n  \
             DROP PRIMARY KEY,\n  \
             ADD PRIMARY KEY (`id`, `status`),\n  \
             ADD UNIQUE INDEX `idx_status` (`status`),\n  \
             ADD INDEX `idx_note` (`note`),\n  \
             ENGINE = MyISAM,\n  \
             COMMENT = 'x',\n  \
             RENAME TO `shop`.`orders_old`;"
        );
    }

    #[test]
    fn locked_columns_and_indexes_are_never_touched() {
        let mut original = orders();
        original.columns[1].locked = true;
        original.indexes[0].locked = true;
        let mut updated = original.clone();
        updated.columns[1].data_type = "TEXT".into();
        updated.indexes.clear();
        assert!(alter_table(&original, &updated).is_err(), "the only edits were to locked parts");
    }

    #[test]
    fn a_first_new_column_goes_first() {
        let original = orders();
        let mut updated = orders();
        updated.columns.insert(0, ColumnSpec { name: "tenant".into(), data_type: "INT".into(), ..ColumnSpec::default() });
        assert!(alter_table(&original, &updated).unwrap().contains("ADD COLUMN `tenant` INT NOT NULL FIRST"));
    }

    #[test]
    fn drop_truncate_and_rename_quote_names() {
        assert_eq!(drop_object(ObjectKind::Table, "shop", "we`ird"), "DROP TABLE `shop`.`we``ird`;");
        assert_eq!(drop_object(ObjectKind::Procedure, "shop", "p"), "DROP PROCEDURE `shop`.`p`;");
        assert_eq!(drop_schema("shop"), "DROP SCHEMA `shop`;");
        assert_eq!(truncate_table("shop", "t"), "TRUNCATE TABLE `shop`.`t`;");
        assert_eq!(rename_table("shop", "t", "u").unwrap(), "RENAME TABLE `shop`.`t` TO `shop`.`u`;");
        assert!(rename_table("shop", "t", " ").is_err());
    }
}
