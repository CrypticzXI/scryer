use crate::{SeedDevArgs, TaskContext};
use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs;
use std::thread;
use std::time::Duration;

pub(crate) fn run(ctx: &TaskContext, args: SeedDevArgs) -> Result<()> {
    crate::require_command("curl")?;

    let scryer_url = env::var("SCRYER_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    let graphql_url = format!("{scryer_url}/graphql");
    let seed_file = args.file.unwrap_or_else(|| "/seed.json".into());

    if !seed_file.is_file() {
        println!(
            "seed: no seed file found at {} — skipping",
            seed_file.display()
        );
        return Ok(());
    }

    let seed: SeedFile = serde_json::from_slice(
        &fs::read(&seed_file).with_context(|| format!("failed to read {}", seed_file.display()))?,
    )
    .with_context(|| format!("failed to parse {}", seed_file.display()))?;

    wait_for_scryer(ctx, &scryer_url)?;

    let mut aliases = HashMap::new();

    let total = seed.indexers.len()
        + seed.download_clients.len()
        + seed.settings.len()
        + seed.titles.movies.len()
        + seed.titles.series.len()
        + seed.titles.anime.len();
    println!(
        "seed: applying {total} operations from {}",
        seed_file
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("seed.json")
    );

    seed_indexers(ctx, &graphql_url, &seed.indexers, &mut aliases)?;
    seed_download_clients(ctx, &graphql_url, &seed.download_clients, &mut aliases)?;
    seed_settings(ctx, &graphql_url, &seed.settings, &aliases)?;
    seed_titles_for_facet(ctx, &graphql_url, &seed.titles.movies, "movie", "movie")?;
    seed_titles_for_facet(ctx, &graphql_url, &seed.titles.series, "series", "series")?;
    seed_titles_for_facet(ctx, &graphql_url, &seed.titles.anime, "anime", "anime")?;

    println!("seed: completed successfully ({total} entities seeded)");
    Ok(())
}

#[derive(Default, serde::Deserialize)]
struct SeedFile {
    #[serde(default)]
    indexers: Vec<Value>,
    #[serde(rename = "downloadClients", default)]
    download_clients: Vec<Value>,
    #[serde(default)]
    settings: Vec<Value>,
    #[serde(default)]
    titles: SeedTitles,
}

#[derive(Default, serde::Deserialize)]
struct SeedTitles {
    #[serde(default)]
    movies: Vec<Value>,
    #[serde(default)]
    series: Vec<Value>,
    #[serde(default)]
    anime: Vec<Value>,
}

struct GraphqlBatch {
    var_defs: Vec<String>,
    fields: Vec<String>,
    variables: Map<String, Value>,
}

impl GraphqlBatch {
    fn new() -> Self {
        Self {
            var_defs: Vec::new(),
            fields: Vec::new(),
            variables: Map::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    fn add(&mut self, input_type: &str, field_name: &str, selection: &str, input_json: Value) {
        let index = self.fields.len();
        let alias = format!("op{index}");
        let variable = format!("input{index}");
        self.var_defs.push(format!("${variable}: {input_type}!"));
        self.fields.push(format!(
            "{alias}: {field_name}(input: ${variable}) {selection}"
        ));
        self.variables.insert(variable, input_json);
    }

    fn execute(self, ctx: &TaskContext, graphql_url: &str, label: &str) -> Result<Value> {
        if self.is_empty() {
            return Ok(json!({ "data": {} }));
        }

        eprintln!(
            "seed: sending batched {label} request ({} operations)",
            self.fields.len()
        );
        let query = format!(
            "mutation SeedBatch({}) {{ {} }}",
            self.var_defs.join(", "),
            self.fields.join(" ")
        );
        graphql_request(ctx, graphql_url, &query, Value::Object(self.variables))
    }
}

fn wait_for_scryer(ctx: &TaskContext, scryer_url: &str) -> Result<()> {
    println!("seed: waiting for scryer at {scryer_url} ...");
    let health_url = format!("{scryer_url}/health");
    let max_attempts = 60usize;

    for attempt in 0..max_attempts {
        let mut command = ctx.command("curl");
        command.args(["-sf", &health_url]);
        if let Ok(output) = command.output()
            && output.status.success()
            && let Ok(value) = serde_json::from_slice::<Value>(&output.stdout)
            && value.get("status").and_then(Value::as_str) == Some("ok")
        {
            println!("seed: scryer is ready");
            return Ok(());
        }

        if attempt + 1 < max_attempts {
            thread::sleep(Duration::from_secs(2));
        }
    }

    bail!("seed: scryer did not become healthy after {max_attempts} attempts")
}

fn graphql_request(
    ctx: &TaskContext,
    graphql_url: &str,
    query: &str,
    variables: Value,
) -> Result<Value> {
    let payload = serde_json::to_string(&json!({
        "query": query,
        "variables": variables,
    }))?;
    let authless_proof = authless_web_client_proof(ctx, graphql_url)?;

    let mut command = ctx.command("curl");
    command.args([
        "-fsS",
        "-X",
        "POST",
        graphql_url,
        "-H",
        "Content-Type: application/json",
    ]);
    if let Some((cookie, proof)) = authless_proof {
        command.args([
            "-H",
            &format!("x-scryer-web-client: {proof}"),
            "-H",
            &format!("Cookie: {cookie}"),
        ]);
    }
    let output = command
        .args(["--data-binary", &payload])
        .output()
        .context("GraphQL request failed")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("GraphQL request failed\n{stderr}");
    }

    let response: Value = serde_json::from_slice(&output.stdout).context("invalid GraphQL JSON")?;
    if let Some(errors) = response.get("errors").and_then(Value::as_array)
        && !errors.is_empty()
    {
        let mut message = format!("GraphQL request returned {} errors", errors.len());
        for error in errors {
            if let Some(text) = error.get("message").and_then(Value::as_str) {
                message.push_str("\n  - ");
                message.push_str(text);
            }
        }
        bail!("{message}");
    }

    Ok(response)
}

fn authless_web_client_proof(
    ctx: &TaskContext,
    graphql_url: &str,
) -> Result<Option<(String, String)>> {
    let Some(base_url) = graphql_url.strip_suffix("/graphql") else {
        return Ok(None);
    };
    let output = ctx
        .command("curl")
        .args(["-fsS", "-D", "-", &format!("{base_url}/authless-client")])
        .output()
        .context("authless web-client proof request failed")?;
    if !output.status.success() {
        return Ok(None);
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let Some((headers, body)) = raw
        .split_once("\r\n\r\n")
        .or_else(|| raw.split_once("\n\n"))
    else {
        return Ok(None);
    };
    let cookie = headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if !name.trim().eq_ignore_ascii_case("set-cookie") {
            return None;
        }
        value
            .trim()
            .split(';')
            .next()
            .map(str::trim)
            .filter(|cookie| cookie.starts_with("scryer_authless_client="))
            .map(str::to_string)
    });
    let proof = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("proof")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .filter(|value| !value.trim().is_empty());

    Ok(cookie.zip(proof))
}

fn seed_indexers(
    ctx: &TaskContext,
    graphql_url: &str,
    entries: &[Value],
    aliases: &mut HashMap<String, String>,
) -> Result<()> {
    let mut batch = GraphqlBatch::new();
    for entry in entries {
        let name = required_string(entry, "name")?;
        println!("seed: creating indexer '{name}'");
        batch.add(
            "CreateIndexerConfigInput",
            "createIndexerConfig",
            "{ id name }",
            json!({
                "name": name,
                "providerType": required_string(entry, "providerType")?,
                "rateLimitSeconds": present_or_null(entry, "rateLimitSeconds"),
                "rateLimitBurst": present_or_null(entry, "rateLimitBurst"),
                "isEnabled": first_present(entry, &["enabled", "isEnabled"]),
                "enableInteractiveSearch": present_or_null(entry, "enableInteractiveSearch"),
                "enableAutoSearch": present_or_null(entry, "enableAutoSearch"),
                "config": indexer_config_value_input(entry)?,
            }),
        );
    }

    let response = batch.execute(ctx, graphql_url, "indexer create")?;
    for (index, entry) in entries.iter().enumerate() {
        let alias = format!("op{index}");
        let id = response
            .get("data")
            .and_then(|data| data.get(&alias))
            .and_then(|op| op.get("id"))
            .and_then(Value::as_str)
            .with_context(|| format!("missing id in response for {alias}"))?
            .to_string();
        let name = required_string(entry, "name")?;
        add_aliases(
            aliases,
            &id,
            &name,
            entry.get("seedId").and_then(Value::as_str),
        );
    }

    Ok(())
}

fn seed_download_clients(
    ctx: &TaskContext,
    graphql_url: &str,
    entries: &[Value],
    aliases: &mut HashMap<String, String>,
) -> Result<()> {
    let mut batch = GraphqlBatch::new();
    for entry in entries {
        let name = required_string(entry, "name")?;
        println!("seed: creating download client '{name}'");
        batch.add(
            "CreateDownloadClientConfigInput",
            "createDownloadClientConfig",
            "{ id name clientType }",
            json!({
                "name": name,
                "clientType": required_string(entry, "clientType")?,
                "config": config_value_input(entry)?,
                "isEnabled": first_present(entry, &["enabled", "isEnabled"]),
            }),
        );
    }

    let response = batch.execute(ctx, graphql_url, "download client create")?;
    for (index, entry) in entries.iter().enumerate() {
        let alias = format!("op{index}");
        let id = response
            .get("data")
            .and_then(|data| data.get(&alias))
            .and_then(|op| op.get("id"))
            .and_then(Value::as_str)
            .with_context(|| format!("missing id in response for {alias}"))?
            .to_string();
        let name = required_string(entry, "name")?;
        add_aliases(
            aliases,
            &id,
            &name,
            entry.get("seedId").and_then(Value::as_str),
        );
    }

    Ok(())
}

fn seed_settings(
    ctx: &TaskContext,
    graphql_url: &str,
    entries: &[Value],
    aliases: &HashMap<String, String>,
) -> Result<()> {
    let mut movie_path = None::<String>;
    let mut series_path = None::<String>;
    let mut anime_path = None::<String>;
    let mut batch = GraphqlBatch::new();

    for entry in entries {
        let key = required_string(entry, "key")?;
        let scope_id = entry
            .get("scopeId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        match key.as_str() {
            "movies.path" => movie_path = Some(setting_value_string(entry)),
            "series.path" => series_path = Some(setting_value_string(entry)),
            "anime.path" => anime_path = Some(setting_value_string(entry)),
            "download_client.routing" => {
                if scope_id.is_empty() {
                    bail!("seed: download_client.routing requires scopeId");
                }
                println!("seed: saving setting '{key}' (system/{scope_id})");
                batch.add(
                    "UpdateDownloadClientRoutingInput",
                    "updateDownloadClientRouting",
                    "{ clientId }",
                    json!({
                        "scope": scope_id,
                        "entries": download_client_routing_entries(entry, aliases)?,
                    }),
                );
            }
            "indexer.routing" => {
                if scope_id.is_empty() {
                    bail!("seed: indexer.routing requires scopeId");
                }
                println!("seed: saving setting '{key}' (system/{scope_id})");
                batch.add(
                    "UpdateIndexerRoutingInput",
                    "updateIndexerRouting",
                    "{ indexerId }",
                    json!({
                        "scope": scope_id,
                        "entries": indexer_routing_entries(entry, aliases)?,
                    }),
                );
            }
            _ => bail!("seed: unsupported typed setting '{key}'"),
        }
    }

    if movie_path.is_some() || series_path.is_some() || anime_path.is_some() {
        let movie_path = movie_path.unwrap_or_else(|| "/data/movies".to_string());
        let series_path = series_path.unwrap_or_else(|| "/data/series".to_string());
        println!("seed: saving media library paths");
        batch.add(
            "UpdateLibraryPathsInput",
            "updateLibraryPaths",
            "{ moviePath seriesPath animePath }",
            json!({
                "moviePath": movie_path,
                "seriesPath": series_path,
                "animePath": anime_path,
            }),
        );
    }

    let _ = batch.execute(ctx, graphql_url, "settings update")?;
    Ok(())
}

fn seed_titles_for_facet(
    ctx: &TaskContext,
    graphql_url: &str,
    entries: &[Value],
    facet: &str,
    label: &str,
) -> Result<()> {
    let mut batch = GraphqlBatch::new();
    for entry in entries {
        let name = required_string(entry, "name")?;
        println!("seed: adding {label} title '{name}'");
        batch.add(
            "AddTitleInput",
            "addTitle",
            "{ title { id name facet } }",
            build_title_input(entry, facet)?,
        );
    }

    let _ = batch.execute(ctx, graphql_url, &format!("{label} title add"))?;
    Ok(())
}

fn build_title_input(entry: &Value, facet: &str) -> Result<Value> {
    let mut external_ids = entry
        .get("externalIds")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(tvdb_id) = entry.get("tvdbId") {
        external_ids.push(json!({
            "source": "tvdb",
            "value": scalar_to_string(tvdb_id),
        }));
    }
    external_ids = unique_external_ids(external_ids);

    Ok(json!({
        "name": required_string(entry, "name")?,
        "facet": facet,
        "monitored": entry.get("monitored").cloned().unwrap_or(Value::Bool(false)),
        "tags": entry.get("tags").cloned().unwrap_or_else(|| json!([])),
        "options": present_or_null(entry, "options"),
        "externalIds": external_ids,
        "sourceHint": present_or_null(entry, "sourceHint"),
        "sourceKind": present_or_null(entry, "sourceKind"),
        "sourceTitle": present_or_null(entry, "sourceTitle"),
        "minAvailability": present_or_null(entry, "minAvailability"),
        "posterUrl": present_or_null(entry, "posterUrl"),
        "year": present_or_null(entry, "year"),
        "overview": present_or_null(entry, "overview"),
        "sortTitle": present_or_null(entry, "sortTitle"),
        "slug": present_or_null(entry, "slug"),
        "runtimeMinutes": present_or_null(entry, "runtimeMinutes"),
        "language": present_or_null(entry, "language"),
        "contentStatus": present_or_null(entry, "contentStatus"),
    }))
}

fn add_aliases(
    aliases: &mut HashMap<String, String>,
    canonical_id: &str,
    name: &str,
    seed_id: Option<&str>,
) {
    let slug = slugify(name);
    for alias in [canonical_id, name, slug.as_str()]
        .into_iter()
        .chain(seed_id.filter(|value| !value.is_empty()))
    {
        aliases.insert(alias.to_string(), canonical_id.to_string());
    }
}

fn setting_value_string(entry: &Value) -> String {
    entry
        .get("value")
        .or_else(|| entry.get("valueJson"))
        .map(scalar_to_string)
        .unwrap_or_default()
}

fn scalar_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        _ => value.to_string(),
    }
}

