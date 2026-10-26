//! Portable records and immutable output artifacts. All scopes are host-selected.
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Binding {
    pub id: String,
    #[serde(default)]
    pub write: bool,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Installation {
    #[serde(default)]
    pub datasets: BTreeMap<String, Binding>,
    pub remote: Option<Remote>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Remote {
    pub url: String,
    pub token: String,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub installations: BTreeMap<String, Installation>,
}
impl Config {
    pub fn load(root: &Path) -> Result<Self, String> {
        let path = root.join("platform.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if !meta.is_file()
                || meta.uid() != unsafe { libc::geteuid() }
                || meta.mode() & 0o077 != 0
            {
                return Err("platform.json must be an owner-only regular file (chmod 600)".into());
            }
        }
        let cfg: Self = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("invalid platform.json: {e}"))?;
        for (installation, config) in &cfg.installations {
            identifier(installation)?;
            for (alias, binding) in &config.datasets {
                identifier(alias)?;
                identifier(&binding.id)?;
                if alias == "private" {
                    return Err("private dataset cannot be rebound".into());
                }
            }
            if let Some(remote) = &config.remote {
                let url =
                    url::Url::parse(&remote.url).map_err(|_| "invalid dataset service URL")?;
                if url.scheme() != "https"
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || remote.token.is_empty()
                {
                    return Err("dataset service requires HTTPS and a scoped bearer token".into());
                }
            }
        }
        Ok(cfg)
    }
}

pub fn migrate(db: &mut Connection) -> Result<(), String> {
    let tx = db.transaction().map_err(err)?;
    tx.execute_batch(include_str!("../runtime/platform-schema.sql"))
        .map_err(err)?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS installation_grants(installation TEXT PRIMARY KEY, application TEXT NOT NULL, capabilities TEXT NOT NULL); PRAGMA user_version=3").map_err(err)?;
    tx.commit().map_err(err)
}
fn err(e: rusqlite::Error) -> String {
    e.to_string()
}
fn identifier(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        Err("identifier must contain 1–128 UTF-8 bytes without control characters".into())
    } else {
        Ok(())
    }
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    let s = v[key]
        .as_str()
        .ok_or_else(|| format!("{key} must be a string"))?;
    identifier(s)?;
    Ok(s)
}
fn new_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn get(db: &Connection, dataset: &str, id: &str) -> Result<Value, String> {
    identifier(id)?;
    db.query_row(
        "SELECT body,sequence FROM platform_artifacts WHERE dataset=?1 AND id=?2",
        params![dataset, id],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
    )
    .optional()
    .map_err(err)?
    .map(|(body, seq)| {
        let mut value: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
        value["sequence"] = json!(seq);
        Ok(value)
    })
    .unwrap_or_else(|| Err("artifact not found or not authorized".into()))
}
fn list(db: &Connection, dataset: &str, query: &Value) -> Result<Vec<Value>, String> {
    if !query.is_object() {
        return Err("artifact query must be an object".into());
    }
    for key in query.as_object().unwrap().keys() {
        if ![
            "applicationId",
            "installationId",
            "output",
            "project",
            "tag",
            "status",
            "limit",
        ]
        .contains(&key.as_str())
        {
            return Err(format!("unknown artifact filter: {key}"));
        }
    }
    let limit = match query.get("limit") {
        None => 20,
        Some(v) => v
            .as_u64()
            .filter(|n| (1..=100).contains(n))
            .ok_or("limit must be 1–100")?,
    };
    let mut filters = Vec::new();
    for key in [
        "applicationId",
        "installationId",
        "output",
        "project",
        "tag",
        "status",
    ] {
        filters.push(if query.get(key).is_some() {
            Some(text(query, key)?.to_string())
        } else {
            None
        });
    }
    let mut stmt=db.prepare("SELECT id FROM platform_artifacts WHERE dataset=?1 AND (?2 IS NULL OR application=?2) AND (?3 IS NULL OR installation=?3) AND (?4 IS NULL OR output=?4) AND (?5 IS NULL OR project=?5) AND (?6 IS NULL OR EXISTS(SELECT 1 FROM json_each(body,'$.tags') WHERE value=?6)) AND (?7 IS NULL OR status=?7) ORDER BY sequence DESC LIMIT ?8").map_err(err)?;
    let ids = stmt
        .query_map(
            params![
                dataset, filters[0], filters[1], filters[2], filters[3], filters[4], filters[5],
                limit
            ],
            |r| r.get::<_, String>(0),
        )
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    ids.iter().map(|id| get(db, dataset, id)).collect()
}

