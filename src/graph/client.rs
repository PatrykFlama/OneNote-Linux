use crate::project::GraphSyncPage;
use keyring::Entry;
use libonenote::GraphPageExport;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use ureq::tls::{RootCerts, TlsConfig};

const AUTHORITY: &str = "https://login.microsoftonline.com/common/oauth2/v2.0";
const GRAPH_ROOT: &str = "https://graph.microsoft.com/v1.0";
const SCOPES: &str = "offline_access Notes.ReadWrite";
const KEYRING_SERVICE: &str = "io.github.onenote-linux.Viewer";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct TokenSecret {
    access_token: String,
    refresh_token: String,
    expires_at: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GraphNotebook {
    pub id: String,
    pub display_name: String,
    pub is_default: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GraphSection {
    pub id: String,
    pub display_name: String,
    pub is_default: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreatedPage {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub last_modified_date_time: String,
}

pub(super) enum UpdatePageOutcome {
    Updated(TokenSecret, CreatedPage),
    Conflict {
        token: TokenSecret,
        remote_modified_at: String,
    },
}

#[derive(Deserialize)]
struct Collection<T> {
    value: Vec<T>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
}

#[derive(Deserialize)]
struct OAuthError {
    error: String,
    error_description: Option<String>,
}

pub(super) fn request_device_code(client_id: &str) -> Result<DeviceCode, String> {
    let response = agent()
        .post(&format!("{AUTHORITY}/devicecode"))
        .send_form([("client_id", client_id), ("scope", SCOPES)])
        .map_err(http_error)?;
    response_json(response)
}

pub(super) fn poll_device_code(client_id: &str, code: &DeviceCode) -> Result<TokenSecret, String> {
    let deadline = unix_time().saturating_add(code.expires_in);
    let mut interval = code.interval.max(1);
    while unix_time() < deadline {
        std::thread::sleep(Duration::from_secs(interval));
        let response = agent()
            .post(&format!("{AUTHORITY}/token"))
            .send_form([
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", client_id),
                ("device_code", code.device_code.as_str()),
            ])
            .map_err(http_error)?;
        let status = response.status();
        let body = response_body(response)?;
        if status.is_success() {
            let token: TokenResponse =
                serde_json::from_str(&body).map_err(|error| error.to_string())?;
            return Ok(token_secret(token, String::new()));
        }
        let error: OAuthError =
            serde_json::from_str(&body).map_err(|_| graph_error(status.as_u16(), &body))?;
        match error.error.as_str() {
            "authorization_pending" => {}
            "slow_down" => interval += 5,
            _ => {
                return Err(error
                    .error_description
                    .unwrap_or_else(|| error.error.replace('_', " ")));
            }
        }
    }
    Err("Microsoft sign-in code expired".to_owned())
}

pub(super) fn load_token(client_id: &str) -> Result<Option<TokenSecret>, String> {
    let entry = token_entry(client_id)?;
    match entry.get_password() {
        Ok(secret) => serde_json::from_str(&secret)
            .map(Some)
            .map_err(|error| format!("invalid stored Microsoft session: {error}")),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(format!("failed to read Microsoft session: {error}")),
    }
}

pub(super) fn save_token(client_id: &str, token: &TokenSecret) -> Result<(), String> {
    let secret = serde_json::to_string(token).map_err(|error| error.to_string())?;
    token_entry(client_id)?
        .set_password(&secret)
        .map_err(|error| format!("failed to save Microsoft session: {error}"))
}

pub(super) fn delete_token(client_id: &str) -> Result<(), String> {
    match token_entry(client_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(format!("failed to delete Microsoft session: {error}")),
    }
}

pub(super) fn list_notebooks(
    client_id: &str,
    token: TokenSecret,
) -> Result<(TokenSecret, Vec<GraphNotebook>), String> {
    let token = refresh_if_needed(client_id, token)?;
    let response = authorized_get(
        &token,
        &format!("{GRAPH_ROOT}/me/onenote/notebooks?$select=id,displayName,isDefault"),
    )?;
    let collection: Collection<GraphNotebook> = response_json(response)?;
    Ok((token, collection.value))
}

pub(super) fn list_sections(
    client_id: &str,
    token: TokenSecret,
    notebook_id: &str,
) -> Result<(TokenSecret, Vec<GraphSection>), String> {
    let token = refresh_if_needed(client_id, token)?;
    let notebook_id = encode(notebook_id);
    let response = authorized_get(
        &token,
        &format!(
            "{GRAPH_ROOT}/me/onenote/notebooks/{notebook_id}/sections?$select=id,displayName,isDefault"
        ),
    )?;
    let collection: Collection<GraphSection> = response_json(response)?;
    Ok((token, collection.value))
}

pub(super) fn create_page(
    client_id: &str,
    token: TokenSecret,
    section_id: &str,
    page: &GraphPageExport,
) -> Result<(TokenSecret, CreatedPage), String> {
    let token = refresh_if_needed(client_id, token)?;
    let section_id = encode(section_id);
    let url = format!("{GRAPH_ROOT}/me/onenote/sections/{section_id}/pages");
    let authorization = format!("Bearer {}", token.access_token);
    let request = agent()
        .post(&url)
        .header("Authorization", &authorization)
        .header("Accept", "application/json");
    let response = if page.resources.is_empty() {
        request
            .header("Content-Type", "application/xhtml+xml; charset=utf-8")
            .send(&page.html)
            .map_err(http_error)?
    } else {
        let boundary = format!("OneNoteLinux{}", unix_time());
        let body = multipart_page(page, &boundary);
        request
            .header(
                "Content-Type",
                &format!("multipart/form-data; boundary={boundary}"),
            )
            .send(body)
            .map_err(http_error)?
    };
    let created = response_json(response)?;
    Ok((token, created))
}

pub(super) fn update_page(
    client_id: &str,
    token: TokenSecret,
    page_id: &str,
    expected_modified_at: &str,
    page: &GraphSyncPage,
) -> Result<UpdatePageOutcome, String> {
    let token = refresh_if_needed(client_id, token)?;
    let page_id = encode(page_id);
    let page_url =
        format!("{GRAPH_ROOT}/me/onenote/pages/{page_id}?$select=id,title,lastModifiedDateTime");
    let remote: CreatedPage = response_json(authorized_get(&token, &page_url)?)?;
    if remote.last_modified_date_time != expected_modified_at {
        return Ok(UpdatePageOutcome::Conflict {
            token,
            remote_modified_at: remote.last_modified_date_time,
        });
    }

    let content_url = format!("{GRAPH_ROOT}/me/onenote/pages/{page_id}/content?includeIDs=true");
    let content = response_text(authorized_get_accept(&token, &content_url, "text/html")?)?;
    let target = find_generated_content_id(&content).ok_or_else(|| {
        "The linked OneNote page does not contain an updateable OneNote Linux content block"
            .to_owned()
    })?;
    let commands = serde_json::to_string(&serde_json::json!([
        {
            "target": "title",
            "action": "replace",
            "content": &page.export.title,
        },
        {
            "target": target,
            "action": "replace",
            "content": &page.replacement_html,
        }
    ]))
    .map_err(|error| error.to_string())?;
    let update_url = format!("{GRAPH_ROOT}/me/onenote/pages/{page_id}/content");
    let authorization = format!("Bearer {}", token.access_token);
    let request = agent()
        .patch(&update_url)
        .header("Authorization", &authorization)
        .header("Accept", "application/json");
    let response = if page.export.resources.is_empty() {
        request
            .header("Content-Type", "application/json")
            .send(&commands)
            .map_err(http_error)?
    } else {
        let boundary = format!("OneNoteLinux{}", unix_time());
        let body = multipart_commands(&commands, &page.export, &boundary);
        request
            .header(
                "Content-Type",
                &format!("multipart/form-data; boundary={boundary}"),
            )
            .send(body)
            .map_err(http_error)?
    };
    ensure_success(response)?;

    let updated: CreatedPage = response_json(authorized_get(&token, &page_url)?)?;
    Ok(UpdatePageOutcome::Updated(token, updated))
}

fn refresh_if_needed(client_id: &str, token: TokenSecret) -> Result<TokenSecret, String> {
    if token.expires_at > unix_time().saturating_add(120) {
        return Ok(token);
    }
    if token.refresh_token.is_empty() {
        return Err("Microsoft session expired; sign in again".to_owned());
    }
    let response = agent()
        .post(&format!("{AUTHORITY}/token"))
        .send_form([
            ("client_id", client_id),
            ("grant_type", "refresh_token"),
            ("refresh_token", token.refresh_token.as_str()),
            ("scope", SCOPES),
        ])
        .map_err(http_error)?;
    let status = response.status();
    let body = response_body(response)?;
    if !status.is_success() {
        return Err(graph_error(status.as_u16(), &body));
    }
    let refreshed: TokenResponse =
        serde_json::from_str(&body).map_err(|error| error.to_string())?;
    Ok(token_secret(refreshed, token.refresh_token))
}

fn authorized_get(
    token: &TokenSecret,
    url: &str,
) -> Result<ureq::http::Response<ureq::Body>, String> {
    authorized_get_accept(token, url, "application/json")
}

fn authorized_get_accept(
    token: &TokenSecret,
    url: &str,
    accept: &str,
) -> Result<ureq::http::Response<ureq::Body>, String> {
    agent()
        .get(url)
        .header("Authorization", &format!("Bearer {}", token.access_token))
        .header("Accept", accept)
        .call()
        .map_err(http_error)
}

fn token_secret(response: TokenResponse, previous_refresh_token: String) -> TokenSecret {
    TokenSecret {
        access_token: response.access_token,
        refresh_token: response.refresh_token.unwrap_or(previous_refresh_token),
        expires_at: unix_time().saturating_add(response.expires_in.saturating_sub(60)),
    }
}

fn token_entry(client_id: &str) -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, &format!("microsoft-graph:{client_id}"))
        .map_err(|error| format!("credential store unavailable: {error}"))
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(60)))
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .new_agent()
}