fn unique_external_ids(values: Vec<Value>) -> Vec<Value> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    for value in values {
        let source = value
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let key = format!(
            "{source}:{}",
            value.get("value").map(scalar_to_string).unwrap_or_default()
        );
        if seen.insert(key) {
            unique.push(value);
        }
    }
    unique
}

fn download_client_routing_entries(
    entry: &Value,
    aliases: &HashMap<String, String>,
) -> Result<Vec<Value>> {
    let routing = parse_setting_object(entry)?;
    let mut entries = Vec::new();
    for (client_id, value) in remap_routing_keys(routing, aliases) {
        entries.push(json!({
            "clientId": client_id,
            "enabled": lookup_bool(&value, &["enabled", "is_enabled", "isEnabled"], true),
            "category": lookup_optional_string(&value, &["category"]),
            "recentQueuePriority": lookup_optional_number(&value, &["recentQueuePriority", "recentPriority", "recent_priority"]),
            "olderQueuePriority": lookup_optional_number(&value, &["olderQueuePriority", "olderPriority", "older_priority"]),
            "removeCompleted": lookup_bool(&value, &["removeCompleted", "remove_completed", "removeComplete"], false),
            "removeFailed": lookup_bool(&value, &["removeFailed", "remove_failed", "removeFailure"], false),
        }));
    }
    Ok(entries)
}

