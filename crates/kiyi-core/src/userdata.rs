//! The person's own work that the interface keeps: query history, saved queries, table views and
//! column layouts, open query tabs, settings. Kept as JSON files in Kiyi's folder rather than only
//! in the web view's storage, so a cleared cache doesn't lose it, it can be backed up with the
//! folder, and saved queries and views travel with exported connections.
//!
//! The core doesn't interpret most of it; it only knows enough of the shapes to carry saved
//! queries and views between computers.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{Error, Result};

/// Everything the interface may store. Anything else is refused.
pub const NAMES: &[&str] = &["history", "table-prefs", "query-tabs", "settings"];

pub struct UserData {
    dir: PathBuf,
}

impl UserData {
    pub fn new(config_dir: &Path) -> Self {
        Self { dir: config_dir.join("user-data") }
    }

    fn path(&self, name: &str) -> Result<PathBuf> {
        if !NAMES.contains(&name) {
            return Err(Error::Invalid(format!("Unknown kind of saved data: {name}")));
        }
        Ok(self.dir.join(format!("{name}.json")))
    }

    pub fn load(&self, name: &str) -> Result<Option<Value>> {
        match std::fs::read(self.path(name)?) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes).ok()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, name: &str, value: &Value) -> Result<()> {
        let path = self.path(name)?;
        std::fs::create_dir_all(&self.dir)?;
        // Write-then-rename so a crash never leaves half a file.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(value)?)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }
}

/// Saved queries and table views that go along with exported connections. Query history stays
/// behind: it can hold values from the data.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Extras {
    #[serde(default)]
    pub saved_queries: Vec<Value>,
    #[serde(default)]
    pub layouts: Map<String, Value>,
    #[serde(default)]
    pub views: Map<String, Value>,
}

impl Extras {
    pub fn is_empty(&self) -> bool {
        self.saved_queries.is_empty() && self.layouts.is_empty() && self.views.is_empty()
    }
}

/// Table preferences are keyed `connectionId:schema.table`.
fn owner(key: &str) -> &str {
    key.split(':').next().unwrap_or("")
}

fn with_owner(key: &str, id: &str) -> String {
    match key.split_once(':') {
        Some((_, rest)) => format!("{id}:{rest}"),
        None => key.to_string(),
    }
}

/// The saved queries (for these connections, or for any connection) and the views and layouts of
/// these connections' tables.
pub fn extras_for(data: &UserData, ids: &HashSet<String>) -> Extras {
    let mut out = Extras::default();
    if let Ok(Some(history)) = data.load("history") {
        out.saved_queries = history["saved"]
            .as_array()
            .map(|a| a.iter().filter(|q| q["connectionId"].as_str().is_none_or(|c| ids.contains(c))).cloned().collect())
            .unwrap_or_default();
    }
    if let Ok(Some(prefs)) = data.load("table-prefs") {
        for (field, target) in [("layouts", &mut out.layouts), ("views", &mut out.views)] {
            if let Some(m) = prefs[field].as_object() {
                for (k, v) in m {
                    if ids.contains(owner(k)) {
                        target.insert(k.clone(), v.clone());
                    }
                }
            }
        }
    }
    out
}