pub fn execute(
    db: &mut Connection,
    installation: &str,
    application: &str,
    outputs: &Value,
    config: &Installation,
    request: &Value,
) -> Result<Value, String> {
    if request.to_string().len() > 65536 {
        return Err("platform request exceeds 64 KiB".into());
    }
    let alias = text(request, "dataset")?;
    let action = text(request, "action")?;
    let (dataset, write) = if alias == "private" {
        (format!("private:{installation}"), true)
    } else {
        let binding = config.datasets.get(alias).ok_or("dataset not granted")?;
        (format!("shared:{}", binding.id), binding.write)
    };
    if ![
        "record.get",
        "artifact.get",
        "artifact.list",
        "artifact.lineage",
        "artifact.resolve",
    ]
    .contains(&action)
        && !write
    {
        return Err("dataset is read-only".into());
    }
    // Only shared datasets are remote: configuration, scheduling, private data stay local.
    if alias != "private"
        && let Some(remote) = &config.remote
    {
        let mut remote_request = request.clone();
        remote_request["expectedScope"] =
            json!({"installationId":installation,"applicationId":application,"datasetId":dataset});
        return remote_call(remote, &remote_request);
    }
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    let result = match action {
        "record.get" | "record.put" | "record.delete" => {
            let key = text(request, "key")?;
            let current: Option<(String,i64,bool)>=tx.query_row("SELECT value,version,deleted FROM platform_records WHERE dataset=?1 AND key=?2",params![dataset,key],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(err)?;
            if action == "record.get" {
                match current {
                    Some((value, version, deleted)) => {
                        json!({"value":serde_json::from_str::<Value>(&value).map_err(|e|e.to_string())?,"version":version,"deleted":deleted})
                    }
                    None => Value::Null,
                }
            } else {
                let expected = request
                    .get("expectedVersion")
                    .ok_or("expectedVersion is required (null to create)")?;
                if !expected.is_null()
                    && expected
                        .as_i64()
                        .is_none_or(|n| !(1..9_007_199_254_740_991).contains(&n))
                {
                    return Err("invalid expectedVersion".into());
                }
                if expected.as_i64() != current.as_ref().map(|c| c.1) {
                    return Err("conflict: record version changed".into());
                }
                let version = current
                    .map(|c| c.1)
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or("version exhausted")?;
                let value = if action == "record.delete" {
                    Value::Null
                } else {
                    request.get("value").ok_or("value required")?.clone()
                };
                let deleted = action == "record.delete";
                tx.execute("INSERT INTO platform_records(dataset,key,value,version,deleted) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(dataset,key) DO UPDATE SET value=excluded.value,version=excluded.version,deleted=excluded.deleted",params![dataset,key,value.to_string(),version,deleted]).map_err(err)?;
                json!({"value":value,"version":version,"deleted":deleted})
            }
        }
        "artifact.get" => get(&tx, &dataset, text(request, "id")?)?,
        "artifact.list" => json!(list(&tx, &dataset, &request["query"])?),
        "artifact.publish" => {
            let draft = &request["draft"];
            let object = draft
                .as_object()
                .ok_or("artifact draft must be an object")?;
            if object.keys().any(|k| {
                ![
                    "output",
                    "mediaType",
                    "content",
                    "project",
                    "tags",
                    "status",
                    "inputs",
                    "executionId",
                    "idempotencyKey",
                ]
                .contains(&k.as_str())
            }) {
                return Err("unknown artifact draft field".into());
            }
            let output = text(draft, "output")?;
            let media = text(draft, "mediaType")?;
            if let Some(declared) = outputs.get(output)
                && declared["mediaType"] != media
            {
                return Err("artifact mediaType differs from declared output".into());
            }
            if draft.get("content").is_none() {
                return Err("artifact content is required".into());
            }
            for key in ["project", "status", "executionId", "idempotencyKey"] {
                if draft.get(key).is_some() {
                    text(draft, key)?;
                }
            }
            for key in ["tags", "inputs"] {
                if let Some(values) = draft.get(key) {
                    let values = values
                        .as_array()
                        .filter(|v| v.len() <= 100)
                        .ok_or("tags and inputs must be arrays of at most 100 identifiers")?;
                    for value in values {
                        let value = value.as_str().ok_or("invalid artifact identifier")?;
                        identifier(value)?;
                        if key == "inputs" {
                            get(&tx, &dataset, value)?;
                        }
                    }
                }
            }
            let fingerprint = draft.to_string();
            let idempotency = draft["idempotencyKey"].as_str();
            let existing = if let Some(key) = idempotency {
                tx.query_row("SELECT id,fingerprint FROM platform_artifacts WHERE dataset=?1 AND installation=?2 AND idempotency=?3",params![dataset,installation,key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional().map_err(err)?
            } else {
                None
            };
            if let Some((id, previous)) = existing {
                if fingerprint != previous {
                    return Err("conflict: idempotency key reused for a different artifact".into());
                }
                get(&tx, &dataset, &id)?
            } else {
                let id = new_id()?;
                let created = chrono::Utc::now().timestamp_millis();
                let mut artifact = draft.clone();
                artifact["id"] = json!(id);
                artifact["applicationId"] = json!(application);
                artifact["installationId"] = json!(installation);
                artifact["datasetId"] = json!(dataset);
                artifact["createdAt"] = json!(created);
                if artifact.get("tags").is_none() {
                    artifact["tags"] = json!([]);
                }
                if artifact.get("inputs").is_none() {
                    artifact["inputs"] = json!([]);
                }
                tx.execute("INSERT INTO platform_artifacts(id,dataset,installation,application,output,project,status,created,body,idempotency,fingerprint) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![id,dataset,installation,application,output,draft["project"].as_str(),draft["status"].as_str(),created,artifact.to_string(),idempotency,fingerprint]).map_err(err)?;
                get(&tx, &dataset, &id)?
            }
        }
        "artifact.resolve" => {
            let execution = text(request, "executionId")?;
            let slot = text(request, "slot")?;
            let reference = &request["reference"];
            let encoded = reference.to_string();
            let existing=tx.query_row("SELECT reference,ids FROM platform_resolutions WHERE dataset=?1 AND installation=?2 AND execution=?3 AND slot=?4",params![dataset,installation,execution,slot],|r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional().map_err(err)?;
            let ids: Vec<String> = if let Some((previous, ids)) = existing {
                if previous != encoded {
                    return Err(
                        "conflict: execution input slot already has a different reference".into(),
                    );
                }
                serde_json::from_str(&ids).map_err(|e| e.to_string())?
            } else {
                let ids = match text(reference, "kind")? {
                    "permanent" => reference["ids"]
                        .as_array()
                        .filter(|ids| !ids.is_empty() && ids.len() <= 100)
                        .ok_or("permanent reference needs 1–100 IDs")?
                        .iter()
                        .map(|id| id.as_str().map(str::to_owned).ok_or("invalid artifact ID"))
                        .collect::<Result<Vec<_>, _>>()?,
                    "dynamic" => list(&tx, &dataset, &reference["query"])?
                        .iter()
                        .map(|a| a["id"].as_str().unwrap().to_string())
                        .collect(),
                    _ => return Err("unknown artifact reference kind".into()),
                };
                for id in &ids {
                    get(&tx, &dataset, id)?;
                }
                tx.execute(
                    "INSERT INTO platform_resolutions VALUES(?1,?2,?3,?4,?5,?6)",
                    params![
                        dataset,
                        installation,
                        execution,
                        slot,
                        encoded,
                        json!(ids).to_string()
                    ],
                )
                .map_err(err)?;
                ids
            };
            json!(
                ids.iter()
                    .map(|id| get(&tx, &dataset, id))
                    .collect::<Result<Vec<_>, _>>()?
            )
        }
        "artifact.lineage" => {
            let id = text(request, "id")?;
            let artifact = get(&tx, &dataset, id)?;
            let inputs = artifact["inputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| get(&tx, &dataset, v.as_str().unwrap()))
                .collect::<Result<Vec<_>, _>>()?;
            let mut stmt=tx.prepare("SELECT id FROM platform_artifacts WHERE dataset=?1 AND EXISTS(SELECT 1 FROM json_each(body,'$.inputs') WHERE value=?2) ORDER BY sequence DESC LIMIT 100").map_err(err)?;
            let ids = stmt
                .query_map(params![dataset, id], |r| r.get::<_, String>(0))
                .map_err(err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(err)?;
            let dependents = ids
                .iter()
                .map(|id| get(&tx, &dataset, id))
                .collect::<Result<Vec<_>, _>>()?;
            json!({"artifact":artifact,"inputs":inputs,"dependents":dependents})
        }
        _ => return Err("unknown platform operation".into()),
    };
    tx.commit().map_err(err)?;
    Ok(result)
}
fn remote_call(remote: &Remote, request: &Value) -> Result<Value, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?;
        let mut response = client
            .post(&remote.url)
            .bearer_auth(&remote.token)
            .json(request)
            .send()
            .await
            .map_err(|_| "shared dataset unavailable; no local fallback was used")?;
        let status = response.status();
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "dataset response failed")?
        {
            if bytes.len() + chunk.len() > 16 * 1024 * 1024 {
                return Err("dataset response exceeds 16 MiB".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let body: Value =
            serde_json::from_slice(&bytes).map_err(|_| "invalid dataset service response")?;
        if !status.is_success() || body.get("error").is_some() {
            return Err(body["error"]
                .as_str()
                .unwrap_or("dataset service failed")
                .to_string());
        }
        body.get("value")
            .cloned()
            .ok_or_else(|| "dataset service omitted value".into())
    })
}

#[cfg(test)]
#[path = "platform_tests.rs"]
mod platform_tests;