fn indexer_routing_entries(entry: &Value, aliases: &HashMap<String, String>) -> Result<Vec<Value>> {
    let routing = parse_setting_object(entry)?;
    let mut entries = Vec::new();
    for (indexer_id, value) in remap_routing_keys(routing, aliases) {
        entries.push(json!({
            "indexerId": indexer_id,
            "enabled": lookup_bool(&value, &["enabled", "is_enabled", "isEnabled"], true),
            "categories": lookup_categories(&value),
            "priority": match lookup_optional_number(&value, &["priority", "order"]) {
                Value::Null => json!(1),
                other => other,
            },
        }));
    }
    Ok(entries)
}

fn parse_setting_object(entry: &Value) -> Result<Map<String, Value>> {
    let key = required_string(entry, "key")?;
    let value = if let Some(value_json) = entry.get("valueJson") {
        value_json.clone()
    } else if let Some(value) = entry.get("value") {
        match value {
            Value::String(text) => serde_json::from_str(text)
                .with_context(|| format!("failed to parse JSON setting for {key}"))?,
            other => other.clone(),
        }
    } else {
        Value::Object(Map::new())
    };

    value
        .as_object()
        .cloned()
        .context("expected routing setting to be a JSON object")
}

fn remap_routing_keys(
    routing: Map<String, Value>,
    aliases: &HashMap<String, String>,
) -> BTreeMap<String, Value> {
    let mut remapped = BTreeMap::new();
    for (key, value) in routing {
        let canonical = aliases.get(&key).cloned().unwrap_or(key);
        remapped.insert(canonical, value);
    }
    remapped
}