/// Adds imported saved queries and views, with connection ids mapped to the ones on this
/// computer. Nothing already here is overwritten. Returns how many queries and views were added.
pub fn merge_extras(data: &UserData, extras: &Extras, id_map: &HashMap<String, String>) -> Result<(usize, usize)> {
    let mapped = |id: &str| id_map.get(id).cloned();
    let mut history = data.load("history")?.unwrap_or_else(|| serde_json::json!({ "entries": [], "saved": [] }));
    let mut saved: Vec<Value> = history["saved"].as_array().cloned().unwrap_or_default();
    let mut queries = 0;
    for q in &extras.saved_queries {
        let mut q = q.clone();
        let connection = match q["connectionId"].as_str() {
            None => None,
            Some(old) => match mapped(old) {
                Some(new) => Some(new),
                None => continue,
            },
        };
        q["connectionId"] = connection.clone().map(Value::String).unwrap_or(Value::Null);
        let same = |x: &Value| x["name"] == q["name"] && x["connectionId"] == q["connectionId"];
        if saved.iter().any(same) {
            continue;
        }
        if saved.iter().any(|x| x["id"] == q["id"]) {
            q["id"] = Value::String(uuid::Uuid::new_v4().simple().to_string());
        }
        saved.push(q);
        queries += 1;
    }
    if queries > 0 {
        history["saved"] = Value::Array(saved);
        data.save("history", &history)?;
    }

    let mut prefs = data.load("table-prefs")?.unwrap_or_else(|| serde_json::json!({ "layouts": {}, "views": {} }));
    let mut views = 0;
    let mut changed = false;
    for (field, incoming) in [("layouts", &extras.layouts), ("views", &extras.views)] {
        if !prefs[field].is_object() {
            prefs[field] = Value::Object(Map::new());
        }
        let here = prefs[field].as_object_mut().unwrap();
        for (k, v) in incoming {
            let Some(new) = mapped(owner(k)) else { continue };
            let key = with_owner(k, &new);
            match (field, here.get_mut(&key)) {
                ("views", Some(Value::Array(existing))) => {
                    for view in v.as_array().into_iter().flatten() {
                        if !existing.iter().any(|e| e["name"] == view["name"]) {
                            existing.push(view.clone());
                            views += 1;
                            changed = true;
                        }
                    }
                }
                (_, Some(_)) => {}
                (_, None) => {
                    if field == "views" {
                        views += v.as_array().map_or(0, Vec::len);
                    }
                    here.insert(key, v.clone());
                    changed = true;
                }
            }
        }
    }
    if changed {
        data.save("table-prefs", &prefs)?;
    }
    Ok((queries, views))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch(name: &str) -> UserData {
        let dir = std::env::temp_dir().join(format!("kiyi-userdata-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        UserData::new(&dir)
    }

    #[test]
    fn only_known_names_are_kept() {
        let d = scratch("names");
        assert!(d.load("history").unwrap().is_none());
        d.save("settings", &json!({ "theme": "dark" })).unwrap();
        assert_eq!(d.load("settings").unwrap().unwrap()["theme"], "dark");
        assert!(d.save("../connections", &json!([])).is_err());
    }

    #[test]
    fn saved_queries_and_views_follow_connections_to_new_ids() {
        let here = scratch("from");
        here.save(
            "history",
            &json!({ "entries": [{ "sql": "SELECT secret" }], "saved": [
                { "id": "q1", "name": "Paid", "sql": "SELECT 1", "connectionId": "old-a" },
                { "id": "q2", "name": "Other db", "sql": "SELECT 2", "connectionId": "old-b" },
                { "id": "q3", "name": "Anywhere", "sql": "SELECT 3", "connectionId": null }
            ]}),
        )
        .unwrap();
        here.save("table-prefs", &json!({ "layouts": { "old-a:public.orders": { "hidden": ["x"] } }, "views": { "old-a:public.orders": [{ "id": "v1", "name": "Big" }], "old-b:public.t": [] } }))
            .unwrap();
        let extras = extras_for(&here, &HashSet::from(["old-a".to_string()]));
        assert_eq!(extras.saved_queries.len(), 2, "this connection's and the any-connection one");
        assert_eq!(extras.views.len(), 1);
        assert!(!serde_json::to_string(&extras).unwrap().contains("SELECT secret"), "history stays behind");

        let there = scratch("to");
        there.save("history", &json!({ "entries": [], "saved": [{ "id": "q1", "name": "Mine", "sql": "x", "connectionId": "local" }] })).unwrap();
        there.save("table-prefs", &json!({ "layouts": {}, "views": { "new-a:public.orders": [{ "id": "v9", "name": "Small" }] } })).unwrap();
        let map = HashMap::from([("old-a".to_string(), "new-a".to_string())]);
        assert_eq!(merge_extras(&there, &extras, &map).unwrap(), (2, 1));
        let h = there.load("history").unwrap().unwrap();
        let saved = h["saved"].as_array().unwrap();
        assert_eq!(saved.len(), 3);
        let paid = saved.iter().find(|q| q["name"] == "Paid").unwrap();
        assert_eq!(paid["connectionId"], "new-a");
        assert_ne!(paid["id"], "q1", "a clashing id is replaced");
        let p = there.load("table-prefs").unwrap().unwrap();
        assert_eq!(p["views"]["new-a:public.orders"].as_array().unwrap().len(), 2);
        assert_eq!(p["layouts"]["new-a:public.orders"]["hidden"][0], "x");
        // Importing again changes nothing.
        assert_eq!(merge_extras(&there, &extras, &map).unwrap(), (0, 0));
    }
}
