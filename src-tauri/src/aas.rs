use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};
use uuid::Uuid;

const XMLNS_SOAP: &str = "http://schemas.xmlsoap.org/soap/envelope/";
const XMLNS_XMLA: &str = "urn:schemas-microsoft-com:xml-analysis";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaxRunRequest {
    pub server_url: String,
    pub catalog: String,
    pub tenant_id: String,
    pub client_id: String,
    pub client_secret: String,
    pub role: String,
    pub custom_data: String,
    pub query: String,
}

#[derive(Serialize)]
pub struct DaxRunResult {
    pub http_status: u16,
    pub elapsed_ms: u128,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

pub fn execute_dax(request: DaxRunRequest) -> Result<DaxRunResult, String> {
    validate_request(&request)?;
    let (region_host, server_name) = parse_server_url(&request.server_url)?;
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(300))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|error| format!("Could not initialize the AAS client: {error}"))?;
    let token = fetch_token(&client, &request, &region_host)?;
    let endpoint = resolve_cluster(&client, &token, &region_host, &server_name)?;
    let body = execute_envelope(
        &request.query,
        &request.catalog,
        &request.role,
        &request.custom_data,
    );
    let started = Instant::now();
    let response = client
        .post(&endpoint)
        .header("Content-Type", "text/xml; charset=utf-8")
        .header("SOAPAction", format!("\"{XMLNS_XMLA}:Execute\""))
        .bearer_auth(&token)
        .header("x-ms-xmlaserver", &server_name)
        .header("x-ms-request-registration-id", Uuid::new_v4().to_string())
        .header("x-ms-round-trip-id", "0")
        .header("x-ms-xmlacaps-negotiation-flags", "0,0,0,0,1")
        .header("x-ms-xmladedicatedconnection", "0")
        .body(body)
        .send()
        .map_err(|error| format!("AAS XMLA request failed: {error}"))?;
    let status = response.status();
    let raw_xml = response
        .text()
        .map_err(|error| format!("Could not read the AAS response: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "AAS returned HTTP {}: {}",
            status.as_u16(),
            response_excerpt(&raw_xml)
        ));
    }
    if let Some(fault) = xmla_fault(&raw_xml) {
        return Err(format!("AAS rejected the DAX query: {fault}"));
    }
    let (columns, rows) = parse_tabular_rows(&raw_xml)?;
    Ok(DaxRunResult {
        http_status: status.as_u16(),
        elapsed_ms: started.elapsed().as_millis(),
        columns,
        rows,
    })
}

fn validate_request(request: &DaxRunRequest) -> Result<(), String> {
    for (value, label) in [
        (&request.server_url, "AAS server URL"),
        (&request.catalog, "catalog"),
        (&request.tenant_id, "tenant ID"),
        (&request.client_id, "client ID"),
        (&request.client_secret, "client secret"),
        (&request.query, "DAX query"),
    ] {
        if value.trim().is_empty() {
            return Err(format!("Missing required {label}."));
        }
    }
    let query = request.query.trim_start_matches('\u{feff}').trim_start();
    let query_without_comments = query
        .lines()
        .skip_while(|line| line.trim().is_empty() || line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let upper = query_without_comments.trim_start().to_ascii_uppercase();
    if !upper.starts_with("EVALUATE") && !upper.starts_with("DEFINE") {
        return Err("Only DAX queries beginning with EVALUATE or DEFINE can be run.".into());
    }
    if request.role.trim().is_empty() != request.custom_data.trim().is_empty() {
        return Err("To apply row-level security, provide both the AAS role and CustomData. Leave both blank to run as the service principal without RLS context.".into());
    }
    Ok(())
}

fn parse_server_url(server_url: &str) -> Result<(String, String), String> {
    let rest = server_url
        .trim()
        .strip_prefix("asazure://")
        .ok_or_else(|| "AAS server URL must start with asazure://".to_string())?;
    let (region_host, server_name) = rest
        .split_once('/')
        .ok_or_else(|| "AAS server URL must include a region host and server name.".to_string())?;
    if region_host.is_empty()
        || server_name.is_empty()
        || !region_host.ends_with(".asazure.windows.net")
        || !region_host
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-'))
        || !server_name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return Err("AAS server URL is not a valid Azure Analysis Services endpoint.".into());
    }
    Ok((region_host.into(), server_name.into()))
}

fn fetch_token(
    client: &Client,
    request: &DaxRunRequest,
    region_host: &str,
) -> Result<String, String> {
    let endpoint = format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
        request.tenant_id.trim()
    );
    let scope = format!("https://{region_host}/.default");
    let response = client
        .post(endpoint)
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", request.client_id.trim()),
            ("client_secret", request.client_secret.as_str()),
            ("scope", scope.as_str()),
        ])
        .send()
        .map_err(|error| format!("Azure token request failed: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .map_err(|error| format!("Could not read the Azure token response: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "Azure token request returned HTTP {}: {}",
            status.as_u16(),
            oauth_error(&body)
        ));
    }
    let value: Value = serde_json::from_str(&body)
        .map_err(|error| format!("Azure token response was not valid JSON: {error}"))?;
    value
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Azure token response did not contain an access token.".into())
}

