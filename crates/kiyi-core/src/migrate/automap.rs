//! A first plan without AI: tables and columns paired by name (and common synonyms, in English
//! and Turkish), references found through foreign keys, names split or joined where the shapes
//! differ. Good enough to start from; the person (or the AI) fixes the rest.

use std::collections::{HashMap, HashSet};

use super::{ColumnMapping, IdMode, MigrationPlan, Step, TableMapping, ValueSource, WriteMode};
use crate::catalog::TypeCategory;
use crate::config::DbKind;
use crate::design::TableDetails;
use crate::drivers::DbDriver;
use crate::error::Result;

/// Lower case letters and digits only: `Created_At`, `createdAt` and `created-at` look alike.
pub(crate) fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn singular(s: &str) -> String {
    if s.len() > 4 && s.ends_with("ies") {
        format!("{}y", &s[..s.len() - 3])
    } else if s.len() > 4 && (s.ends_with("sses") || s.ends_with("xes") || s.ends_with("ches") || s.ends_with("shes")) {
        s[..s.len() - 2].to_string()
    } else if s.len() > 3 && s.ends_with('s') && !s.ends_with("ss") {
        s[..s.len() - 1].to_string()
    } else if s.len() > 5 && (s.ends_with("ler") || s.ends_with("lar")) {
        // Turkish plurals: musteriler → musteri.
        s[..s.len() - 3].to_string()
    } else {
        s.to_string()
    }
}

/// Legacy prefixes that say nothing: tbl_customers, tb_orders.
fn bare_table(s: &str) -> String {
    let n = norm(s);
    let n = ["tbl", "tb"].iter().find_map(|p| n.strip_prefix(p).filter(|r| r.len() > 2)).map(str::to_string).unwrap_or(n);
    singular(&n)
}

const TABLE_SYNONYMS: &[&[&str]] = &[
    &["customer", "client", "musteri", "cari"],
    &["user", "member", "account", "kullanici", "uye"],
    &["product", "item", "article", "urun", "stok"],
    &["order", "purchase", "siparis"],
    &["orderitem", "orderline", "lineitem", "orderdetail", "siparisdetay", "siparisurun", "siparissatir"],
    &["category", "kategori"],
    &["invoice", "bill", "fatura"],
    &["payment", "transaction", "odeme"],
    &["address", "adres"],
    &["employee", "staff", "personel", "calisan"],
    &["supplier", "vendor", "tedarikci"],
    &["company", "organization", "organisation", "firma", "sirket"],
];

const COLUMN_SYNONYMS: &[&[&str]] = &[
    &["email", "emailaddress", "mail", "eposta", "epostaadresi"],
    &["phone", "phonenumber", "tel", "telephone", "mobile", "gsm", "telefon", "cep", "ceptelefonu"],
    &["name", "fullname", "displayname", "adsoyad", "adisoyadi", "isim", "unvan", "title"],
    &["firstname", "givenname", "fname", "forename", "ad", "adi"],
    &["lastname", "surname", "familyname", "lname", "soyad", "soyadi"],
    &["createdat", "created", "createdon", "creationdate", "datecreated", "insertedat", "createdate", "kayittarihi", "olusturmatarihi", "signedupat", "registeredat", "joined", "joinedat", "joindate", "registered", "since", "membersince"],
    &["updatedat", "updated", "modifiedat", "modified", "lastmodified", "updatedon", "guncellemetarihi"],
    &["price", "unitprice", "fiyat", "birimfiyat"],
    &["quantity", "qty", "adet", "miktar"],
    &["total", "totalamount", "grandtotal", "amount", "tutar", "toplam", "toplamtutar"],
    &["description", "desc", "details", "aciklama", "detay"],
    &["address", "addr", "streetaddress", "adres"],
    &["city", "town", "sehir", "il"],
    &["country", "ulke"],
    &["zip", "zipcode", "postalcode", "postcode", "postakodu"],
    &["active", "isactive", "enabled", "aktif"],
    &["status", "state", "durum"],
    &["sku", "code", "productcode", "stokkodu", "urunkodu"],
    &["note", "notes", "comment", "comments", "not", "notlar"],
    &["birthdate", "dateofbirth", "dob", "birthday", "dogumtarihi"],
];

fn same_group(groups: &[&[&str]], a: &str, b: &str) -> bool {
    groups.iter().any(|g| g.contains(&a) && g.contains(&b))
}

