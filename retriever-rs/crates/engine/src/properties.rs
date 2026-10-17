use crate::engine::Engine;
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

impl Engine {
    pub fn property_definitions(&self) -> Result<Value> {
        let db = self.store.db.lock();
        let mut stmt = db.prepare("SELECT DISTINCT j.value FROM workspace_meta m, json_each(CASE WHEN json_valid(m.tags) THEN m.tags ELSE '[]' END) j WHERE j.type='text' ORDER BY j.value COLLATE NOCASE LIMIT 500")?;
        let tags = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(json!({"properties": definitions(&db)?, "tags":tags}))
    }

    pub fn save_property(&self, body: &Value) -> Result<Value> {
        let name = body["name"].as_str().unwrap_or("").trim();
        let kind = body["kind"].as_str().unwrap_or("");
        if name.is_empty()
            || name.chars().count() > 60
            || name.chars().any(char::is_control)
            || ![
                "select",
                "multi_select",
                "text",
                "number",
                "date",
                "checkbox",
            ]
            .contains(&kind)
        {
            bail!("Choose a property name and supported type");
        }
        let mut options = Vec::<String>::new();
        if ["select", "multi_select"].contains(&kind) {
            let input = body["options"]
                .as_array()
                .context("Select properties need options")?;
            if input.len() > 50 {
                bail!("Use up to 50 options");
            }
            for value in input {
                let text = value.as_str().context("Options must be text")?.trim();
                if text.is_empty()
                    || text.chars().count() > 60
                    || text.chars().any(char::is_control)
                {
                    bail!("Options must contain 1–60 characters");
                }
                if options
                    .iter()
                    .any(|s| s.to_lowercase() == text.to_lowercase())
                {
                    bail!("Options must be unique");
                }
                options.push(text.into());
            }
        }
        let id = body["id"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        uuid::Uuid::parse_str(&id).context("Invalid property ID")?;
        let mut db = self.store.db.lock();
        let tx = db.transaction()?;
        let old: Option<(String, i64)> = tx
            .query_row(
                "SELECT kind,revision FROM workspace_properties WHERE id=?",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let revision = old.as_ref().map(|v| v.1).unwrap_or(0);
        if revision != body["revision"].as_i64().unwrap_or(0) {
            bail!("conflict: Property definition changed; reload before saving");
        }
        if old.as_ref().is_some_and(|v| v.0 != kind) {
            bail!("Create a new property to change its type");
        }
        if old.is_none()
            && tx.query_row("SELECT COUNT(*) FROM workspace_properties", [], |r| {
                r.get::<_, i64>(0)
            })? >= 64
        {
            bail!("The workspace supports up to 64 custom properties");
        }
        // Existing choices cannot disappear while files still refer to them.
        if old.is_some() && ["select", "multi_select"].contains(&kind) {
            let previous: String = tx.query_row(
                "SELECT options FROM workspace_properties WHERE id=?",
                [&id],
                |r| r.get(0),
            )?;
            for choice in serde_json::from_str::<Vec<String>>(&previous)? {
                if !options.contains(&choice) {
                    bail!("Existing options must be retained; add new options instead");
                }
            }
        }
        let duplicate: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM workspace_properties WHERE lower(name)=lower(?) AND id<>?)", params![name,id], |r| r.get(0))?;
        if duplicate {
            bail!("A property with that name already exists");
        }
        tx.execute("INSERT INTO workspace_properties(id,name,kind,options,revision) VALUES(?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,options=excluded.options,revision=excluded.revision", params![id,name,kind,json!(options).to_string(),revision+1])?;
        tx.commit()?;
        Ok(json!({"id":id,"name":name,"kind":kind,"options":options,"revision":revision+1}))
    }
}

fn definitions(db: &Connection) -> Result<Vec<Value>> {
    let mut statement = db.prepare("SELECT id,name,kind,options,revision FROM workspace_properties ORDER BY name COLLATE NOCASE,id")?;
    Ok(statement.query_map([], |r| Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"options":r.get::<_,String>(3)?,"revision":r.get::<_,i64>(4)?})))?
        .map(|r| { let mut v=r?; v["options"]=serde_json::from_str(v["options"].as_str().unwrap())?; Ok(v) }).collect::<Result<Vec<_>>>()?)
}

pub(crate) fn validate_values(db: &Connection, values: &Value) -> Result<()> {
    if let Some(priority) = values.get("priority") {
        if !priority
            .as_str()
            .is_some_and(|v| ["", "low", "medium", "high", "urgent"].contains(&v))
        {
            bail!("Invalid priority");
        }
    }
    for definition in definitions(db)? {
        let Some(value) = values
            .get(definition["id"].as_str().unwrap())
            .filter(|v| !v.is_null())
        else {
            continue;
        };
        let options = definition["options"].as_array().unwrap();
        let valid = match definition["kind"].as_str().unwrap() {
            "text" => value.as_str().is_some_and(|v| v.chars().count() <= 2000),
            "number" => value.as_f64().is_some_and(f64::is_finite),
            "checkbox" => value.is_boolean(),
            "date" => value.as_str().is_some_and(valid_date),
            "select" => value == "" || options.contains(value),
            "multi_select" => value.as_array().is_some_and(|v| {
                v.len() <= 50
                    && v.iter()
                        .enumerate()
                        .all(|(i, x)| options.contains(x) && !v[..i].contains(x))
            }),
            _ => false,
        };
        if !valid {
            bail!(
                "Invalid value for property {}",
                definition["name"].as_str().unwrap()
            );
        }
    }
    Ok(())
}

fn valid_date(value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(i, b)| i != 4 && i != 7 && !b.is_ascii_digit())
    {
        return false;
    }
    let year = value[..4].parse::<u32>().unwrap_or(0);
    let month = value[5..7].parse::<u32>().unwrap_or(0);
    let day = value[8..].parse::<u32>().unwrap_or(0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if year % 400 == 0 || (year % 4 == 0 && year % 100 != 0) {
                29
            } else {
                28
            }
        }
        _ => 0,
    };
    year > 0 && day > 0 && day <= days
}