fn resolve_cluster(
    client: &Client,
    token: &str,
    region_host: &str,
    server_name: &str,
) -> Result<String, String> {
    let endpoint = format!("https://{region_host}/webapi/clusterResolve");
    let response = client
        .post(endpoint)
        .bearer_auth(token)
        .json(&serde_json::json!({ "serverName": server_name }))
        .send()
        .map_err(|error| format!("AAS cluster resolution failed: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .map_err(|error| format!("Could not read the cluster resolution response: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "AAS cluster resolution returned HTTP {}: {}",
            status.as_u16(),
            response_excerpt(&body)
        ));
    }
    let value: Value = serde_json::from_str(&body)
        .map_err(|error| format!("AAS cluster response was not valid JSON: {error}"))?;
    let cluster = value
        .get("clusterFQDN")
        .and_then(Value::as_str)
        .filter(|value| value.ends_with(".asazure.windows.net"))
        .ok_or_else(|| "AAS cluster response did not contain a valid clusterFQDN.".to_string())?;
    Ok(format!("https://{cluster}/webapi/xmla"))
}

fn execute_envelope(query: &str, catalog: &str, role: &str, custom_data: &str) -> String {
    let rls = if role.trim().is_empty() {
        String::new()
    } else {
        format!(
            "<CUSTOMDATA>{}</CUSTOMDATA><Roles>{}</Roles>",
            xml_escape(custom_data.trim()),
            xml_escape(role.trim())
        )
    };
    format!(
        "<soap:Envelope xmlns:soap=\"{XMLNS_SOAP}\"><soap:Header><BeginSession xmlns=\"{XMLNS_XMLA}\"/></soap:Header><soap:Body><Execute xmlns=\"{XMLNS_XMLA}\"><Command><Statement>{}</Statement></Command><Properties><PropertyList><Catalog>{}</Catalog><Format>TABULAR</Format><SafetyOptions>2</SafetyOptions>{rls}<Timeout>300</Timeout></PropertyList></Properties></Execute></soap:Body></soap:Envelope>",
        xml_escape(query),
        xml_escape(catalog)
    )
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn oauth_error(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error_description")
                .or_else(|| value.get("error"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| response_excerpt(body))
}

fn xmla_fault(xml: &str) -> Option<String> {
    let document = roxmltree::Document::parse(xml).ok()?;
    for name in ["faultstring", "Error", "Messages"] {
        if let Some(node) = document
            .descendants()
            .find(|node| node.is_element() && node.tag_name().name().eq_ignore_ascii_case(name))
        {
            let description = node
                .attribute("Description")
                .or_else(|| node.text())
                .unwrap_or("")
                .trim();
            if !description.is_empty() {
                return Some(description.to_string());
            }
        }
    }
    None
}

fn parse_tabular_rows(xml: &str) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
    let document = roxmltree::Document::parse(xml)
        .map_err(|error| format!("AAS returned malformed XML: {error}"))?;
    let row_nodes = document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "row")
        .collect::<Vec<_>>();
    let mut columns = Vec::new();
    let mut seen_columns = HashSet::new();
    for row in &row_nodes {
        for cell in row.children().filter(|node| node.is_element()) {
            let name = cell.tag_name().name().to_string();
            if seen_columns.insert(name.clone()) {
                columns.push(name);
            }
        }
    }
    let rows = row_nodes
        .into_iter()
        .take(10_000)
        .map(|row| {
            columns
                .iter()
                .map(|column| {
                    row.children()
                        .find(|cell| cell.is_element() && cell.tag_name().name() == column)
                        .and_then(|cell| cell.text())
                        .unwrap_or("")
                        .to_string()
                })
                .collect()
        })
        .collect();
    Ok((columns, rows))
}

fn response_excerpt(body: &str) -> String {
    let compact = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() > 600 {
        format!("{}…", compact.chars().take(600).collect::<String>())
    } else {
        compact
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_and_parses_aas_server_urls() {
        assert_eq!(
            parse_server_url("asazure://region.asazure.windows.net/server").unwrap(),
            ("region.asazure.windows.net".into(), "server".into())
        );
        assert!(parse_server_url("https://example.com/server").is_err());
        assert!(parse_server_url("asazure://example.com/server").is_err());
    }

    #[test]
    fn escapes_dax_inside_the_xmla_envelope() {
        let body = execute_envelope(
            "EVALUATE FILTER('T', [A] < 2 && [B] > 0)",
            "A&B",
            "configured-role",
            "configured-custom-data",
        );
        assert!(body.contains("[A] &lt; 2 &amp;&amp; [B] &gt; 0"));
        assert!(body.contains("<Catalog>A&amp;B</Catalog>"));
        assert!(body.contains("<Roles>configured-role</Roles>"));
    }

    #[test]
    fn extracts_generic_tabular_rows_and_faults() {
        let xml = r#"<Envelope><Body><root><row><Name>A</Name><Count>2</Count></row><row><Name>B</Name><Count>3</Count></row></root></Body></Envelope>"#;
        let (columns, rows) = parse_tabular_rows(xml).unwrap();
        assert_eq!(columns, vec!["Name", "Count"]);
        assert_eq!(rows[0], vec!["A", "2"]);
        assert_eq!(
            xmla_fault("<Fault><faultstring>Bad DAX</faultstring></Fault>"),
            Some("Bad DAX".into())
        );
    }
}