fn table_score(source: &str, target: &str) -> u32 {
    let (s, t) = (bare_table(source), bare_table(target));
    if norm(source) == norm(target) {
        100
    } else if s == t {
        95
    } else if same_group(TABLE_SYNONYMS, &s, &t) {
        80
    } else if padded(&s, &t) || padded(&t, &s) {
        55
    } else {
        0
    }
}

/// `customer_data`, `customers_master`: the same table with a word that adds nothing.
/// (`order_items` is not `orders`.)
fn padded(long: &str, short: &str) -> bool {
    const FILLER: &[&str] = &["data", "info", "list", "table", "master", "record", "records", "main", "new", "old", "v2"];
    short.len() >= 4 && (long.strip_prefix(short).or_else(|| long.strip_suffix(short))).is_some_and(|rest| FILLER.contains(&singular(rest).as_str()) || FILLER.contains(&rest))
}

/// `is_vip` and `vip`, `has_newsletter` and `newsletter`: the same yes/no.
fn bare_flag(s: &str) -> &str {
    ["is", "has"].iter().find_map(|p| s.strip_prefix(p).filter(|r| r.len() >= 3)).unwrap_or(s)
}

fn column_score(source: &str, target: &str) -> u32 {
    let (s, t) = (norm(source), norm(target));
    if s == t {
        100
    } else if singular(&s) == singular(&t) {
        95
    } else if same_group(COLUMN_SYNONYMS, &s, &t) {
        80
    } else if bare_flag(&s) == bare_flag(&t) {
        75
    } else if s.len() >= 4 && t.len() >= 4 && (s.ends_with(&t) || t.ends_with(&s)) {
        // customer_email ↔ email
        55
    } else {
        0
    }
}

fn compatible(from: TypeCategory, to: TypeCategory) -> bool {
    use TypeCategory::*;
    match (from, to) {
        (_, Text | Other) | (Other, _) => true,
        (a, b) if a == b => true,
        (Number | Decimal, Number | Decimal | Boolean) | (Boolean, Number | Boolean) => true,
        (Date | DateTime, Date | DateTime) => true,
        (Text, _) => true,
        (List, Json) => true,
        _ => false,
    }
}

fn group_of(name: &str) -> Option<&'static str> {
    let n = norm(name);
    COLUMN_SYNONYMS.iter().find(|g| g.contains(&n.as_str())).map(|g| g[0])
}

struct Side {
    kind: DbKind,
    details: TableDetails,
}

impl Side {
    fn category(&self, column: &str) -> TypeCategory {
        self.details.design.columns.iter().find(|c| c.name == column).map(|c| crate::catalog::category(self.kind, &c.data_type)).unwrap_or(TypeCategory::Other)
    }
    fn single_key(&self) -> Option<&crate::design::ColumnDesign> {
        let keys: Vec<_> = self.details.design.columns.iter().filter(|c| c.primary_key).collect();
        (keys.len() == 1).then(|| keys[0])
    }
    fn fk_to(&self, column: &str) -> Option<(Option<String>, String)> {
        self.details.design.foreign_keys.iter().find(|f| f.columns.len() == 1 && f.columns[0] == column).map(|f| (f.ref_schema.clone(), f.ref_table.clone()))
    }
}