fn lookup_bool(value: &Value, keys: &[&str], default: bool) -> bool {
    for key in keys {
        if let Some(found) = value.get(*key).and_then(Value::as_bool) {
            return found;
        }
    }
    default
}

fn lookup_optional_string(value: &Value, keys: &[&str]) -> Value {
    for key in keys {
        if let Some(found) = value.get(*key) {
            return found.clone();
        }
    }
    Value::Null
}

fn lookup_optional_number(value: &Value, keys: &[&str]) -> Value {
    for key in keys {
        if let Some(found) = value.get(*key) {
            return found.clone();
        }
    }
    Value::Null
}

fn lookup_categories(value: &Value) -> Vec<Value> {
    if let Some(categories) = value.get("categories").and_then(Value::as_array) {
        return categories.clone();
    }
    if let Some(category) = value.get("category") {
        if let Some(categories) = category.as_array() {
            return categories.clone();
        }
        if let Some(category) = category.as_str() {
            return vec![Value::String(category.to_string())];
        }
    }
    Vec::new()
}

fn required_string(entry: &Value, key: &str) -> Result<String> {
    entry
        .get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .with_context(|| format!("missing required string field '{key}'"))
}

fn present_or_null(entry: &Value, key: &str) -> Value {
    entry.get(key).cloned().unwrap_or(Value::Null)
}

