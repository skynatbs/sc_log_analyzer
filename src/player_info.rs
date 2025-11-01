use once_cell::sync::Lazy;
use reqwest::{StatusCode, blocking::Client};
use scraper::{Html, Selector};
use std::{fmt, time::Duration};

#[derive(Debug, Clone, Default)]
pub struct PlayerInfo {
    pub enlisted: Option<String>,
    pub location: Option<String>,
    pub fluency: Option<String>,
    pub main_organization: Option<String>,
}

impl PlayerInfo {
    pub fn is_empty(&self) -> bool {
        self.enlisted.is_none()
            && self.location.is_none()
            && self.fluency.is_none()
            && self.main_organization.is_none()
    }
}

#[derive(Debug, Clone)]
pub enum PlayerInfoError {
    Network(String),
    Http(u16),
    NotFound,
    Parse(String),
}

impl fmt::Display for PlayerInfoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlayerInfoError::Network(err) => write!(f, "Network error: {}", err),
            PlayerInfoError::Http(code) => write!(f, "HTTP error: status {}", code),
            PlayerInfoError::NotFound => write!(f, "Citizen profile not found"),
            PlayerInfoError::Parse(err) => write!(f, "Failed to parse profile: {}", err),
        }
    }
}

impl std::error::Error for PlayerInfoError {}

static CLIENT: Lazy<Client> = Lazy::new(|| {
    Client::builder()
        .user_agent("SC Log Analyzer/0.1")
        .timeout(Duration::from_secs(10))
        .build()
        .expect("failed to build HTTP client")
});

pub fn fetch_player_info(handle: &str) -> Result<PlayerInfo, PlayerInfoError> {
    let trimmed = handle.trim();
    if trimmed.is_empty() {
        return Err(PlayerInfoError::Parse("Empty handle".to_string()));
    }

    let url = format!("https://robertsspaceindustries.com/en/citizens/{}", trimmed);
    let response = CLIENT
        .get(url)
        .send()
        .map_err(|err| PlayerInfoError::Network(err.to_string()))?;

    let status = response.status();
    if status == StatusCode::NOT_FOUND {
        return Err(PlayerInfoError::NotFound);
    }
    if !status.is_success() {
        return Err(PlayerInfoError::Http(status.as_u16()));
    }

    let body = response
        .text()
        .map_err(|err| PlayerInfoError::Network(err.to_string()))?;
    let info = parse_player_info(&body);
    if info.is_empty() {
        Err(PlayerInfoError::Parse(
            "Profile page did not include expected fields".to_string(),
        ))
    } else {
        Ok(info)
    }
}

fn parse_player_info(html: &str) -> PlayerInfo {
    let document = Html::parse_document(html);
    let entry_selector = Selector::parse("div.profile-content .left-col .inner p.entry").unwrap();
    let label_selector = Selector::parse(".label").unwrap();
    let value_selector = Selector::parse(".value").unwrap();
    let main_org_link_selector = Selector::parse("div.main-org .info p.entry a.value").unwrap();
    let main_org_value_selector = Selector::parse("div.main-org .info p.entry .value").unwrap();
    let mut info = PlayerInfo::default();

    for entry in document.select(&entry_selector) {
        if let Some(label_elem) = entry.select(&label_selector).next() {
            let label_text = normalize_label(&label_elem.text().collect::<String>());
            let value_text = extract_value_text(&entry, &value_selector);
            if value_text.is_empty() {
                continue;
            }
            if label_text.eq_ignore_ascii_case("Enlisted") {
                info.enlisted = Some(value_text);
            } else if label_text.eq_ignore_ascii_case("Location") {
                info.location = Some(value_text);
            } else if label_text.eq_ignore_ascii_case("Fluency") {
                info.fluency = Some(value_text);
            } else if label_text.eq_ignore_ascii_case("Main Organization")
                && info.main_organization.is_none()
            {
                info.main_organization = Some(value_text);
            }
        }
    }

    if info.main_organization.is_none() {
        if let Some(org_value) = document.select(&main_org_link_selector).next() {
            let org_name = normalize_text(&org_value.text().collect::<String>());
            if !org_name.is_empty() {
                info.main_organization = Some(org_name);
            }
        } else if let Some(org_value) = document.select(&main_org_value_selector).next() {
            let org_name = normalize_text(&org_value.text().collect::<String>());
            if !org_name.is_empty() && !org_name.eq_ignore_ascii_case("Main organization") {
                info.main_organization = Some(org_name);
            }
        }
    }

    info
}

fn extract_value_text(entry: &scraper::ElementRef<'_>, value_selector: &Selector) -> String {
    if let Some(value_elem) = entry.select(value_selector).next() {
        let text = normalize_text(&value_elem.text().collect::<String>());
        if !text.is_empty() {
            return text;
        }
    }

    let text = entry
        .text()
        .map(|piece| piece.trim())
        .filter(|piece| !piece.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    normalize_text(&text)
}

fn normalize_text(input: &str) -> String {
    input
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn normalize_label(input: &str) -> String {
    input.trim().trim_end_matches(':').trim().to_string()
}