/// Column mappings for one table pair. `pairs` maps target table name → (source schema, source table)
/// for every pair in the plan, so foreign keys can become references.
fn map_columns(source: &Side, target: &Side, pairs: &HashMap<String, (Option<String>, String)>, renumber: bool) -> Vec<ColumnMapping> {
    let mut used: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    let target_key = target.single_key().map(|c| c.name.clone());
    for tc in &target.details.design.columns {
        let tcat = crate::catalog::category(target.kind, &tc.data_type);
        let mapping = |source: ValueSource| ColumnMapping { target: tc.name.clone(), source, steps: vec![] };
        if tc.generated {
            out.push(mapping(ValueSource::Default));
            continue;
        }
        // The key of a renumbered table gets new numbers; nothing to map.
        if renumber && target_key.as_deref() == Some(tc.name.as_str()) {
            out.push(mapping(ValueSource::Default));
            continue;
        }
        // A link to another table in the plan: follow the source's link to the matching table.
        if let Some((_, ref_table)) = target.fk_to(&tc.name) {
            if let Some((rs, rt)) = pairs.get(&ref_table) {
                let via = source.details.design.foreign_keys.iter().find(|f| f.columns.len() == 1 && f.ref_table == *rt && !used.contains(&f.columns[0]));
                if let Some(f) = via {
                    used.insert(f.columns[0].clone());
                    out.push(mapping(ValueSource::Reference { column: f.columns[0].clone(), schema: rs.clone(), table: rt.clone() }));
                    continue;
                }
            }
        }
        let best = source
            .details
            .design
            .columns
            .iter()
            .filter(|sc| !used.contains(&sc.name))
            .map(|sc| (column_score(&sc.name, &tc.name), sc))
            .filter(|(score, sc)| *score >= 55 && compatible(source.category(&sc.name), tcat))
            .max_by_key(|(score, _)| *score);
        match best {
            Some((_, sc)) => {
                used.insert(sc.name.clone());
                // A plain column copied into a link would point at the wrong rows if the
                // referenced table is renumbered; references were handled above.
                out.push(mapping(ValueSource::Column { column: sc.name.clone() }));
            }
            None => out.push(mapping(ValueSource::Default)),
        }
    }
    split_and_join_names(source, &mut out);
    out
}

/// first name + last name ↔ full name, when only one side has the pair.
fn split_and_join_names(source: &Side, out: &mut [ColumnMapping]) {
    let find_source = |group: &str| source.details.design.columns.iter().find(|c| group_of(&c.name) == Some(group)).map(|c| c.name.clone());
    let unmapped = |m: &ColumnMapping| m.source == ValueSource::Default;
    // Only a person's whole name is worth splitting, not a product's title.
    let full = source.details.design.columns.iter().find(|c| ["name", "fullname", "displayname", "adsoyad", "adisoyadi", "isim"].contains(&norm(&c.name).as_str())).map(|c| c.name.clone());
    let (first, last) = (find_source("firstname"), find_source("lastname"));
    for m in out.iter_mut() {
        if !unmapped(m) {
            continue;
        }
        match group_of(&m.target) {
            Some("firstname") if first.is_none() => {
                if let Some(full) = &full {
                    m.source = ValueSource::Column { column: full.clone() };
                    m.steps = vec![Step::Split { separator: " ".into(), part: 1, rest: false }];
                }
            }
            Some("lastname") if last.is_none() => {
                if let Some(full) = &full {
                    m.source = ValueSource::Column { column: full.clone() };
                    m.steps = vec![Step::Split { separator: " ".into(), part: 2, rest: true }];
                }
            }
            Some("name") => {
                if let (Some(f), Some(l)) = (&first, &last) {
                    m.source = ValueSource::Combine { columns: vec![f.clone(), l.clone()], separator: " ".into() };
                }
            }
            _ => {}
        }
    }
}

async fn is_empty(driver: &dyn DbDriver, schema: Option<&str>, table: &str) -> bool {
    let d = driver.dialect();
    let req = crate::dml::BrowseRequest {
        schema: schema.map(str::to_string),
        table: table.into(),
        filters: vec![],
        raw_where: None,
        search: None,
        search_columns: vec![],
        sort: vec![],
        tiebreak: vec![],
        limit: 1,
        offset: 0,
    };
    driver.fetch(&crate::dml::browse_sql(d, &req)).await.map(|(_, rows)| rows.is_empty()).unwrap_or(false)
}

