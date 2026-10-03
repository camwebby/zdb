//! Cached schema (tables, columns, keys) for the finder, sidebar, autocomplete and grid
//! headers, plus the structure view queries.

use super::{DbError, Session, quote_qualified};
use crate::config::DriverKind;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableKind {
    Table,
    View,
}

#[derive(Debug, Clone)]
pub struct TableInfo {
    pub schema: String,
    pub name: String,
    pub kind: TableKind,
    pub rows_estimate: Option<i64>,
}

impl TableInfo {
    /// Name as shown to the user: `public.orders` on Postgres, `orders` on MySQL.
    pub fn display(&self) -> String {
        if self.schema.is_empty() { self.name.clone() } else { format!("{}.{}", self.schema, self.name) }
    }
}

#[derive(Debug, Clone)]
pub struct ColInfo {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
    pub pk: bool,
    /// (referenced table display name, referenced column)
    pub fk: Option<(String, String)>,
}

#[derive(Debug, Clone, Default)]
pub struct Schema {
    pub tables: Vec<TableInfo>,
    /// keyed by `TableInfo::display()`
    pub columns: HashMap<String, Vec<ColInfo>>,
}

impl Schema {
    pub fn table(&self, name: &str) -> Option<&TableInfo> {
        self.tables
            .iter()
            .find(|t| t.display() == name)
            .or_else(|| self.tables.iter().find(|t| t.name == name && (t.schema == "public" || t.schema.is_empty())))
            .or_else(|| self.tables.iter().find(|t| t.name.eq_ignore_ascii_case(name)))
    }
    pub fn columns_of(&self, name: &str) -> &[ColInfo] {
        self.table(name).and_then(|t| self.columns.get(&t.display())).map(|v| v.as_slice()).unwrap_or(&[])
    }
    pub fn primary_key(&self, name: &str) -> Vec<String> {
        self.columns_of(name).iter().filter(|c| c.pk).map(|c| c.name.clone()).collect()
    }
}

fn s(v: &Option<String>) -> String {
    v.clone().unwrap_or_default()
}

