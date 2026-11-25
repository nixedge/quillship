use crate::{
    db::Store,
    document::{page_id, Document},
    journal::{Intent, State},
    notion::Notion,
};
use anyhow::{ensure, Context, Result};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Schema {
    pub id: String,
    pub properties: Map<String, Value>,
}

impl Schema {
    fn property(&self, key: &str) -> Result<(&str, &Value)> {
        let matches = self
            .properties
            .iter()
            .filter(|(name, p)| *name == key || p["id"] == key)
            .collect::<Vec<_>>();
        ensure!(
            matches.len() == 1,
            "unknown or ambiguous schema property {key}"
        );
        Ok((matches[0].0, matches[0].1))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Property {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub value: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Properties {
    pub id: String,
    pub source_id: String,
    pub last_edited_time: String,
    pub complete: bool,
    pub properties: BTreeMap<String, Property>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Query {
    pub source_id: String,
    pub request: Value,
    pub results: Vec<Value>,
    pub complete: bool,
    pub warnings: Vec<String>,
}

/// Remove ephemeral file download access from durable structured snapshots.
pub(crate) fn stable(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                stable(item);
            }
        }
        Value::Object(map) => {
            if map.get("type") == Some(&json!("file")) {
                if let Some(file) = map.get_mut("file").and_then(Value::as_object_mut) {
                    file.remove("url");
                    file.remove("expiry_time");
                }
            }
            for value in map.values_mut() {
                stable(value);
            }
        }
        _ => (),
    }
}

fn property_path(id: &str) -> Result<&str> {
    ensure!(
        !id.is_empty() && id.len() <= 256 && id != "." && id != "..",
        "invalid property identity"
    );
    let mut bytes = id.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            ensure!(
                bytes.next().is_some_and(|b| b.is_ascii_hexdigit())
                    && bytes.next().is_some_and(|b| b.is_ascii_hexdigit()),
                "invalid encoded property identity"
            );
        } else {
            ensure!(
                byte.is_ascii_alphanumeric() || b"-_.~:".contains(&byte),
                "property identity must be URL encoded as returned by Notion"
            );
        }
    }
    Ok(id)
}

impl Notion {
    pub async fn source_schema(&self, id: &str) -> Result<Schema> {
        let id = page_id(id)?;
        let value = self
            .request(Method::GET, &format!("/data_sources/{id}"), None)
            .await?;
        ensure!(
            value["object"] == "data_source"
                && page_id(value["id"].as_str().context("source missing ID")?)? == id,
            "unexpected data source identity/type"
        );
        let properties = value["properties"]
            .as_object()
            .context("missing source properties")?
            .clone();
        for property in properties.values() {
            property_path(
                property["id"]
                    .as_str()
                    .context("schema property missing ID")?,
            )?;
            property["type"]
                .as_str()
                .context("schema property missing type")?;
        }
        Ok(Schema { id, properties })
    }

