//! Table structure as the UI edits it, and the planner that turns "old design → new design"
//! into DDL for each dialect. The UI never builds DDL itself.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::dialect::Dialect;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnDesign {
    /// Name in the database; `None` for a column being added.
    pub original: Option<String>,
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    /// Default as a SQL expression (`0`, `'pending'`, `now()`), not a value.
    pub default: Option<String>,
    pub primary_key: bool,
    /// Postgres identity column / MySQL AUTO_INCREMENT.
    pub auto_increment: bool,
    pub comment: Option<String>,
    /// Computed column; shown but never written to or redefined.
    #[serde(default)]
    pub generated: bool,
    /// MySQL-only clauses that must survive a MODIFY, e.g. `ON UPDATE CURRENT_TIMESTAMP`.
    #[serde(default)]
    pub extra: Option<String>,
    /// Allowed values when the column's type is an enum; informational, never diffed.
    #[serde(default)]
    pub enum_values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexDesign {
    pub original: Option<String>,
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    /// Postgres: the index backs a UNIQUE constraint and must be dropped as a constraint.
    #[serde(default)]
    pub is_constraint: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FkAction {
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

impl FkAction {
    fn sql(self) -> &'static str {
        match self {
            FkAction::NoAction => "NO ACTION",
            FkAction::Restrict => "RESTRICT",
            FkAction::Cascade => "CASCADE",
            FkAction::SetNull => "SET NULL",
            FkAction::SetDefault => "SET DEFAULT",
        }
    }

    pub(crate) fn parse(s: &str) -> FkAction {
        match s.trim().to_ascii_uppercase().as_str() {
            "R" | "RESTRICT" => FkAction::Restrict,
            "C" | "CASCADE" => FkAction::Cascade,
            "N" | "SET NULL" => FkAction::SetNull,
            "D" | "SET DEFAULT" => FkAction::SetDefault,
            _ => FkAction::NoAction,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKeyDesign {
    pub original: Option<String>,
    pub name: String,
    pub columns: Vec<String>,
    pub ref_schema: Option<String>,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
    pub on_delete: FkAction,
    pub on_update: FkAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableDesign {
    pub name: String,
    pub columns: Vec<ColumnDesign>,
    pub indexes: Vec<IndexDesign>,
    pub foreign_keys: Vec<ForeignKeyDesign>,
    /// Name of the existing primary key constraint (Postgres needs it to drop the key).
    #[serde(default)]
    pub primary_key_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableDetails {
    pub schema: Option<String>,
    pub design: TableDesign,
    pub is_view: bool,
    /// Planner statistics, not an exact count.
    pub row_estimate: Option<i64>,
}

impl TableDesign {
    pub fn primary_key(&self) -> Vec<String> {
        self.columns.iter().filter(|c| c.primary_key).map(|c| c.name.clone()).collect()
    }
}

/// Problems the user must fix before any SQL is generated.
pub fn validate(design: &TableDesign) -> Result<(), String> {
    if design.name.trim().is_empty() {
        return Err("The table needs a name.".into());
    }
    if design.columns.is_empty() {
        return Err("A table needs at least one column.".into());
    }
    let mut seen = HashSet::new();
    for c in &design.columns {
        if c.name.trim().is_empty() {
            return Err("Every column needs a name.".into());
        }
        if c.data_type.trim().is_empty() {
            return Err(format!("Column \"{}\" needs a type.", c.name));
        }
        if !seen.insert(c.name.as_str()) {
            return Err(format!("There is more than one column named \"{}\".", c.name));
        }
    }
    for i in &design.indexes {
        if i.name.trim().is_empty() {
            return Err("Every index needs a name.".into());
        }
        if i.columns.is_empty() {
            return Err(format!("Index \"{}\" has no columns.", i.name));
        }
        if let Some(missing) = i.columns.iter().find(|c| !seen.contains(c.as_str())) {
            return Err(format!("Index \"{}\" uses a column that does not exist: {missing}", i.name));
        }
    }
    for f in &design.foreign_keys {
        if f.name.trim().is_empty() || f.ref_table.trim().is_empty() {
            return Err("A relationship needs a name and a target table.".into());
        }
        if f.columns.is_empty() || f.columns.len() != f.ref_columns.len() {
            return Err(format!("Relationship \"{}\" must link the same number of columns on both sides.", f.name));
        }
        if let Some(missing) = f.columns.iter().find(|c| !seen.contains(c.as_str())) {
            return Err(format!("Relationship \"{}\" uses a column that does not exist: {missing}", f.name));
        }
    }
    Ok(())
}

fn column_def(d: Dialect, c: &ColumnDesign) -> String {
    let mut sql = format!("{} {}", d.ident(&c.name), c.data_type.trim());
    if d.is_sqlite() {
        // SQLite's auto-increment is an INTEGER PRIMARY KEY declared on the column itself.
        if c.auto_increment {
            return format!("{} INTEGER PRIMARY KEY AUTOINCREMENT", d.ident(&c.name));
        }
        if !c.nullable {
            sql.push_str(" NOT NULL");
        }
        if let Some(def) = &c.default {
            // Expressions must be parenthesised; literals and CURRENT_* keywords needn't be.
            let simple = def.starts_with('\'') || def.parse::<f64>().is_ok() || def.to_ascii_uppercase().starts_with("CURRENT_") || def.starts_with('(');
            sql.push_str(&if simple { format!(" DEFAULT {def}") } else { format!(" DEFAULT ({def})") });
        }
        return sql;
    }
    if d.is_mysql() {
        sql.push_str(if c.nullable { " NULL" } else { " NOT NULL" });
        if let Some(def) = c.default.as_deref().filter(|_| !c.auto_increment) {
            sql.push_str(&format!(" DEFAULT {def}"));
        }
        if c.auto_increment {
            sql.push_str(" AUTO_INCREMENT");
        }
        if let Some(extra) = &c.extra {
            sql.push_str(&format!(" {extra}"));
        }
        if let Some(comment) = c.comment.as_deref().filter(|s| !s.is_empty()) {
            sql.push_str(&format!(" COMMENT {}", d.string(comment)));
        }
    } else {
        if c.auto_increment {
            sql.push_str(" GENERATED BY DEFAULT AS IDENTITY");
        }
        if !c.nullable && !c.auto_increment {
            sql.push_str(" NOT NULL");
        }
        if let Some(def) = c.default.as_deref().filter(|_| !c.auto_increment) {
            sql.push_str(&format!(" DEFAULT {def}"));
        }
    }
    sql
}

fn fk_def(d: Dialect, schema: Option<&str>, f: &ForeignKeyDesign) -> String {
    let ref_schema = f.ref_schema.as_deref().or(schema);
    format!(
        "CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({}) ON DELETE {} ON UPDATE {}",
        d.ident(&f.name),
        d.ident_list(&f.columns),
        d.table(ref_schema, &f.ref_table),
        d.ident_list(&f.ref_columns),
        f.on_delete.sql(),
        f.on_update.sql()
    )
}

fn create_index(d: Dialect, schema: Option<&str>, table: &str, i: &IndexDesign) -> String {
    format!(
        "CREATE {}INDEX {} ON {} ({})",
        if i.unique { "UNIQUE " } else { "" },
        d.ident(&i.name),
        d.table(schema, table),
        d.ident_list(&i.columns)
    )
}

fn pg_comment(d: Dialect, table: &str, c: &ColumnDesign) -> String {
    let value = c.comment.as_deref().filter(|s| !s.is_empty()).map(|s| d.string(s)).unwrap_or_else(|| "NULL".into());
    format!("COMMENT ON COLUMN {}.{} IS {value}", table, d.ident(&c.name))
}

pub fn plan_create(d: Dialect, schema: Option<&str>, design: &TableDesign) -> Result<Vec<String>, String> {
    validate(design)?;
    let table = d.table(schema, &design.name);
    let mut parts: Vec<String> = design.columns.iter().map(|c| column_def(d, c)).collect();
    let pk = design.primary_key();
    let inline_pk = d.is_sqlite() && design.columns.iter().any(|c| c.auto_increment);
    if d.is_sqlite() && design.columns.iter().any(|c| c.auto_increment) && pk.len() > 1 {
        return Err("SQLite's auto-increment column must be the only primary key column.".into());
    }
    if !pk.is_empty() && !inline_pk {
        parts.push(format!("PRIMARY KEY ({})", d.ident_list(&pk)));
    }
    parts.extend(design.foreign_keys.iter().map(|f| fk_def(d, schema, f)));

    let mut out = vec![format!("CREATE TABLE {table} (\n  {}\n)", parts.join(",\n  "))];
    out.extend(design.indexes.iter().map(|i| create_index(d, schema, &design.name, i)));
    if d.kind == crate::config::DbKind::Postgres {
        out.extend(design.columns.iter().filter(|c| c.comment.as_deref().is_some_and(|s| !s.is_empty())).map(|c| pg_comment(d, &table, c)));
    }
    Ok(out)
}

/// SQLite can rename tables and columns, add and drop columns, and manage indexes —
/// but not change an existing column or its constraints in place.
fn plan_alter_sqlite(d: Dialect, schema: Option<&str>, old: &TableDesign, new: &TableDesign) -> Result<Vec<String>, String> {
    let table = d.table(schema, &old.name);
    let olds: HashMap<&str, &ColumnDesign> = old.columns.iter().map(|c| (c.name.as_str(), c)).collect();
    for c in &new.columns {
        let Some(o) = c.original.as_deref().and_then(|n| olds.get(n)) else {
            if c.primary_key || c.auto_increment {
                return Err(format!("SQLite can't add a primary key column (\"{}\") to an existing table.", c.name));
            }
            if !c.nullable && c.default.is_none() {
                return Err(format!("In SQLite a new required column (\"{}\") needs a default value.", c.name));
            }
            continue;
        };
        if o.data_type != c.data_type || o.nullable != c.nullable || o.default != c.default || o.primary_key != c.primary_key || o.auto_increment != c.auto_increment {
            return Err(format!("SQLite can't change the type, rules or default of an existing column (\"{}\"). Rename, add or remove columns instead.", o.name));
        }
    }
    let renamed = |cols: &[String]| -> Vec<String> {
        cols.iter().map(|c| new.columns.iter().find(|n| n.original.as_deref() == Some(c)).map(|n| n.name.clone()).unwrap_or_else(|| c.clone())).collect()
    };
    let same_fks = old.foreign_keys.len() == new.foreign_keys.len()
        && old.foreign_keys.iter().zip(&new.foreign_keys).all(|(o, n)| renamed(&o.columns) == n.columns && o.ref_table == n.ref_table && o.ref_columns == n.ref_columns && o.on_delete == n.on_delete);
    if !same_fks {
        return Err("SQLite can't add or remove relationships on an existing table.".into());
    }

    let mut out = Vec::new();
    let kept = |i: &IndexDesign| new.indexes.iter().any(|n| n.original.as_deref() == Some(&i.name) && n.unique == i.unique && renamed(&i.columns) == n.columns);
    for i in old.indexes.iter().filter(|i| !kept(i)) {
        if i.is_constraint {
            return Err(format!("SQLite can't remove the uniqueness rule \"{}\" from an existing table.", i.name));
        }
        out.push(format!("DROP INDEX {}", d.table(schema, &i.name)));
    }
    let survivors: Vec<&str> = new.columns.iter().filter_map(|c| c.original.as_deref()).collect();
    for c in old.columns.iter().filter(|c| !survivors.contains(&c.name.as_str())) {
        out.push(format!("ALTER TABLE {table} DROP COLUMN {}", d.ident(&c.name)));
    }
    for c in &new.columns {
        match c.original.as_deref() {
            Some(o) if o != c.name => out.push(format!("ALTER TABLE {table} RENAME COLUMN {} TO {}", d.ident(o), d.ident(&c.name))),
            Some(_) => {}
            None => out.push(format!("ALTER TABLE {table} ADD COLUMN {}", column_def(d, c))),
        }
    }
    let old_names: Vec<&str> = old.indexes.iter().filter(|i| kept(i)).map(|i| i.name.as_str()).collect();
    for i in new.indexes.iter().filter(|i| !i.original.as_deref().is_some_and(|o| old_names.contains(&o))) {
        out.push(create_index(d, schema, &old.name, i));
    }
    if new.name != old.name {
        out.push(format!("ALTER TABLE {table} RENAME TO {}", d.ident(&new.name)));
    }
    Ok(out)
}

/// DDL that turns `old` into `new`. Statements are ordered so each one is valid given
/// the ones before it: constraints are dropped first and recreated last, and a table
/// rename comes at the very end so every other statement can use the old name.
pub fn plan_alter(d: Dialect, schema: Option<&str>, old: &TableDesign, new: &TableDesign) -> Result<Vec<String>, String> {
    validate(new)?;
    if d.is_sqlite() {
        return plan_alter_sqlite(d, schema, old, new);
    }
    let table = d.table(schema, &old.name);
    let alter = |clause: String| format!("ALTER TABLE {table} {clause}");
    let mut out = Vec::new();

    // Old column name → new name, for columns that survive.
    let renames: HashMap<&str, &str> =
        new.columns.iter().filter_map(|c| c.original.as_deref().map(|o| (o, c.name.as_str()))).collect();
    let map_cols = |cols: &[String]| -> Option<Vec<String>> {
        cols.iter().map(|c| renames.get(c.as_str()).map(|n| n.to_string())).collect()
    };

    // Constraints whose definition changed are dropped and recreated.
    let kept_fk = |f: &ForeignKeyDesign| {
        new.foreign_keys.iter().any(|n| {
            n.original.as_deref() == Some(&f.name)
                && n.name == f.name
                && map_cols(&f.columns).as_ref() == Some(&n.columns)
                && n.ref_schema == f.ref_schema
                && n.ref_table == f.ref_table
                && n.ref_columns == f.ref_columns
                && n.on_delete == f.on_delete
                && n.on_update == f.on_update
        })
    };
    let kept_index = |i: &IndexDesign| {
        new.indexes.iter().any(|n| {
            n.original.as_deref() == Some(&i.name)
                && n.name == i.name
                && n.unique == i.unique
                && map_cols(&i.columns).as_ref() == Some(&n.columns)
        })
    };

    for f in old.foreign_keys.iter().filter(|f| !kept_fk(f)) {
        out.push(if d.is_mysql() {
            alter(format!("DROP FOREIGN KEY {}", d.ident(&f.name)))
        } else {
            alter(format!("DROP CONSTRAINT {}", d.ident(&f.name)))
        });
    }
    for i in old.indexes.iter().filter(|i| !kept_index(i)) {
        out.push(if d.is_mysql() {
            format!("DROP INDEX {} ON {table}", d.ident(&i.name))
        } else if i.is_constraint {
            alter(format!("DROP CONSTRAINT {}", d.ident(&i.name)))
        } else {
            format!("DROP INDEX {}", d.table(schema, &i.name))
        });
    }

    let old_pk = old.primary_key();
    let new_pk = new.primary_key();
    let pk_changed = map_cols(&old_pk).as_ref() != Some(&new_pk);
    if pk_changed && !old_pk.is_empty() {
        out.push(if d.is_mysql() {
            alter("DROP PRIMARY KEY".into())
        } else {
            let name = old.primary_key_name.clone().unwrap_or_else(|| format!("{}_pkey", old.name));
            alter(format!("DROP CONSTRAINT {}", d.ident(&name)))
        });
    }

    let survivors: HashSet<&str> = renames.keys().copied().collect();
    for c in old.columns.iter().filter(|c| !survivors.contains(c.name.as_str())) {
        out.push(alter(format!("DROP COLUMN {}", d.ident(&c.name))));
    }

    let olds: HashMap<&str, &ColumnDesign> = old.columns.iter().map(|c| (c.name.as_str(), c)).collect();
    let mut previous: Option<&str> = None;
    for c in &new.columns {
        match c.original.as_deref().and_then(|o| olds.get(o).copied()) {
            Some(o) if !c.generated => {
                if d.is_mysql() {
                    let same = o.name == c.name
                        && o.data_type == c.data_type
                        && o.nullable == c.nullable
                        && o.default == c.default
                        && o.auto_increment == c.auto_increment
                        && o.comment == c.comment;
                    if !same {
                        out.push(if o.name != c.name {
                            alter(format!("CHANGE COLUMN {} {}", d.ident(&o.name), column_def(d, c)))
                        } else {
                            alter(format!("MODIFY COLUMN {}", column_def(d, c)))
                        });
                    }
                } else {
                    let col = d.ident(&c.name);
                    if o.name != c.name {
                        out.push(alter(format!("RENAME COLUMN {} TO {col}", d.ident(&o.name))));
                    }
                    if o.data_type != c.data_type {
                        let ty = c.data_type.trim();
                        out.push(alter(format!("ALTER COLUMN {col} TYPE {ty} USING {col}::{ty}")));
                    }
                    if o.auto_increment != c.auto_increment {
                        out.push(alter(if c.auto_increment {
                            format!("ALTER COLUMN {col} ADD GENERATED BY DEFAULT AS IDENTITY")
                        } else {
                            format!("ALTER COLUMN {col} DROP IDENTITY IF EXISTS")
                        }));
                    }
                    if o.nullable != c.nullable {
                        out.push(alter(format!("ALTER COLUMN {col} {} NOT NULL", if c.nullable { "DROP" } else { "SET" })));
                    }
                    if o.default != c.default && !c.auto_increment {
                        out.push(alter(match &c.default {
                            Some(def) => format!("ALTER COLUMN {col} SET DEFAULT {def}"),
                            None => format!("ALTER COLUMN {col} DROP DEFAULT"),
                        }));
                    }
                    if o.comment != c.comment {
                        out.push(pg_comment(d, &table, c));
                    }
                }
            }
            Some(_) => {}
            None => {
                let mut clause = format!("ADD COLUMN {}", column_def(d, c));
                if d.is_mysql() {
                    clause.push_str(&match previous {
                        Some(p) => format!(" AFTER {}", d.ident(p)),
                        None => " FIRST".into(),
                    });
                }
                out.push(alter(clause));
                if !d.is_mysql() && c.comment.as_deref().is_some_and(|s| !s.is_empty()) {
                    out.push(pg_comment(d, &table, c));
                }
            }
        }
        previous = Some(&c.name);
    }

    if pk_changed && !new_pk.is_empty() {
        out.push(alter(format!("ADD PRIMARY KEY ({})", d.ident_list(&new_pk))));
    }

    let old_index_names: HashSet<&str> =
        old.indexes.iter().filter(|i| kept_index(i)).map(|i| i.name.as_str()).collect();
    for i in new.indexes.iter().filter(|i| !i.original.as_deref().is_some_and(|o| old_index_names.contains(o))) {
        out.push(create_index(d, schema, &old.name, i));
    }
    let old_fk_names: HashSet<&str> =
        old.foreign_keys.iter().filter(|f| kept_fk(f)).map(|f| f.name.as_str()).collect();
    for f in new.foreign_keys.iter().filter(|f| !f.original.as_deref().is_some_and(|o| old_fk_names.contains(o))) {
        out.push(alter(format!("ADD {}", fk_def(d, schema, f))));
    }

    if new.name != old.name {
        out.push(if d.is_mysql() {
            format!("RENAME TABLE {table} TO {}", d.table(schema, &new.name))
        } else {
            alter(format!("RENAME TO {}", d.ident(&new.name)))
        });
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TableAction {
    Drop,
    Truncate,
    Rename { to: String },
}

pub fn plan_action(d: Dialect, schema: Option<&str>, table: &str, is_view: bool, action: &TableAction) -> Vec<String> {
    let t = d.table(schema, table);
    vec![match action {
        TableAction::Drop if is_view => format!("DROP VIEW {t}"),
        TableAction::Drop => format!("DROP TABLE {t}"),
        TableAction::Truncate if d.is_sqlite() => format!("DELETE FROM {t}"),
        TableAction::Truncate => format!("TRUNCATE TABLE {t}"),
        TableAction::Rename { to } if d.is_mysql() => format!("RENAME TABLE {t} TO {}", d.table(schema, to)),
        TableAction::Rename { to } => {
            format!("ALTER {} {t} RENAME TO {}", if is_view { "VIEW" } else { "TABLE" }, d.ident(to))
        }
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, ty: &str) -> ColumnDesign {
        ColumnDesign {
            original: Some(name.into()),
            name: name.into(),
            data_type: ty.into(),
            nullable: true,
            default: None,
            primary_key: false,
            auto_increment: false,
            comment: None,
            generated: false,
            extra: None,
            enum_values: vec![],
        }
    }

    fn base() -> TableDesign {
        TableDesign {
            name: "users".into(),
            columns: vec![
                ColumnDesign { primary_key: true, auto_increment: true, nullable: false, ..col("id", "bigint") },
                col("email", "text"),
                col("age", "integer"),
            ],
            indexes: vec![IndexDesign {
                original: Some("users_email_idx".into()),
                name: "users_email_idx".into(),
                columns: vec!["email".into()],
                unique: true,
                is_constraint: false,
            }],
            foreign_keys: vec![],
            primary_key_name: Some("users_pkey".into()),
        }
    }

    const PG: Dialect = Dialect::POSTGRES;
    const MY: Dialect = Dialect::MYSQL;

    #[test]
    fn create_table_postgres() {
        let mut d = base();
        for c in &mut d.columns {
            c.original = None;
        }
        d.indexes[0].original = None;
        d.columns[1].nullable = false;
        d.columns[2].default = Some("18".into());
        let sql = plan_create(PG, Some("public"), &d).unwrap();
        assert_eq!(
            sql[0],
            "CREATE TABLE \"public\".\"users\" (\n  \"id\" bigint GENERATED BY DEFAULT AS IDENTITY,\n  \"email\" text NOT NULL,\n  \"age\" integer DEFAULT 18,\n  PRIMARY KEY (\"id\")\n)"
        );
        assert_eq!(sql[1], "CREATE UNIQUE INDEX \"users_email_idx\" ON \"public\".\"users\" (\"email\")");
    }

    #[test]
    fn create_table_mysql() {
        let d = base();
        let sql = plan_create(MY, None, &d).unwrap();
        assert!(sql[0].contains("`id` bigint NOT NULL AUTO_INCREMENT"), "{}", sql[0]);
        assert!(sql[0].contains("`email` text NULL"));
    }

    #[test]
    fn unchanged_design_produces_nothing() {
        assert!(plan_alter(PG, None, &base(), &base()).unwrap().is_empty());
        assert!(plan_alter(MY, None, &base(), &base()).unwrap().is_empty());
    }

    #[test]
    fn alter_postgres_column_changes() {
        let old = base();
        let mut new = base();
        new.columns[1].name = "mail".into();
        new.columns[1].nullable = false;
        new.columns[2].data_type = "bigint".into();
        new.columns[2].default = Some("0".into());
        new.columns.push(ColumnDesign { original: None, ..col("bio", "text") });
        new.indexes[0].columns = vec!["mail".into()]; // follows the rename, so it's unchanged
        let sql = plan_alter(PG, Some("public"), &old, &new).unwrap();
        let t = "ALTER TABLE \"public\".\"users\"";
        assert_eq!(
            sql,
            vec![
                format!("{t} RENAME COLUMN \"email\" TO \"mail\""),
                format!("{t} ALTER COLUMN \"mail\" SET NOT NULL"),
                format!("{t} ALTER COLUMN \"age\" TYPE bigint USING \"age\"::bigint"),
                format!("{t} ALTER COLUMN \"age\" SET DEFAULT 0"),
                format!("{t} ADD COLUMN \"bio\" text"),
            ]
        );
    }

    #[test]
    fn alter_mysql_uses_full_definitions() {
        let old = base();
        let mut new = base();
        new.columns[1].name = "mail".into();
        new.columns[2].default = Some("0".into());
        new.columns[2].extra = Some("ON UPDATE CURRENT_TIMESTAMP".into()); // carried, not diffed
        new.columns.insert(1, ColumnDesign { original: None, ..col("bio", "text") });
        new.indexes[0].columns = vec!["mail".into()];
        let sql = plan_alter(MY, None, &old, &new).unwrap();
        assert_eq!(
            sql,
            vec![
                "ALTER TABLE `users` ADD COLUMN `bio` text NULL AFTER `id`",
                "ALTER TABLE `users` CHANGE COLUMN `email` `mail` text NULL",
                "ALTER TABLE `users` MODIFY COLUMN `age` integer NULL DEFAULT 0 ON UPDATE CURRENT_TIMESTAMP",
            ]
        );
    }

    #[test]
    fn drops_come_first_and_rename_last() {
        let old = base();
        let mut new = base();
        new.name = "people".into();
        new.columns.remove(2);
        new.indexes[0].unique = false; // changed → drop + recreate
        new.columns[1].primary_key = true; // pk becomes (id, email)
        let sql = plan_alter(PG, None, &old, &new).unwrap();
        assert_eq!(
            sql,
            vec![
                "DROP INDEX \"users_email_idx\"",
                "ALTER TABLE \"users\" DROP CONSTRAINT \"users_pkey\"",
                "ALTER TABLE \"users\" DROP COLUMN \"age\"",
                "ALTER TABLE \"users\" ADD PRIMARY KEY (\"id\", \"email\")",
                "CREATE INDEX \"users_email_idx\" ON \"users\" (\"email\")",
                "ALTER TABLE \"users\" RENAME TO \"people\"",
            ]
        );
    }

    #[test]
    fn foreign_keys() {
        let old = base();
        let mut new = base();
        new.foreign_keys.push(ForeignKeyDesign {
            original: None,
            name: "users_team_fk".into(),
            columns: vec!["age".into()],
            ref_schema: None,
            ref_table: "teams".into(),
            ref_columns: vec!["id".into()],
            on_delete: FkAction::Cascade,
            on_update: FkAction::NoAction,
        });
        let sql = plan_alter(MY, Some("shop"), &old, &new).unwrap();
        assert_eq!(
            sql,
            vec!["ALTER TABLE `shop`.`users` ADD CONSTRAINT `users_team_fk` FOREIGN KEY (`age`) REFERENCES `shop`.`teams` (`id`) ON DELETE CASCADE ON UPDATE NO ACTION"]
        );
        let back = plan_alter(MY, Some("shop"), &new_with_original(new), &old).unwrap();
        assert_eq!(back, vec!["ALTER TABLE `shop`.`users` DROP FOREIGN KEY `users_team_fk`"]);
    }

    fn new_with_original(mut d: TableDesign) -> TableDesign {
        for f in &mut d.foreign_keys {
            f.original = Some(f.name.clone());
        }
        d
    }

    #[test]
    fn sqlite_create_and_supported_alters() {
        const SQ: Dialect = Dialect::SQLITE;
        let mut d = base();
        for c in &mut d.columns {
            c.original = None;
        }
        d.columns[1].default = Some("lower('X')".into());
        let sql = plan_create(SQ, Some("main"), &d).unwrap();
        assert_eq!(sql[0], "CREATE TABLE \"main\".\"users\" (\n  \"id\" INTEGER PRIMARY KEY AUTOINCREMENT,\n  \"email\" text DEFAULT (lower('X')),\n  \"age\" integer\n)");

        let old = base();
        let mut new = base();
        new.columns[1].name = "mail".into();
        new.indexes[0].columns = vec!["mail".into()];
        new.columns.remove(2);
        new.columns.push(ColumnDesign { original: None, ..col("bio", "text") });
        let sql = plan_alter(SQ, None, &old, &new).unwrap();
        assert_eq!(
            sql,
            vec![
                "ALTER TABLE \"users\" DROP COLUMN \"age\"",
                "ALTER TABLE \"users\" RENAME COLUMN \"email\" TO \"mail\"",
                "ALTER TABLE \"users\" ADD COLUMN \"bio\" text",
            ]
        );
        let mut retyped = base();
        retyped.columns[2].data_type = "bigint".into();
        assert!(plan_alter(SQ, None, &old, &retyped).unwrap_err().contains("can't change the type"));
    }

    #[test]
    fn validation_errors() {
        let mut d = base();
        d.columns[2].name = "email".into();
        assert!(plan_alter(PG, None, &base(), &d).unwrap_err().contains("more than one"));
        let mut d = base();
        d.indexes[0].columns = vec!["nope".into()];
        assert!(plan_alter(PG, None, &base(), &d).is_err());
    }
}