fn response_json<T: DeserializeOwned>(
    response: ureq::http::Response<ureq::Body>,
) -> Result<T, String> {
    let status = response.status();
    let body = response_body(response)?;
    if !status.is_success() {
        return Err(graph_error(status.as_u16(), &body));
    }
    serde_json::from_str(&body).map_err(|error| format!("invalid Microsoft response: {error}"))
}

fn response_text(response: ureq::http::Response<ureq::Body>) -> Result<String, String> {
    let status = response.status();
    let body = response_body(response)?;
    if !status.is_success() {
        return Err(graph_error(status.as_u16(), &body));
    }
    Ok(body)
}

fn ensure_success(response: ureq::http::Response<ureq::Body>) -> Result<(), String> {
    let status = response.status();
    let body = response_body(response)?;
    if status.is_success() {
        Ok(())
    } else {
        Err(graph_error(status.as_u16(), &body))
    }
}

fn response_body(mut response: ureq::http::Response<ureq::Body>) -> Result<String, String> {
    response
        .body_mut()
        .read_to_string()
        .map_err(|error| error.to_string())
}

fn graph_error(status: u16, body: &str) -> String {
    let message = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .or_else(|| value.pointer("/error_description"))
                .or_else(|| value.pointer("/error"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| body.trim().to_owned());
    if message.is_empty() {
        format!("Microsoft Graph request failed with HTTP {status}")
    } else {
        format!("Microsoft Graph HTTP {status}: {message}")
    }
}

fn http_error(error: ureq::Error) -> String {
    format!("Microsoft network request failed: {error}")
}

fn multipart_page(page: &GraphPageExport, boundary: &str) -> Vec<u8> {
    let mut body = Vec::new();
    push_part(
        &mut body,
        boundary,
        "Presentation",
        "text/html; charset=utf-8",
        page.html.as_bytes(),
    );
    for resource in &page.resources {
        push_part(
            &mut body,
            boundary,
            &resource.part_name,
            &resource.content_type,
            resource.bytes(),
        );
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

fn multipart_commands(commands: &str, page: &GraphPageExport, boundary: &str) -> Vec<u8> {
    let mut body = Vec::new();
    push_part(
        &mut body,
        boundary,
        "Commands",
        "application/json",
        commands.as_bytes(),
    );
    for resource in &page.resources {
        push_part(
            &mut body,
            boundary,
            &resource.part_name,
            &resource.content_type,
            resource.bytes(),
        );
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

fn push_part(body: &mut Vec<u8>, boundary: &str, name: &str, content_type: &str, data: &[u8]) {
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"{name}\"\r\n").as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {content_type}\r\n\r\n").as_bytes());
    body.extend_from_slice(data);
    body.extend_from_slice(b"\r\n");
}

fn encode(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

fn find_generated_content_id(html: &str) -> Option<String> {
    const DATA_ID: &str = "onenote-linux-content";
    let marker = [
        format!("data-id=\"{DATA_ID}\""),
        format!("data-id='{DATA_ID}'"),
    ]
    .into_iter()
    .find_map(|marker| html.find(&marker))?;
    let tag_start = html[..marker].rfind('<')?;
    let tag_end = marker + html[marker..].find('>')?;
    let tag = &html[tag_start..=tag_end];
    attribute_value(tag, "id").map(str::to_owned)
}

fn attribute_value<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    for quote in ['"', '\''] {
        let prefix = format!(" {name}={quote}");
        if let Some(start) = tag.find(&prefix).map(|start| start + prefix.len())
            && let Some(end) = tag[start..].find(quote)
        {
            return Some(&tag[start..start + end]);
        }
    }
    None
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use libonenote::GraphPageExport;

    #[test]
    fn percent_encodes_graph_ids() {
        assert_eq!(encode("a/b c"), "a%2Fb%20c");
    }

    #[test]
    fn multipart_uses_crlf_and_presentation_part() {
        let page = GraphPageExport {
            section_path: Vec::new(),
            source_page_id: String::new(),
            title: "Page".to_owned(),
            html: "<html></html>".to_owned(),
            resources: Vec::new(),
            warnings: Vec::new(),
        };
        let body = String::from_utf8(multipart_page(&page, "Boundary")).unwrap();
        assert!(body.contains("name=\"Presentation\"\r\nContent-Type: text/html"));
        assert!(body.ends_with("--Boundary--\r\n"));
    }

    #[test]
    fn finds_generated_id_for_updateable_content() {
        let html = r#"<body><div id="div:{123}" data-id="onenote-linux-content">x</div></body>"#;
        assert_eq!(
            find_generated_content_id(html).as_deref(),
            Some("div:{123}")
        );
    }

    #[test]
    fn finds_generated_id_when_attributes_are_reordered() {
        let html = "<div data-id='onenote-linux-content' class='x' id='content-id'></div>";
        assert_eq!(
            find_generated_content_id(html).as_deref(),
            Some("content-id")
        );
    }
}