    pub async fn record_properties(&self, id: &str) -> Result<Properties> {
        let id = page_id(id)?;
        let before = self
            .request(Method::GET, &format!("/pages/{id}"), None)
            .await?;
        ensure!(
            page_id(before["id"].as_str().context("page missing ID")?)? == id,
            "page identity mismatch"
        );
        let source_id = page_id(
            before["parent"]["data_source_id"]
                .as_str()
                .context("page is not in a data source")?,
        )?;
        let mut properties = BTreeMap::new();
        for (name, property) in before["properties"]
            .as_object()
            .context("missing record properties")?
        {
            let prop_id =
                property_path(property["id"].as_str().context("property missing ID")?)?.to_owned();
            let kind = property["type"]
                .as_str()
                .context("property missing type")?
                .to_owned();
            let mut value = property
                .get(&kind)
                .context("property missing typed value")?
                .clone();
            if ["title", "rich_text", "people", "relation", "rollup"].contains(&kind.as_str()) {
                let mut items = Vec::new();
                let mut cursor = None::<String>;
                let mut seen = HashSet::new();
                let mut finished = false;
                for _ in 0..1000 {
                    let mut path = format!("/pages/{id}/properties/{prop_id}?page_size=100");
                    if let Some(cursor) = &cursor {
                        let mut url =
                            reqwest::Url::parse(&format!("https://api.notion.com{path}"))?;
                        url.query_pairs_mut().append_pair("start_cursor", cursor);
                        path = format!("{}?{}", url.path(), url.query().unwrap_or_default());
                    }
                    let result = self.request(Method::GET, &path, None).await?;
                    ensure!(
                        result["object"] == "list",
                        "expected paginated property items"
                    );
                    for item in result["results"]
                        .as_array()
                        .context("missing property items")?
                    {
                        if kind == "rollup" {
                            items.push(item.clone());
                        } else {
                            items.push(
                                item.get(&kind)
                                    .context("missing typed property item")?
                                    .clone(),
                            );
                        }
                    }
                    if !result["has_more"]
                        .as_bool()
                        .context("missing property pagination flag")?
                    {
                        if kind == "rollup" {
                            let rollup = result
                                .pointer("/property_item/rollup")
                                .context("missing final rollup calculation")?;
                            ensure!(
                                rollup["type"] != "incomplete",
                                "rollup calculation incomplete"
                            );
                            value = json!({"calculation":rollup,"items":items});
                        } else {
                            value = json!(items);
                        }
                        finished = true;
                        break;
                    }
                    let next = result["next_cursor"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .context("missing property cursor")?
                        .to_owned();
                    ensure!(seen.insert(next.clone()), "repeated property cursor");
                    cursor = Some(next);
                }
                ensure!(
                    finished,
                    "property pagination limit reached; prior snapshot preserved"
                );
            }
            stable(&mut value);
            ensure!(
                properties
                    .insert(
                        prop_id.clone(),
                        Property {
                            id: prop_id,
                            name: name.clone(),
                            kind,
                            value
                        }
                    )
                    .is_none(),
                "duplicate property ID"
            );
        }
        let after = self
            .request(Method::GET, &format!("/pages/{id}"), None)
            .await?;
        let time = before["last_edited_time"]
            .as_str()
            .context("missing record edit time")?;
        ensure!(
            page_id(after["id"].as_str().context("page missing ID")?)? == id
                && after["last_edited_time"] == time,
            "record changed during property retrieval"
        );
        Ok(Properties {
            id,
            source_id,
            last_edited_time: time.into(),
            complete: true,
            properties,
        })
    }
}

impl Store {
    pub fn source(&self, id: &str) -> Result<Schema> {
        self.cache_get("schemas-v1", &page_id(id)?)
    }
    pub fn properties(&self, id: &str) -> Result<Properties> {
        self.cache_get("record-properties-v1", &page_id(id)?)
    }
    pub(crate) fn save_properties(&self, value: &Properties) -> Result<()> {
        ensure!(
            value.complete,
            "cannot cache an incomplete property baseline"
        );
        self.cache_save("record-properties-v1", &value.id, value)
    }
}

pub async fn database(store: &Store, notion: Option<&Notion>, id: &str) -> Result<Value> {
    let id = page_id(id)?;
    if let Some(notion) = notion {
        let value = notion
            .request(Method::GET, &format!("/databases/{id}"), None)
            .await?;
        ensure!(
            value["object"] == "database"
                && page_id(value["id"].as_str().context("database missing ID")?)? == id,
            "database identity/type mismatch"
        );
        let sources = value["data_sources"]
            .as_array()
            .context("missing data sources")?;
        for source in sources {
            page_id(source["id"].as_str().context("missing source ID")?)?;
        }
        let value = json!({"id":id,"data_sources":sources,"parent":value["parent"]});
        store.cache_save("database-containers-v1", &id, &value)?;
        Ok(value)
    } else {
        store.cache_get("database-containers-v1", &id)
    }
}

pub async fn source(store: &Store, notion: Option<&Notion>, id: &str) -> Result<Schema> {
    if let Some(notion) = notion {
        let schema = notion.source_schema(id).await?;
        store.cache_save("schemas-v1", &schema.id, &schema)?;
        Ok(schema)
    } else {
        store.source(id)
    }
}

pub async fn properties(store: &Store, notion: Option<&Notion>, id: &str) -> Result<Properties> {
    if let Some(notion) = notion {
        let value = notion.record_properties(id).await?;
        store.save_properties(&value)?;
        Ok(value)
    } else {
        store.properties(id)
    }
}

pub fn validate_properties(schema: &Schema, input: &Value) -> Result<Value> {
    let input = input
        .as_object()
        .context("properties must be an object of native typed values")?;
    ensure!(!input.is_empty(), "select at least one property");
    let mut output = Map::new();
    for (key, value) in input {
        let (_, property) = schema.property(key)?;
        let kind = property["type"].as_str().context("missing property type")?;
        let object = value
            .as_object()
            .context("property entry must be a typed object")?;
        ensure!(
            object.len() == 1 && object.contains_key(kind),
            "property {key} must contain only its schema type {kind}"
        );
        let value = &object[kind];
        match kind {
            "title" | "rich_text" => {
                let parts = value.as_array().context("text property must be an array")?;
                ensure!(parts.len() <= 100, "too many rich text items");
                for part in parts {
                    let part = part.as_object().context("text item must be an object")?;
                    ensure!(
                        part.keys()
                            .all(|k| ["type", "text", "annotations"].contains(&k.as_str())),
                        "only text rich-text writes are supported"
                    );
                    ensure!(
                        part.get("type").is_none_or(|t| t == "text"),
                        "only text rich-text writes are supported"
                    );
                    let text = part
                        .get("text")
                        .and_then(Value::as_object)
                        .context("text item needs text.content")?;
                    ensure!(
                        text.keys()
                            .all(|k| ["content", "link"].contains(&k.as_str()))
                            && text
                                .get("content")
                                .and_then(Value::as_str)
                                .is_some_and(|s| s.chars().count() <= 2000),
                        "invalid text content"
                    );
                    if let Some(link) = text.get("link").filter(|v| !v.is_null()) {
                        reqwest::Url::parse(link["url"].as_str().context("link needs URL")?)?;
                    }
                }
            }
            "number" => ensure!(
                value.is_number() || value.is_null(),
                "number must be numeric or null"
            ),
            "checkbox" => ensure!(value.is_boolean(), "checkbox must be boolean"),
            "url" => {
                if !value.is_null() {
                    reqwest::Url::parse(value.as_str().context("URL must be a string or null")?)?;
                }
            }
            "date" => {
                if !value.is_null() {
                    let date = value
                        .as_object()
                        .context("date must be an object or null")?;
                    ensure!(
                        date.keys()
                            .all(|k| ["start", "end", "time_zone"].contains(&k.as_str()))
                            && date
                                .get("start")
                                .and_then(Value::as_str)
                                .is_some_and(|s| !s.is_empty()),
                        "date needs start and supported fields"
                    );
                    for key in ["end", "time_zone"] {
                        ensure!(
                            date.get(key).is_none_or(|v| v.is_null() || v.is_string()),
                            "invalid date field"
                        );
                    }
                }
            }
            "select" | "status" => {
                if !value.is_null() {
                    let option = value
                        .as_object()
                        .context("option must be an object or null")?;
                    ensure!(
                        option.len() == 1
                            && (option.contains_key("id") || option.contains_key("name")),
                        "select/status needs exactly id or name"
                    );
                    let options = property[kind]["options"]
                        .as_array()
                        .context("schema omitted options")?;
                    ensure!(
                        options
                            .iter()
                            .any(|o| option.iter().all(|(k, v)| o.get(k) == Some(v))),
                        "option is absent from schema; schema changes are not implicit"
                    );
                }
            }
            "relation" => {
                let ids = value.as_array().context("relation must be an array")?;
                ensure!(ids.len() <= 100, "relation write exceeds 100 items");
                let mut seen = HashSet::new();
                for relation in ids {
                    ensure!(
                        relation.as_object().is_some_and(|o| o.len() == 1),
                        "relation entries contain only id"
                    );
                    let id = page_id(relation["id"].as_str().context("relation missing ID")?)?;
                    ensure!(seen.insert(id), "duplicate relation identity");
                }
            }
            _ => anyhow::bail!("property type {kind} is read-only/unsupported for writes"),
        }
        let id = property["id"]
            .as_str()
            .context("schema missing property ID")?
            .to_owned();
        ensure!(
            output.insert(id, json!({(kind):value})).is_none(),
            "same property selected twice"
        );
    }
    Ok(Value::Object(output))
}

/// Validate native filters against schema types and supported value/operator pairs.
pub fn validate_filter(schema: &Schema, value: &Value, depth: usize) -> Result<()> {
    ensure!(depth <= 2, "filter nesting exceeds supported depth");
    let object = value.as_object().context("filter must be an object")?;
    if let Some(group) = object.get("and").or_else(|| object.get("or")) {
        ensure!(object.len() == 1, "compound filter contains unrelated keys");
        let items = group.as_array().context("filter group must be an array")?;
        ensure!(
            !items.is_empty() && items.len() <= 100,
            "filter group must contain 1..100 entries"
        );
        for item in items {
            validate_filter(schema, item, depth + 1)?;
        }
        return Ok(());
    }
    let key = object
        .get("property")
        .and_then(Value::as_str)
        .context("filter needs property")?;
    let (_, property) = schema.property(key)?;
    let kind = property["type"].as_str().context("missing property type")?;
    ensure!(
        object.len() == 2 && object.contains_key(kind),
        "filter must match schema type {kind}"
    );
    let condition = object[kind]
        .as_object()
        .context("filter condition must be an object")?;
    ensure!(
        condition.len() == 1,
        "one typed condition per predicate is supported"
    );
    let (op, operand) = condition.iter().next().expect("one condition");
    let valid = if ["is_empty", "is_not_empty"].contains(&op.as_str()) {
        operand == true
            && [
                "title",
                "rich_text",
                "number",
                "url",
                "date",
                "select",
                "status",
                "relation",
            ]
            .contains(&kind)
    } else {
        match kind {
            "checkbox" => {
                ["equals", "does_not_equal"].contains(&op.as_str()) && operand.is_boolean()
            }
            "number" => {
                [
                    "equals",
                    "does_not_equal",
                    "greater_than",
                    "less_than",
                    "greater_than_or_equal_to",
                    "less_than_or_equal_to",
                ]
                .contains(&op.as_str())
                    && operand.is_number()
            }
            "title" | "rich_text" | "url" => {
                [
                    "equals",
                    "does_not_equal",
                    "contains",
                    "does_not_contain",
                    "starts_with",
                    "ends_with",
                ]
                .contains(&op.as_str())
                    && operand.is_string()
            }
            "select" | "status" => {
                ["equals", "does_not_equal"].contains(&op.as_str()) && operand.is_string()
            }
            "date" => {
                ["equals", "before", "after", "on_or_before", "on_or_after"].contains(&op.as_str())
                    && operand.is_string()
            }
            "relation" => {
                ["contains", "does_not_contain"].contains(&op.as_str())
                    && operand.as_str().is_some_and(|s| page_id(s).is_ok())
            }
            _ => false,
        }
    };
    ensure!(valid, "unsupported or incorrectly typed {kind} filter {op}");
    Ok(())
}

pub async fn query(
    store: &Store,
    notion: Option<&Notion>,
    id: &str,
    filter: Option<Value>,
    sorts: Option<Value>,
    limit: usize,
) -> Result<Query> {
    let id = page_id(id)?;
    ensure!((1..=10000).contains(&limit), "query limit must be 1..10000");
    let mut request = json!({"page_size":100});
    if let Some(filter) = filter {
        request["filter"] = filter;
    }
    if let Some(sorts) = sorts {
        request["sorts"] = sorts;
    }
    let key = format!("{id}:{limit}:{}", serde_json::to_string(&request)?);
    let Some(notion) = notion else {
        return store.cache_get("queries-v1", &key);
    };
    let schema = source(store, Some(notion), &id).await?;
    if let Some(filter) = request.get("filter") {
        validate_filter(&schema, filter, 0)?;
    }
    if let Some(sorts) = request.get("sorts") {
        for sort in sorts.as_array().context("sorts must be an array")? {
            let sort = sort.as_object().context("sort must be an object")?;
            ensure!(
                sort.len() == 2
                    && sort
                        .get("direction")
                        .is_some_and(|d| d == "ascending" || d == "descending"),
                "invalid sort direction/fields"
            );
            if let Some(property) = sort.get("property") {
                schema.property(
                    property
                        .as_str()
                        .context("sort property must be a string")?,
                )?;
            } else {
                ensure!(
                    sort.get("timestamp")
                        .is_some_and(|t| t == "created_time" || t == "last_edited_time"),
                    "invalid timestamp sort"
                );
            }
        }
    }
    let mut result = Query {
        source_id: id.clone(),
        request: request.clone(),
        results: Vec::new(),
        complete: false,
        warnings: Vec::new(),
    };
    let mut cursors = HashSet::new();
    let mut identities = HashSet::new();
    for _ in 0..1000 {
        request["page_size"] = json!((limit - result.results.len()).min(100));
        let page = notion
            .request(
                Method::POST,
                &format!("/data_sources/{id}/query"),
                Some(request.clone()),
            )
            .await?;
        if page
            .pointer("/request_status/type")
            .is_some_and(|v| v != "complete")
        {
            result
                .warnings
                .push("Notion capped/marked this query incomplete".into());
        }
        let rows = page["results"]
            .as_array()
            .context("query missing results")?;
        ensure!(
            rows.len() <= (limit - result.results.len()).min(100),
            "query exceeded requested result limit"
        );
        for row in rows {
            let row_id = page_id(row["id"].as_str().context("query result missing ID")?)?;
            ensure!(
                identities.insert(row_id),
                "duplicate query identity; incomplete snapshot"
            );
            let mut row = row.clone();
            stable(&mut row);
            result.results.push(row);
        }
        let more = page["has_more"]
            .as_bool()
            .context("query missing pagination flag")?;
        if !more {
            result.complete = result.warnings.is_empty();
            break;
        }
        if result.results.len() == limit {
            result
                .warnings
                .push("requested result limit reached; more rows exist".into());
            break;
        }
        let cursor = page["next_cursor"]
            .as_str()
            .filter(|s| !s.is_empty())
            .context("query missing cursor")?
            .to_owned();
        ensure!(cursors.insert(cursor.clone()), "repeated query cursor");
        request["start_cursor"] = json!(cursor);
    }
    if !result.complete && result.warnings.is_empty() {
        result
            .warnings
            .push("query pagination budget reached".into());
    }
    if result.complete {
        store.cache_save("queries-v1", &key, &result)?;
    }
    Ok(result)
}

pub async fn create(
    store: &Store,
    notion: &Notion,
    source_id: &str,
    input: &Value,
    body: Option<String>,
) -> Result<Properties> {
    let schema = source(store, Some(notion), source_id).await?;
    let properties = validate_properties(&schema, input)?;
    ensure!(
        body.as_ref().is_none_or(|b| !b.contains("<unknown")),
        "cannot create unknown placeholders"
    );
    let mut op = store.prepare(
        Intent::RecordCreate {
            source: schema.id.clone(),
            properties: properties.clone(),
            markdown: body.clone(),
        },
        None,
    )?;
    let result=async {
        store.transition(&mut op,State::Sent)?;
        let mut request=json!({"parent":{"type":"data_source_id","data_source_id":schema.id},"properties":properties,"notifications":{"mode":"silent"},"allow_async":false});
        if let Some(body)=&body {request["markdown"]=json!(body);request["markdown_version"]=json!("v2");}
        let value=notion.request(Method::POST,"/pages",Some(request)).await?;
        let id=page_id(value["id"].as_str().context("created record missing identity")?)?;
        eprintln!("created record {id} (operation {})",op.id);op.remote_id=Some(id.clone());store.transition(&mut op,State::RemoteConfirmed)?;
        let record=notion.record_properties(&id).await?;ensure!(record.source_id==schema.id,"created record source mismatch");store.save_properties(&record)?;
        if let Some(body)=&body {let mut doc=Document::from_remote(notion.retrieve(&id).await?);doc.markdown=body.clone();store.save(&doc,None)?;}
        store.transition(&mut op,State::LocallySaved)?;Ok(record)
    }.await;
    store.operation_result(&mut op, result)
}

pub async fn update(store: &Store, notion: &Notion, id: &str, input: &Value) -> Result<Properties> {
    let before = store.properties(id)?;
    ensure!(before.complete, "complete property baseline required");
    let schema = source(store, Some(notion), &before.source_id).await?;
    let desired = validate_properties(&schema, input)?;
    let fresh = notion.record_properties(&before.id).await?;
    ensure!(
        fresh.source_id == before.source_id,
        "record moved to a different source"
    );
    let mut expected = Map::new();
    for key in desired.as_object().expect("validated object").keys() {
        let old = before
            .properties
            .get(key)
            .context("selected property absent from baseline")?;
        let current = fresh
            .properties
            .get(key)
            .context("selected property no longer exists")?;
        ensure!(
            old.kind == current.kind
                && old.value == current.value
                && desired[key].get(&current.kind).is_some(),
            "selected property {key} changed remotely; fetch/review properties before retrying"
        );
        expected.insert(key.clone(), serde_json::to_value(current)?);
    }
    let mut op = store.prepare(
        Intent::RecordUpdate {
            id: before.id.clone(),
            source: before.source_id,
            properties: desired.clone(),
            expected: Value::Object(expected),
        },
        None,
    )?;
    let result = async {
        store.transition(&mut op, State::Sent)?;
        let value = notion
            .request(
                Method::PATCH,
                &format!("/pages/{}", before.id),
                Some(json!({"properties":desired})),
            )
            .await?;
        ensure!(
            page_id(value["id"].as_str().context("updated record missing ID")?)? == before.id,
            "updated record identity mismatch"
        );
        op.remote_id = Some(before.id.clone());
        store.transition(&mut op, State::RemoteConfirmed)?;
        let record = notion.record_properties(&before.id).await?;
        ensure!(
            record.source_id == schema.id,
            "record source changed during update"
        );
        store.save_properties(&record)?;
        store.transition(&mut op, State::LocallySaved)?;
        Ok(record)
    }
    .await;
    store.operation_result(&mut op, result)
}

pub async fn move_record(
    store: &Store,
    notion: &Notion,
    id: &str,
    source_id: &str,
) -> Result<crate::notion::PageInfo> {
    let id = page_id(id)?;
    let schema = source(store, Some(notion), source_id).await?;
    let mut op = store.prepare(
        Intent::RecordMove {
            id: id.clone(),
            source: schema.id.clone(),
        },
        None,
    )?;
    let result = async {
        store.transition(&mut op, State::Sent)?;
        let response = notion
            .request(
                Method::POST,
                &format!("/pages/{id}/move"),
                Some(json!({"parent":{"type":"data_source_id","data_source_id":schema.id}})),
            )
            .await?;
        ensure!(
            page_id(response["id"].as_str().context("move missing page ID")?)? == id,
            "record move identity mismatch"
        );
        eprintln!("moved {id} into source {}", schema.id);
        op.remote_id = Some(id.clone());
        store.transition(&mut op, State::RemoteConfirmed)?;
        let info = notion.inspect(&id).await?;
        ensure!(
            info.parent
                .as_ref()
                .and_then(|p| p["data_source_id"].as_str())
                .and_then(|id| page_id(id).ok())
                .as_deref()
                == Some(&schema.id),
            "move readback source differs"
        );
        store.save_metadata(&info)?;
        store.transition(&mut op, State::LocallySaved)?;
        Ok(info)
    }
    .await;
    store.operation_result(&mut op, result)
}

fn requested_matches(requested: &Value, actual: &Value) -> bool {
    match (requested, actual) {
        (Value::Object(a), Value::Object(b)) => a.iter().all(|(key, value)| {
            b.get(key)
                .is_some_and(|actual| requested_matches(value, actual))
        }),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| requested_matches(a, b))
        }
        _ => requested == actual,
    }
}