pub async fn load(sess: &mut Session) -> Result<Schema, DbError> {
    let d = sess.driver();
    let mut schema = Schema::default();
    let (tables_q, cols_q, pk_q, fk_q) = match d {
        DriverKind::Postgres => (
            "select n.nspname, c.relname, c.relkind::text, c.reltuples::bigint
             from pg_class c join pg_namespace n on n.oid = c.relnamespace
             where c.relkind in ('r','v','m','p','f') and n.nspname not in ('pg_catalog','information_schema')
               and n.nspname not like 'pg_toast%' and n.nspname not like 'pg_temp%'
             order by 1, 2",
            "select n.nspname, c.relname, a.attname, format_type(a.atttypid, a.atttypmod), (not a.attnotnull)::text
             from pg_attribute a join pg_class c on c.oid = a.attrelid join pg_namespace n on n.oid = c.relnamespace
             where a.attnum > 0 and not a.attisdropped and c.relkind in ('r','v','m','p','f')
               and n.nspname not in ('pg_catalog','information_schema') and n.nspname not like 'pg_toast%'
             order by 1, 2, a.attnum",
            "select n.nspname, c.relname, a.attname
             from pg_index i join pg_class c on c.oid = i.indrelid join pg_namespace n on n.oid = c.relnamespace
             join pg_attribute a on a.attrelid = c.oid and a.attnum = any(i.indkey)
             where i.indisprimary and n.nspname not in ('pg_catalog','information_schema')",
            "select n.nspname, c.relname, a.attname, fn.nspname, fc.relname, fa.attname
             from pg_constraint k
             join pg_class c on c.oid = k.conrelid join pg_namespace n on n.oid = c.relnamespace
             join pg_class fc on fc.oid = k.confrelid join pg_namespace fn on fn.oid = fc.relnamespace
             cross join lateral unnest(k.conkey, k.confkey) as u(a1, a2)
             join pg_attribute a on a.attrelid = c.oid and a.attnum = u.a1
             join pg_attribute fa on fa.attrelid = fc.oid and fa.attnum = u.a2
             where k.contype = 'f' and array_length(k.conkey, 1) = 1",
        ),
        DriverKind::Sqlite => (
            "select '', name, type, null from sqlite_schema where type in ('table','view') and name not like 'sqlite_%' order by name",
            "select '', s.name, p.name, p.type, case when p.[notnull] = 0 then 'true' else 'false' end from sqlite_schema s, pragma_table_xinfo(s.name) p where s.type in ('table','view') and s.name not like 'sqlite_%' and p.hidden != 1 order by s.name, p.cid",
            "select '', s.name, p.name from sqlite_schema s, pragma_table_info(s.name) p where s.type = 'table' and p.pk > 0",
            "select '', s.name, f.[from], '', f.[table], coalesce(f.[to], (select name from pragma_table_info(f.[table]) where pk = f.seq + 1)) from sqlite_schema s, pragma_foreign_key_list(s.name) f where s.type = 'table' and (select count(*) from pragma_foreign_key_list(s.name) ff where ff.id = f.id) = 1",
        ),
        DriverKind::Mysql => (
            "select '', table_name, table_type, cast(table_rows as signed) from information_schema.tables
             where table_schema = database() order by table_name",
            "select '', table_name, column_name, column_type, if(is_nullable = 'YES', 'true', 'false')
             from information_schema.columns where table_schema = database() order by table_name, ordinal_position",
            "select '', table_name, column_name from information_schema.key_column_usage
             where table_schema = database() and constraint_name = 'PRIMARY'",
            "select '', table_name, column_name, '', referenced_table_name, referenced_column_name
             from information_schema.key_column_usage
             where table_schema = database() and referenced_table_name is not null",
        ),
    };
    let disp = |sch: &str, name: &str| if sch.is_empty() { name.to_string() } else { format!("{sch}.{name}") };
    for r in sess.rows(tables_q).await? {
        let kind = match s(&r[2]).as_str() {
            "view" | "v" | "m" | "VIEW" | "SYSTEM VIEW" => TableKind::View,
            _ => TableKind::Table,
        };
        let est = r[3].as_ref().and_then(|v| v.parse::<i64>().ok()).filter(|n| *n >= 0);
        schema.tables.push(TableInfo { schema: s(&r[0]), name: s(&r[1]), kind, rows_estimate: est });
    }
    for r in sess.rows(cols_q).await? {
        schema.columns.entry(disp(&s(&r[0]), &s(&r[1]))).or_default().push(ColInfo {
            name: s(&r[2]),
            type_name: s(&r[3]),
            nullable: s(&r[4]) == "true",
            pk: false,
            fk: None,
        });
    }
    for r in sess.rows(pk_q).await? {
        if let Some(cols) = schema.columns.get_mut(&disp(&s(&r[0]), &s(&r[1])))
            && let Some(c) = cols.iter_mut().find(|c| c.name == s(&r[2])) {
                c.pk = true;
            }
    }
    for r in sess.rows(fk_q).await? {
        if let Some(cols) = schema.columns.get_mut(&disp(&s(&r[0]), &s(&r[1])))
            && let Some(c) = cols.iter_mut().find(|c| c.name == s(&r[2])) {
                c.fk = Some((disp(&s(&r[3]), &s(&r[4])), s(&r[5])));
            }
    }
    Ok(schema)
}

/// Structure view sections as plain text tables for the results area.
pub struct Structure {
    pub indexes: Vec<(String, String)>,
    pub foreign_keys: Vec<(String, String)>,
    pub create: String,
}