fn first_present(entry: &Value, keys: &[&str]) -> Value {
    for key in keys {
        if let Some(value) = entry.get(*key) {
            return value.clone();
        }
    }
    Value::Null
}

fn config_json_value(entry: &Value) -> Value {
    if let Some(config) = entry.get("config") {
        Value::String(config.to_string())
    } else if let Some(config_json) = entry.get("configJson") {
        config_json.clone()
    } else {
        Value::Null
    }
}

fn config_value_input(entry: &Value) -> Result<Value> {
    provider_config_value_input(config_json_value(entry))
}

fn indexer_config_value_input(entry: &Value) -> Result<Value> {
    if let Value::Null = config_json_value(entry) {
        let mut legacy_config = Map::new();

        if let Some(base_url) = entry.get("baseUrl") {
            legacy_config.insert("base_url".to_string(), base_url.clone());
        }
        if let Some(api_key) = entry.get("apiKey") {
            legacy_config.insert("api_key".to_string(), api_key.clone());
        }

        provider_config_value_input(Value::Object(legacy_config))
    } else {
        provider_config_value_input(config_json_value(entry))
    }
}

fn provider_config_value_input(config: Value) -> Result<Value> {
    let object = match config {
        Value::Null => Map::new(),
        Value::Object(object) => object,
        Value::String(raw) => serde_json::from_str::<Value>(&raw)
            .with_context(|| "failed to parse seed configJson as JSON")?
            .as_object()
            .cloned()
            .with_context(|| "seed configJson must be a JSON object")?,
        _ => bail!("seed config must be a JSON object"),
    };

    let mut values = Vec::with_capacity(object.len());
    for (key, value) in object {
        if key.trim().is_empty() {
            bail!("seed config value key is required");
        }
        values.push(provider_config_scalar_input(&key, value)?);
    }

    Ok(Value::Array(values))
}

