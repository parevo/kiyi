//! Everything in a database besides tables and views: functions and procedures, triggers,
//! sequences, and the users and roles that can sign in. Listed from the catalog, with the SQL
//! that defines each one, the SQL that drops it, and a starting point for creating a new one.
//! Nothing here changes the database: changes go through the SQL editor and its review.

use serde::{Deserialize, Serialize};

use crate::config::DbKind;
use crate::dialect::Dialect;
use crate::drivers::DbDriver;
use crate::error::{Error, Result};
use crate::types::Cell;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObjectKind {
    Function,
    Procedure,
    Trigger,
    Sequence,
    User,
}

impl ObjectKind {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "function" => Self::Function,
            "procedure" => Self::Procedure,
            "trigger" => Self::Trigger,
            "sequence" => Self::Sequence,
            "user" => Self::User,
            _ => return None,
        })
    }

    fn plural(self) -> &'static str {
        match self {
            Self::Function => "functions",
            Self::Procedure => "procedures",
            Self::Trigger => "triggers",
            Self::Sequence => "sequences",
            Self::User => "users and roles",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbObject {
    pub kind: ObjectKind,
    pub schema: Option<String>,
    pub name: String,
    /// Arguments and result, the trigger's table, the sequence's position, or what a user can do.
    pub detail: String,
    /// The catalog's id for it (Postgres oid, SQL Server object or principal id, MySQL grantee).
    #[serde(default)]
    pub key: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectList {
    /// The kinds this database has at all, in display order.
    pub kinds: Vec<ObjectKind>,
    pub objects: Vec<DbObject>,
    /// Kinds that couldn't be read, usually for lack of privileges, and why.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectSource {
    /// SQL that creates it as it is now (for users: their sign-in and privileges).
    pub definition: String,
    pub drop: String,
}

pub fn kinds(kind: DbKind) -> Vec<ObjectKind> {
    use ObjectKind::*;
    match kind {
        DbKind::Sqlite => vec![Trigger],
        _ => vec![Function, Procedure, Trigger, Sequence, User],
    }
}

const PG_SKIP: &str = "('pg_catalog', 'information_schema')";
const MYSQL_SKIP: &str = "('mysql', 'sys', 'information_schema', 'performance_schema')";

/// Catalog queries returning `kind, schema, name, detail, key`, one per section so a section the
/// user may not read doesn't hide the others.
fn list_queries(kind: DbKind) -> Vec<(ObjectKind, String)> {
    use ObjectKind::*;
    match kind {
        DbKind::Postgres => vec![
            (
                Function,
                format!(
                    "SELECT CASE p.prokind WHEN 'p' THEN 'procedure' ELSE 'function' END, n.nspname, p.proname,
                       '(' || pg_get_function_identity_arguments(p.oid) || ')'
                         || CASE WHEN p.prokind = 'p' THEN '' ELSE ' → ' || pg_get_function_result(p.oid) END,
                       p.oid::text
                     FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
                     WHERE n.nspname NOT IN {PG_SKIP} AND n.nspname NOT LIKE 'pg\\_%' AND p.prokind IN ('f', 'p')
                       AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_proc'::regclass AND d.objid = p.oid AND d.deptype = 'e')
                     ORDER BY 2, 3, 4"
                ),
            ),
            (
                Trigger,
                format!(
                    "SELECT 'trigger', n.nspname, t.tgname, 'on ' || c.relname, t.oid::text
                     FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid JOIN pg_namespace n ON n.oid = c.relnamespace
                     WHERE NOT t.tgisinternal AND n.nspname NOT IN {PG_SKIP}
                     ORDER BY 2, 3"
                ),
            ),
            (
                Sequence,
                format!(
                    "SELECT 'sequence', s.schemaname, s.sequencename,
                       concat_ws(' · ',
                         (SELECT 'for ' || c.relname || '.' || a.attname
                          FROM pg_depend d JOIN pg_class c ON c.oid = d.refobjid
                          JOIN pg_attribute a ON a.attrelid = d.refobjid AND a.attnum = d.refobjsubid
                          WHERE d.classid = 'pg_class'::regclass AND d.deptype IN ('a', 'i')
                            AND d.objid = (quote_ident(s.schemaname) || '.' || quote_ident(s.sequencename))::regclass
                          LIMIT 1),
                         COALESCE('at ' || s.last_value::text, 'not used yet')),
                       ''
                     FROM pg_sequences s WHERE s.schemaname NOT IN {PG_SKIP} ORDER BY 2, 3"
                ),
            ),
            (
                User,
                "SELECT 'user', NULL, r.rolname,
                   concat_ws(' · ', CASE WHEN r.rolcanlogin THEN 'can sign in' ELSE 'role' END,
                     CASE WHEN r.rolsuper THEN 'superuser' END, CASE WHEN r.rolcreatedb THEN 'creates databases' END,
                     CASE WHEN r.rolcreaterole THEN 'manages roles' END),
                   r.oid::text
                 FROM pg_roles r WHERE r.rolname NOT LIKE 'pg\\_%' ORDER BY 3"
                    .into(),
            ),
        ],
        // MySQL 8 returns catalog names as binary strings, hence the casts.
        DbKind::Mysql => vec![
            (
                Function,
                format!(
                    "SELECT LOWER(CAST(ROUTINE_TYPE AS CHAR)), CAST(ROUTINE_SCHEMA AS CHAR), CAST(ROUTINE_NAME AS CHAR),
                       CAST(IF(ROUTINE_TYPE = 'FUNCTION', CONCAT('→ ', DTD_IDENTIFIER), '') AS CHAR), ''
                     FROM information_schema.ROUTINES WHERE ROUTINE_SCHEMA NOT IN {MYSQL_SKIP} ORDER BY 2, 3"
                ),
            ),
            (
                Trigger,
                format!(
                    "SELECT 'trigger', CAST(TRIGGER_SCHEMA AS CHAR), CAST(TRIGGER_NAME AS CHAR),
                       CAST(CONCAT(LOWER(ACTION_TIMING), ' ', LOWER(EVENT_MANIPULATION), ' on ', EVENT_OBJECT_TABLE) AS CHAR), ''
                     FROM information_schema.TRIGGERS WHERE TRIGGER_SCHEMA NOT IN {MYSQL_SKIP} ORDER BY 2, 3"
                ),
            ),
            // MariaDB has sequences; MySQL returns nothing here.
            (
                Sequence,
                format!(
                    "SELECT 'sequence', CAST(TABLE_SCHEMA AS CHAR), CAST(TABLE_NAME AS CHAR), '', ''
                     FROM information_schema.TABLES WHERE TABLE_TYPE = 'SEQUENCE' AND TABLE_SCHEMA NOT IN {MYSQL_SKIP} ORDER BY 2, 3"
                ),
            ),
            (
                User,
                "SELECT 'user', NULL, CAST(GRANTEE AS CHAR),
                   CAST(IF(SUM(PRIVILEGE_TYPE <> 'USAGE') = 0, 'no server-wide privileges',
                     CONCAT(SUM(PRIVILEGE_TYPE <> 'USAGE'), ' server-wide privileges')) AS CHAR),
                   CAST(GRANTEE AS CHAR)
                 FROM information_schema.USER_PRIVILEGES GROUP BY GRANTEE ORDER BY 3"
                    .into(),
            ),
        ],
        DbKind::Sqlite => vec![(Trigger, "SELECT 'trigger', 'main', name, 'on ' || tbl_name, '' FROM sqlite_master WHERE type = 'trigger' ORDER BY name".into())],
        DbKind::Sqlserver => vec![
            (
                Function,
                "SELECT CASE WHEN o.type = 'P' THEN 'procedure' ELSE 'function' END, s.name, o.name,
                   CASE o.type WHEN 'FN' THEN 'returns a value' WHEN 'IF' THEN 'returns a table' WHEN 'TF' THEN 'returns a table' ELSE '' END,
                   CAST(o.object_id AS varchar(20))
                 FROM sys.objects o JOIN sys.schemas s ON s.schema_id = o.schema_id
                 WHERE o.type IN ('P', 'FN', 'IF', 'TF') AND o.is_ms_shipped = 0 ORDER BY 2, 3"
                    .into(),
            ),
            (
                Trigger,
                "SELECT 'trigger', s.name, t.name, 'on ' + o.name, CAST(t.object_id AS varchar(20))
                 FROM sys.triggers t JOIN sys.objects o ON o.object_id = t.parent_id JOIN sys.schemas s ON s.schema_id = o.schema_id
                 WHERE t.parent_class = 1 AND t.is_ms_shipped = 0 ORDER BY 2, 3"
                    .into(),
            ),
            (
                Sequence,
                "SELECT 'sequence', s.name, q.name, 'at ' + CAST(q.current_value AS varchar(40)), CAST(q.object_id AS varchar(20))
                 FROM sys.sequences q JOIN sys.schemas s ON s.schema_id = q.schema_id ORDER BY 2, 3"
                    .into(),
            ),
            (
                User,
                "SELECT 'user', NULL, p.name, LOWER(REPLACE(p.type_desc, '_', ' ')), CAST(p.principal_id AS varchar(20))
                 FROM sys.database_principals p
                 WHERE p.type IN ('S', 'U', 'G', 'E', 'X', 'R') AND p.is_fixed_role = 0
                   AND p.name NOT IN ('dbo', 'guest', 'INFORMATION_SCHEMA', 'sys', 'public')
                 ORDER BY 3"
                    .into(),
            ),
        ],
    }
}

fn text(row: &[Cell], i: usize) -> String {
    row.get(i).cloned().flatten().unwrap_or_default()
}

/// `'name'@'host'` as MySQL reports grantees, split so it can be quoted again safely.
fn mysql_grantee(g: &str) -> Option<(String, String)> {
    let (user, host) = g.strip_prefix('\'')?.strip_suffix('\'')?.split_once("'@'")?;
    Some((user.replace("''", "'"), host.replace("''", "'")))
}

pub fn from_rows(rows: &[Vec<Cell>]) -> Vec<DbObject> {
    rows.iter()
        .filter_map(|r| {
            let kind = ObjectKind::parse(&text(r, 0))?;
            let schema = r.get(1).cloned().flatten().filter(|s| !s.is_empty());
            let mut name = text(r, 2);
            if let Some((user, host)) = (kind == ObjectKind::User).then(|| mysql_grantee(&name)).flatten() {
                name = format!("{user}@{host}");
            }
            Some(DbObject { kind, schema, name, detail: text(r, 3), key: text(r, 4) })
        })
        .collect()
}

pub async fn list(driver: &dyn DbDriver) -> ObjectList {
    let kind = driver.dialect().kind;
    let mut out = ObjectList { kinds: kinds(kind), ..Default::default() };
    for (section, sql) in list_queries(kind) {
        match driver.fetch(&sql).await {
            Ok((_, rows)) => out.objects.extend(from_rows(&rows)),
            Err(e) => out.notes.push(format!("Couldn't read {}: {e}", section.plural())),
        }
    }
    out
}

fn numeric_key(o: &DbObject) -> Result<u64> {
    o.key.parse().map_err(|_| Error::Invalid(format!("No catalog id for {}", o.name)))
}

/// Rows of `(order, line)` or `(line)` joined into one script.
fn lines(rows: &[Vec<Cell>], col: usize) -> String {
    rows.iter().map(|r| text(r, col)).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

async fn one(driver: &dyn DbDriver, sql: &str, col: usize) -> Result<String> {
    let (_, rows) = driver.fetch(sql).await?;
    Ok(lines(&rows, col))
}

pub async fn source(driver: &dyn DbDriver, o: &DbObject) -> Result<ObjectSource> {
    use ObjectKind::*;
    let d = driver.dialect();
    let qualified = d.table(o.schema.as_deref(), &o.name);
    let missing = || Error::Invalid(format!("{} isn't there any more, or you aren't allowed to see its definition.", o.name));
    let (definition, drop) = match d.kind {
        DbKind::Postgres => {
            let oid = if o.kind == Sequence { 0 } else { numeric_key(o)? };
            match o.kind {
                Function | Procedure => (
                    one(driver, &format!("SELECT pg_get_functiondef({oid})"), 0).await?,
                    one(
                        driver,
                        &format!(
                            "SELECT 'DROP ' || CASE p.prokind WHEN 'p' THEN 'PROCEDURE' ELSE 'FUNCTION' END || ' '
                               || quote_ident(n.nspname) || '.' || quote_ident(p.proname) || '(' || pg_get_function_identity_arguments(p.oid) || ');'
                             FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE p.oid = {oid}"
                        ),
                        0,
                    )
                    .await?,
                ),
                Trigger => (
                    one(driver, &format!("SELECT pg_get_triggerdef(t.oid, true) || E';\\n\\n-- The function it runs:\\n' || pg_get_functiondef(t.tgfoid) FROM pg_trigger t WHERE t.oid = {oid}"), 0).await?,
                    one(driver, &format!("SELECT 'DROP TRIGGER ' || quote_ident(t.tgname) || ' ON ' || t.tgrelid::regclass::text || ';' FROM pg_trigger t WHERE t.oid = {oid}"), 0).await?,
                ),
                Sequence => (
                    one(
                        driver,
                        &format!(
                            "SELECT format(E'CREATE SEQUENCE %I.%I\\n  AS %s\\n  INCREMENT BY %s\\n  MINVALUE %s\\n  MAXVALUE %s\\n  START WITH %s\\n  CACHE %s%s;\\n\\n-- Current value: %s',
                               schemaname, sequencename, data_type, increment_by, min_value, max_value, start_value, cache_size,
                               CASE WHEN cycle THEN E'\\n  CYCLE' ELSE '' END, COALESCE(last_value::text, 'not used yet'))
                             FROM pg_sequences WHERE schemaname = {} AND sequencename = {}",
                            d.string(o.schema.as_deref().unwrap_or("public")),
                            d.string(&o.name)
                        ),
                        0,
                    )
                    .await?,
                    format!("DROP SEQUENCE {qualified};"),
                ),
                User => {
                    let sql = format!(
                        "SELECT 1, 'CREATE ROLE ' || quote_ident(r.rolname) || CASE WHEN r.rolcanlogin THEN ' LOGIN' ELSE ' NOLOGIN' END
                           || CASE WHEN r.rolsuper THEN ' SUPERUSER' ELSE '' END || CASE WHEN r.rolcreatedb THEN ' CREATEDB' ELSE '' END
                           || CASE WHEN r.rolcreaterole THEN ' CREATEROLE' ELSE '' END || ';'
                         FROM pg_roles r WHERE r.oid = {oid}
                         UNION ALL
                         SELECT 2, 'GRANT ' || quote_ident(g.rolname) || ' TO ' || quote_ident(r.rolname) || ';'
                         FROM pg_auth_members m JOIN pg_roles g ON g.oid = m.roleid JOIN pg_roles r ON r.oid = m.member WHERE m.member = {oid}
                         UNION ALL
                         SELECT 3, 'GRANT ' || string_agg(privilege_type, ', ' ORDER BY privilege_type) || ' ON '
                           || quote_ident(table_schema) || '.' || quote_ident(table_name) || ' TO ' || quote_ident(grantee) || ';'
                         FROM information_schema.role_table_grants
                         WHERE grantee = (SELECT rolname FROM pg_roles WHERE oid = {oid}) AND table_schema NOT IN {PG_SKIP}
                         GROUP BY table_schema, table_name, grantee
                         ORDER BY 1, 2"
                    );
                    (one(driver, &sql, 1).await?, format!("DROP ROLE {};", d.ident(&o.name)))
                }
            }
        }
        DbKind::Mysql => match o.kind {
            Function | Procedure => {
                let word = if o.kind == Function { "FUNCTION" } else { "PROCEDURE" };
                (one(driver, &format!("SHOW CREATE {word} {qualified}"), 2).await?, format!("DROP {word} {qualified};"))
            }
            Trigger => (one(driver, &format!("SHOW CREATE TRIGGER {qualified}"), 2).await?, format!("DROP TRIGGER {qualified};")),
            Sequence => (one(driver, &format!("SHOW CREATE SEQUENCE {qualified}"), 1).await?, format!("DROP SEQUENCE {qualified};")),
            User => {
                let (user, host) = mysql_grantee(&o.key).ok_or_else(missing)?;
                let account = format!("{}@{}", d.string(&user), d.string(&host));
                let (_, rows) = driver.fetch(&format!("SHOW GRANTS FOR {account}")).await?;
                let grants = rows.iter().map(|r| format!("{};", text(r, 0))).collect::<Vec<_>>().join("\n");
                (grants, format!("DROP USER {account};"))
            }
        },
        DbKind::Sqlite => (
            one(driver, &format!("SELECT sql || ';' FROM sqlite_master WHERE type = 'trigger' AND name = {}", d.string(&o.name)), 0).await?,
            format!("DROP TRIGGER {};", d.ident(&o.name)),
        ),
        DbKind::Sqlserver => {
            let id = numeric_key(o)?;
            match o.kind {
                Function | Procedure | Trigger => {
                    let word = match o.kind {
                        Function => "FUNCTION",
                        Procedure => "PROCEDURE",
                        _ => "TRIGGER",
                    };
                    (one(driver, &format!("SELECT OBJECT_DEFINITION({id})"), 0).await?, format!("DROP {word} {qualified};"))
                }
                Sequence => (
                    one(
                        driver,
                        &format!(
                            "SELECT 'CREATE SEQUENCE ' + QUOTENAME(s.name) + '.' + QUOTENAME(q.name) + CHAR(10)
                               + '  AS ' + TYPE_NAME(q.user_type_id) + CHAR(10)
                               + '  START WITH ' + CAST(q.start_value AS varchar(40)) + CHAR(10)
                               + '  INCREMENT BY ' + CAST(q.increment AS varchar(40)) + CHAR(10)
                               + '  MINVALUE ' + CAST(q.minimum_value AS varchar(40)) + CHAR(10)
                               + '  MAXVALUE ' + CAST(q.maximum_value AS varchar(40)) + CHAR(10)
                               + CASE WHEN q.is_cycling = 1 THEN '  CYCLE' ELSE '  NO CYCLE' END + ';' + CHAR(10) + CHAR(10)
                               + '-- Current value: ' + CAST(q.current_value AS varchar(40))
                             FROM sys.sequences q JOIN sys.schemas s ON s.schema_id = q.schema_id WHERE q.object_id = {id}"
                        ),
                        0,
                    )
                    .await?,
                    format!("DROP SEQUENCE {qualified};"),
                ),
                User => {
                    // Catalog names and literals have different collations; the casts let them meet in one list.
                    let sql = format!(
                        "SELECT 1, CAST('-- ' + p.type_desc + CASE WHEN p.default_schema_name IS NOT NULL THEN ', default schema ' + p.default_schema_name ELSE '' END AS nvarchar(max)) COLLATE DATABASE_DEFAULT
                         FROM sys.database_principals p WHERE p.principal_id = {id}
                         UNION ALL
                         SELECT 2, CAST('ALTER ROLE ' + QUOTENAME(r.name) + ' ADD MEMBER ' + QUOTENAME(m.name) + ';' AS nvarchar(max)) COLLATE DATABASE_DEFAULT
                         FROM sys.database_role_members rm
                         JOIN sys.database_principals r ON r.principal_id = rm.role_principal_id
                         JOIN sys.database_principals m ON m.principal_id = rm.member_principal_id
                         WHERE rm.member_principal_id = {id}
                         UNION ALL
                         SELECT 3, CAST(CASE WHEN dp.state = 'W' THEN 'GRANT' ELSE dp.state_desc END + ' ' + dp.permission_name
                           + CASE WHEN dp.class = 1 THEN ' ON ' + QUOTENAME(OBJECT_SCHEMA_NAME(dp.major_id)) + '.' + QUOTENAME(OBJECT_NAME(dp.major_id))
                                  WHEN dp.class = 3 THEN ' ON SCHEMA::' + QUOTENAME(SCHEMA_NAME(dp.major_id)) ELSE '' END
                           + ' TO ' + QUOTENAME(USER_NAME({id})) + CASE WHEN dp.state = 'W' THEN ' WITH GRANT OPTION' ELSE '' END + ';' AS nvarchar(max)) COLLATE DATABASE_DEFAULT
                         FROM sys.database_permissions dp WHERE dp.grantee_principal_id = {id} AND dp.permission_name <> 'CONNECT'
                         ORDER BY 1, 2"
                    );
                    let is_role = o.detail.contains("role");
                    (one(driver, &sql, 1).await?, format!("DROP {} {};", if is_role { "ROLE" } else { "USER" }, d.ident(&o.name)))
                }
            }
        }
    };
    if definition.trim().is_empty() {
        return Err(missing());
    }
    Ok(ObjectSource { definition, drop })
}

/// A starting point for a new object, to finish in the SQL editor.
pub fn template(d: Dialect, kind: ObjectKind, schema: Option<&str>) -> String {
    use ObjectKind::*;
    let name = |n: &str| d.table(schema, n);
    match (d.kind, kind) {
        (DbKind::Postgres, Function) => format!(
            "CREATE OR REPLACE FUNCTION {}(a integer, b integer)\nRETURNS integer\nLANGUAGE sql\nIMMUTABLE\nAS $$\n  SELECT a + b;\n$$;",
            name("add_numbers")
        ),
        (DbKind::Postgres, Procedure) => format!(
            "CREATE OR REPLACE PROCEDURE {}(days integer)\nLANGUAGE plpgsql\nAS $$\nBEGIN\n  -- DELETE FROM my_table WHERE created_at < now() - make_interval(days => days);\nEND;\n$$;",
            name("clean_up")
        ),
        (DbKind::Postgres, Trigger) => format!(
            "-- Keeps an updated_at column current.\nCREATE OR REPLACE FUNCTION {}()\nRETURNS trigger\nLANGUAGE plpgsql\nAS $$\nBEGIN\n  NEW.updated_at := now();\n  RETURN NEW;\nEND;\n$$;\n\nCREATE TRIGGER set_updated_at\nBEFORE UPDATE ON {}\nFOR EACH ROW EXECUTE FUNCTION {}();",
            name("touch_updated_at"),
            name("my_table"),
            name("touch_updated_at")
        ),
        (DbKind::Postgres, Sequence) => format!("CREATE SEQUENCE {}\n  START WITH 1000\n  INCREMENT BY 1;", name("invoice_numbers")),
        (DbKind::Postgres, User) => "-- A user who can read every table in the public schema.\nCREATE ROLE report_reader LOGIN PASSWORD 'change me';\nGRANT USAGE ON SCHEMA public TO report_reader;\nGRANT SELECT ON ALL TABLES IN SCHEMA public TO report_reader;\nALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT ON TABLES TO report_reader;".into(),
        (DbKind::Mysql, Function) => format!(
            "CREATE FUNCTION {}(a INT, b INT)\nRETURNS INT\nDETERMINISTIC\nRETURN a + b;",
            name("add_numbers")
        ),
        (DbKind::Mysql, Procedure) => format!(
            "CREATE PROCEDURE {}(IN days INT)\nBEGIN\n  -- DELETE FROM my_table WHERE created_at < NOW() - INTERVAL days DAY;\nEND;",
            name("clean_up")
        ),
        (DbKind::Mysql, Trigger) => format!(
            "-- Keeps an updated_at column current.\nCREATE TRIGGER {}\nBEFORE UPDATE ON {}\nFOR EACH ROW SET NEW.updated_at = NOW();",
            name("set_updated_at"),
            name("my_table")
        ),
        (DbKind::Mysql, Sequence) => format!("-- MariaDB 10.3 and later.\nCREATE SEQUENCE {}\n  START WITH 1000\n  INCREMENT BY 1;", name("invoice_numbers")),
        (DbKind::Mysql, User) => {
            let db = schema.map(|s| d.ident(s)).unwrap_or_else(|| "my_database".into());
            format!("-- A user who can read every table in {db}.\nCREATE USER 'report_reader'@'%' IDENTIFIED BY 'change me';\nGRANT SELECT ON {db}.* TO 'report_reader'@'%';")
        }
        (DbKind::Sqlite, _) => "-- Keeps an updated_at column current.\nCREATE TRIGGER set_updated_at\nAFTER UPDATE ON my_table\nFOR EACH ROW\nBEGIN\n  UPDATE my_table SET updated_at = CURRENT_TIMESTAMP WHERE rowid = NEW.rowid;\nEND;".into(),
        (DbKind::Sqlserver, Function) => format!(
            "CREATE OR ALTER FUNCTION {}(@a int, @b int)\nRETURNS int\nAS\nBEGIN\n  RETURN @a + @b;\nEND;",
            name("add_numbers")
        ),
        (DbKind::Sqlserver, Procedure) => format!(
            "CREATE OR ALTER PROCEDURE {} @days int\nAS\nBEGIN\n  SET NOCOUNT ON;\n  -- DELETE FROM my_table WHERE created_at < DATEADD(day, -@days, SYSUTCDATETIME());\nEND;",
            name("clean_up")
        ),
        (DbKind::Sqlserver, Trigger) => format!(
            "-- Keeps an updated_at column current.\nCREATE OR ALTER TRIGGER {}\nON {}\nAFTER UPDATE\nAS\nBEGIN\n  SET NOCOUNT ON;\n  UPDATE t SET updated_at = SYSUTCDATETIME()\n  FROM {} t JOIN inserted i ON i.id = t.id;\nEND;",
            name("set_updated_at"),
            name("my_table"),
            name("my_table")
        ),
        (DbKind::Sqlserver, Sequence) => format!("CREATE SEQUENCE {}\n  AS bigint\n  START WITH 1000\n  INCREMENT BY 1;", name("invoice_numbers")),
        (DbKind::Sqlserver, User) => "-- A user who can read every table in this database.\nCREATE USER report_reader WITH PASSWORD = 'Change me 1!';\nALTER ROLE db_datareader ADD MEMBER report_reader;".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_grantees_are_split_and_unescaped() {
        assert_eq!(mysql_grantee("'kiyi'@'%'"), Some(("kiyi".into(), "%".into())));
        assert_eq!(mysql_grantee("'o''brien'@'10.0.%'"), Some(("o'brien".into(), "10.0.%".into())));
        assert_eq!(mysql_grantee("kiyi@%"), None);
    }

    #[test]
    fn rows_become_objects() {
        let rows = vec![
            vec![Some("function".into()), Some("public".into()), Some("add".into()), Some("(a integer) → integer".into()), Some("123".into())],
            vec![Some("user".into()), None, Some("'kiyi'@'%'".into()), Some("".into()), Some("'kiyi'@'%'".into())],
            vec![Some("unknown".into()), None, Some("x".into()), None, None],
        ];
        let objects = from_rows(&rows);
        assert_eq!(objects.len(), 2);
        assert_eq!(objects[0].kind, ObjectKind::Function);
        assert_eq!(objects[0].key, "123");
        assert_eq!(objects[1].name, "kiyi@%");
        assert_eq!(objects[1].schema, None);
    }

    #[test]
    fn sqlite_has_only_triggers() {
        assert_eq!(kinds(DbKind::Sqlite), vec![ObjectKind::Trigger]);
        assert!(template(Dialect::SQLITE, ObjectKind::Trigger, None).contains("CREATE TRIGGER"));
    }

    #[test]
    fn templates_quote_the_schema() {
        let t = template(Dialect::POSTGRES, ObjectKind::Sequence, Some("sales"));
        assert!(t.contains("\"sales\".\"invoice_numbers\""), "{t}");
        let t = template(Dialect::MYSQL, ObjectKind::User, Some("shop"));
        assert!(t.contains("`shop`.*"), "{t}");
    }
}