pub async fn structure(sess: &mut Session, t: &TableInfo, cols: &[ColInfo]) -> Result<Structure, DbError> {
    let d = sess.driver();
    let q = quote_qualified(d, &t.schema, &t.name);
    match d {
        DriverKind::Postgres => {
            let lit = crate::sql::quote_literal(&q);
            let idx = sess
                .rows(&format!("select indexrelid::regclass::text, pg_get_indexdef(indexrelid) from pg_index where indrelid = {lit}::regclass order by 1"))
                .await?;
            let cons = sess
                .rows(&format!(
                    "select conname, pg_get_constraintdef(oid), contype::text from pg_constraint where conrelid = {lit}::regclass order by contype desc, conname"
                ))
                .await?;
            let fks: Vec<(String, String)> = cons.iter().filter(|r| s(&r[2]) == "f").map(|r| (s(&r[0]), s(&r[1]))).collect();
            let create = if t.kind == TableKind::View {
                let def = sess.rows(&format!("select pg_get_viewdef({lit}::regclass, true)")).await?;
                format!("create view {q} as\n{}", def.first().map(|r| s(&r[0])).unwrap_or_default().trim_end())
            } else {
                let mut lines: Vec<String> = cols
                    .iter()
                    .map(|c| format!("  {} {}{}", super::quote_ident(d, &c.name), c.type_name, if c.nullable { "" } else { " not null" }))
                    .collect();
                for r in &cons {
                    lines.push(format!("  constraint {} {}", super::quote_ident(d, &s(&r[0])), s(&r[1])));
                }
                let mut out = format!("create table {q} (\n{}\n);", lines.join(",\n"));
                for r in &idx {
                    let def = s(&r[1]);
                    if !cons.iter().any(|c| def.contains(&format!("INDEX {} ON", s(&c[0])))) {
                        out.push_str(&format!("\n{def};"));
                    }
                }
                out
            };
            Ok(Structure { indexes: idx.iter().map(|r| (s(&r[0]), s(&r[1]))).collect(), foreign_keys: fks, create })
        }
        DriverKind::Sqlite => {
            let lit = crate::sql::quote_literal(&t.name);
            let idx = sess.rows(&format!("select name, coalesce(sql, 'automatic index') from sqlite_schema where type = 'index' and tbl_name = {lit} order by name")).await?;
            let create = sess.rows(&format!("select sql from sqlite_schema where name = {lit} and type in ('table','view')")).await?;
            let fks = sess.rows(&format!("select id, seq, [from], [table], [to], on_update, on_delete from pragma_foreign_key_list({lit}) order by id, seq")).await?;
            Ok(Structure {
                indexes: idx.iter().map(|r| (s(&r[0]), s(&r[1]))).collect(),
                foreign_keys: fks.iter().map(|r| (format!("fk_{}_{}", s(&r[0]), s(&r[1])), format!("({}) references {}({}) on update {} on delete {}", s(&r[2]), s(&r[3]), s(&r[4]), s(&r[5]), s(&r[6])))).collect(),
                create: create.first().map(|r| s(&r[0])).unwrap_or_default(),
            })
        }
        DriverKind::Mysql => {
            let idx = sess.rows(&format!("show index from {q}")).await?;
            let mut grouped: Vec<(String, String)> = Vec::new();
            for r in &idx {
                let name = s(&r[2]);
                let col = s(&r[4]);
                let unique = s(&r[1]) == "0";
                match grouped.iter_mut().find(|(n, _)| *n == name) {
                    Some((_, def)) => def.push_str(&format!(", {col}")),
                    None => grouped.push((name, format!("{}({col}", if unique { "unique " } else { "" }))),
                }
            }
            for (_, d) in grouped.iter_mut() {
                d.push(')');
            }
            let create = sess.rows(&format!("show create table {q}")).await?;
            let create = create.first().and_then(|r| r.get(1).cloned().flatten()).unwrap_or_default();
            let fks = cols
                .iter()
                .filter_map(|c| c.fk.as_ref().map(|(t, rc)| (c.name.clone(), format!("({}) references {t}({rc})", c.name))))
                .collect();
            Ok(Structure { indexes: grouped, foreign_keys: fks, create })
        }
    }
}