pub(crate) async fn recover_record(
    store: &Store,
    notion: &Notion,
    op: &mut crate::journal::Operation,
    target: &str,
) -> Result<()> {
    let record = notion.record_properties(target).await?;
    match op.intent.clone() {
        Intent::RecordCreate {
            source, markdown, ..
        } => {
            ensure!(
                record.source_id == source,
                "created record source changed; reconcile explicitly"
            );
            if op.state != State::RemoteConfirmed {
                store.transition(op, State::RemoteConfirmed)?;
            }
            if let Some(body) = markdown {
                let remote = notion.retrieve(target).await?;
                if store.contains(target)? {
                    ensure!(
                        store.get(target)?.baseline == remote.markdown,
                        "body baseline changed; reconcile explicitly"
                    );
                } else {
                    let mut doc = Document::from_remote(remote);
                    doc.markdown = body;
                    store.save(&doc, None)?;
                }
            }
        }
        Intent::RecordUpdate {
            source, properties, ..
        } => {
            ensure!(
                record.source_id == source,
                "record moved to another source; reconcile explicitly"
            );
            for (id, value) in properties
                .as_object()
                .context("invalid journal property input")?
            {
                let actual = record
                    .properties
                    .get(id)
                    .context("selected property disappeared")?;
                let requested = value
                    .get(&actual.kind)
                    .context("selected property type changed")?;
                ensure!(requested_matches(requested,&actual.value),"property outcome differs/was normalized; inspect properties before an explicit retry");
            }
            if op.state != State::RemoteConfirmed {
                store.transition(op, State::RemoteConfirmed)?;
            }
        }
        _ => anyhow::bail!("not a record operation"),
    }
    store.save_properties(&record)
}