fn provider_config_scalar_input(key: &str, value: Value) -> Result<Value> {
    let mut input = Map::new();
    input.insert("key".to_string(), Value::String(key.to_string()));

    match value {
        Value::Null => {
            input.insert("clearSecret".to_string(), Value::Bool(true));
        }
        Value::Bool(raw) => {
            input.insert("boolValue".to_string(), Value::Bool(raw));
        }
        Value::Number(raw) => {
            if let Some(raw) = raw.as_i64() {
                input.insert("intValue".to_string(), Value::Number(raw.into()));
            } else if let Some(raw) = raw.as_f64() {
                input.insert(
                    "floatValue".to_string(),
                    Value::Number(
                        serde_json::Number::from_f64(raw).with_context(|| {
                            format!("config value '{key}' has an invalid float")
                        })?,
                    ),
                );
            } else {
                bail!("config value '{key}' has an unsupported number");
            }
        }
        Value::String(raw) => {
            let field_name = if provider_config_key_is_secret(key) {
                "secretValue"
            } else {
                "stringValue"
            };
            input.insert(field_name.to_string(), Value::String(raw));
        }
        Value::Array(_) | Value::Object(_) => {
            bail!("config value '{key}' must be a scalar for ProviderConfigValueInput");
        }
    }

    Ok(Value::Object(input))
}

fn provider_config_key_is_secret(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    normalized.contains("apikey")
        || normalized.contains("password")
        || normalized.contains("token")
        || normalized.contains("secret")
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for character in value.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            last_dash = false;
        } else if !last_dash && !slug.is_empty() {
            slug.push('-');
            last_dash = true;
        }
    }
    slug.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::indexer_config_value_input;
    use serde_json::json;

    #[test]
    fn indexer_config_value_input_prefers_explicit_config() {
        let entry = json!({
            "config": {
                "base_url": "http://override:8080",
                "api_key": "override-key"
            },
            "baseUrl": "http://legacy:8080",
            "apiKey": "legacy-key"
        });

        let got = indexer_config_value_input(&entry).expect("config input");
        let values = got.as_array().expect("config input array");

        assert!(values.contains(&json!({
            "key": "base_url",
            "stringValue": "http://override:8080"
        })));
        assert!(values.contains(&json!({
            "key": "api_key",
            "secretValue": "override-key"
        })));
    }

    #[test]
    fn indexer_config_value_input_builds_legacy_newznab_shape() {
        let entry = json!({
            "baseUrl": "http://legacy:8080",
            "apiKey": "legacy-key"
        });

        let got = indexer_config_value_input(&entry).expect("config input");
        let values = got.as_array().expect("config input array");

        assert!(values.contains(&json!({
            "key": "base_url",
            "stringValue": "http://legacy:8080"
        })));
        assert!(values.contains(&json!({
            "key": "api_key",
            "secretValue": "legacy-key"
        })));
    }
}