/// A first plan from the two databases' structure.
pub async fn suggest(source: &dyn DbDriver, target: &dyn DbDriver, source_id: &str, target_id: &str) -> Result<MigrationPlan> {
    let (s_snap, t_snap) = (source.schema().await?, target.schema().await?);
    let s_kind = source.dialect().kind;
    let t_kind = target.dialect().kind;
    // SQLite has one unnamed schema; elsewhere tables are named with theirs.
    let named = |kind: DbKind, s: &crate::types::SchemaInfo| if kind == DbKind::Sqlite { None } else { Some(s.name.clone()) };
    // On MySQL each schema is a separate database; only the connection's own is considered.
    let own = |kind: DbKind, snap: &crate::types::SchemaSnapshot, s: &crate::types::SchemaInfo| kind != DbKind::Mysql || snap.default_schema.is_none() || snap.default_schema.as_deref() == Some(s.name.as_str());
    let sources: Vec<(Option<String>, String)> =
        s_snap.schemas.iter().filter(|s| own(s_kind, &s_snap, s)).flat_map(|s| s.tables.iter().map(move |t| (named(s_kind, s), t.name.clone()))).collect();
    let targets: Vec<(Option<String>, String)> = t_snap
        .schemas
        .iter()
        .filter(|s| own(t_kind, &t_snap, s))
        .flat_map(|s| s.tables.iter().filter(|t| t.kind == crate::types::TableKind::Table).map(move |t| (named(t_kind, s), t.name.clone())))
        .collect();

    // Best source for each target, each source used once, strongest matches first.
    let mut candidates: Vec<(u32, usize, usize)> = Vec::new();
    for (ti, (_, t)) in targets.iter().enumerate() {
        for (si, (_, s)) in sources.iter().enumerate() {
            let score = table_score(s, t);
            if score > 0 {
                candidates.push((score, ti, si));
            }
        }
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let (mut taken_t, mut taken_s) = (HashSet::new(), HashSet::new());
    let mut chosen: Vec<(usize, usize)> = Vec::new();
    for (_, ti, si) in candidates {
        if !taken_t.contains(&ti) && !taken_s.contains(&si) {
            taken_t.insert(ti);
            taken_s.insert(si);
            chosen.push((ti, si));
        }
    }
    chosen.sort();

    let pairs: HashMap<String, (Option<String>, String)> = chosen.iter().map(|(ti, si)| (targets[*ti].1.clone(), sources[*si].clone())).collect();
    let mut tables = Vec::new();
    for (ti, si) in chosen {
        let (ts, tt) = &targets[ti];
        let (ss, st) = &sources[si];
        let target_side = Side { kind: t_kind, details: target.table_details(ts.as_deref(), tt).await? };
        let source_side = Side { kind: s_kind, details: source.table_details(ss.as_deref(), st).await? };
        // Keep the source's numbers when they fit and nothing is in the way; renumber otherwise.
        let renumber = match (target_side.single_key(), source_side.single_key()) {
            (Some(tk), sk) if tk.auto_increment && crate::catalog::category(t_kind, &tk.data_type) == TypeCategory::Number => {
                let numeric_source = sk.is_some_and(|k| crate::catalog::category(s_kind, &k.data_type) == TypeCategory::Number);
                sk.is_some() && !(numeric_source && is_empty(target, ts.as_deref(), tt).await)
            }
            _ => false,
        };
        tables.push(TableMapping {
            enabled: true,
            source_schema: ss.clone(),
            source_table: st.clone(),
            target_schema: ts.clone(),
            target_table: tt.clone(),
            columns: map_columns(&source_side, &target_side, &pairs, renumber),
            ids: if renumber { IdMode::Renumber } else { IdMode::Keep },
            write: WriteMode::Insert,
            match_on: target_side.single_key().map(|k| vec![k.name.clone()]).unwrap_or_default(),
        });
    }
    Ok(MigrationPlan { source: source_id.into(), target: target_id.into(), source_name: String::new(), target_name: String::new(), tables })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_pair_by_name_prefix_and_synonym() {
        assert_eq!(table_score("customers", "customers"), 100);
        assert_eq!(table_score("tbl_customers", "customer"), 95);
        assert_eq!(table_score("Musteriler", "customers"), 80);
        assert_eq!(table_score("clients", "customers"), 80);
        assert_eq!(table_score("order_items", "orders"), 0);
        assert_eq!(table_score("OrderLines", "order_items"), 80);
        assert_eq!(table_score("customer_data", "customers"), 55);
    }

    #[test]
    fn columns_pair_across_naming_styles_and_languages() {
        assert_eq!(column_score("createdAt", "created_at"), 100);
        assert_eq!(column_score("E_Posta", "email"), 80);
        assert_eq!(column_score("signed_up_at", "created_at"), 80);
        assert_eq!(column_score("customer_email", "email"), 55);
        assert_eq!(column_score("id", "uuid"), 0);
        assert_eq!(column_score("Soyadi", "last_name"), 80);
        assert_eq!(column_score("vip", "is_vip"), 75);
    }

    #[test]
    fn incompatible_types_are_not_paired() {
        use TypeCategory::*;
        assert!(compatible(Number, Decimal));
        assert!(compatible(Text, DateTime), "text can hold dates; conversion checks each value");
        assert!(!compatible(DateTime, Boolean));
        assert!(!compatible(Binary, Number));
    }
}
