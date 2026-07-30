use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};

/// Modell, das fuer den Usage-Poll benutzt wird. Bewusst das billigste Modell
/// mit max_tokens=1, weil jeder Poll selbst ein kleines bisschen vom Kontingent
/// des Nutzers verbraucht (siehe Discovery-Spike: count_tokens liefert KEINE
/// Rate-Limit-Header, nur ein echter /v1/messages-Call tut das).
const POLL_MODEL: &str = "claude-haiku-4-5-20251001";
const API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const OAUTH_BETA: &str = "oauth-2025-04-20";

#[derive(Debug, thiserror::Error)]
pub enum UsageError {
    #[error("Netzwerk-/API-Fehler: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Antwort enthielt keine bekannten Rate-Limit-Header (HTTP {0}) - API-Vertrag hat sich evtl. geaendert")]
    NoRateLimitHeaders(u16),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RateWindow {
    pub status: Option<String>,
    pub reset_unix: Option<i64>,
    pub utilization: Option<f64>,
}

impl RateWindow {
    fn is_empty(&self) -> bool {
        self.status.is_none() && self.reset_unix.is_none() && self.utilization.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    pub five_hour: Option<RateWindow>,
    pub seven_day: Option<RateWindow>,
    pub overage: Option<RateWindow>,
    pub representative_claim: Option<String>,
    pub fetched_at_unix: i64,
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

fn window_from_headers(headers: &HeaderMap, prefix: &str) -> RateWindow {
    let status = header_str(headers, &format!("anthropic-ratelimit-unified-{prefix}-status"))
        .map(str::to_string);
    let reset_unix = header_str(headers, &format!("anthropic-ratelimit-unified-{prefix}-reset"))
        .and_then(|v| v.parse::<i64>().ok());
    let utilization =
        header_str(headers, &format!("anthropic-ratelimit-unified-{prefix}-utilization"))
            .and_then(|v| v.parse::<f64>().ok());
    RateWindow { status, reset_unix, utilization }
}

/// Extrahiert Nutzungsdaten aus den Response-Headern eines authentifizierten
/// /v1/messages-Requests. Alle Annahmen ueber die (inoffizielle) Header-Form
/// sind bewusst hier gebuendelt, damit ein Fix bei API-Aenderungen an einer
/// einzigen Stelle passiert.
pub fn parse_headers(headers: &HeaderMap, fetched_at_unix: i64) -> UsageSnapshot {
    let five_hour = window_from_headers(headers, "5h");
    let seven_day = window_from_headers(headers, "7d");
    let overage = window_from_headers(headers, "overage");
    let representative_claim =
        header_str(headers, "anthropic-ratelimit-unified-representative-claim")
            .map(str::to_string);

    UsageSnapshot {
        five_hour: (!five_hour.is_empty()).then_some(five_hour),
        seven_day: (!seven_day.is_empty()).then_some(seven_day),
        overage: (!overage.is_empty()).then_some(overage),
        representative_claim,
        fetched_at_unix,
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Fuehrt den minimalen authentifizierten Poll aus und liefert die geparsten
/// Nutzungsdaten. Verbraucht selbst ein winziges bisschen Kontingent - siehe
/// POLL_MODEL-Kommentar. Deshalb sollte der Aufrufer nicht haeufiger als
/// alle paar Minuten pollen.
pub async fn fetch_usage(client: &reqwest::Client, access_token: &str) -> Result<UsageSnapshot, UsageError> {
    let resp = client
        .post(API_URL)
        .bearer_auth(access_token)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("anthropic-beta", OAUTH_BETA)
        .json(&serde_json::json!({
            "model": POLL_MODEL,
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "."}]
        }))
        .send()
        .await?;

    let status = resp.status().as_u16();
    let headers = resp.headers().clone();
    let snapshot = parse_headers(&headers, now_unix());

    if snapshot.five_hour.is_none() && snapshot.seven_day.is_none() {
        return Err(UsageError::NoRateLimitHeaders(status));
    }
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
    use std::str::FromStr;

    fn headers_from_pairs(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (k, v) in pairs {
            map.insert(HeaderName::from_str(k).unwrap(), HeaderValue::from_str(v).unwrap());
        }
        map
    }

    /// Reale Header-Werte aus dem Discovery-Spike gegen eine echte Pro-Session.
    #[test]
    fn parses_real_world_header_sample() {
        let headers = headers_from_pairs(&[
            ("anthropic-ratelimit-unified-status", "allowed"),
            ("anthropic-ratelimit-unified-5h-status", "allowed"),
            ("anthropic-ratelimit-unified-5h-reset", "1785414600"),
            ("anthropic-ratelimit-unified-5h-utilization", "0.13"),
            ("anthropic-ratelimit-unified-7d-status", "allowed"),
            ("anthropic-ratelimit-unified-7d-reset", "1785884400"),
            ("anthropic-ratelimit-unified-7d-utilization", "0.12"),
            ("anthropic-ratelimit-unified-overage-status", "allowed"),
            ("anthropic-ratelimit-unified-overage-reset", "1785397800"),
            ("anthropic-ratelimit-unified-overage-utilization", "0.0"),
            ("anthropic-ratelimit-unified-representative-claim", "five_hour"),
        ]);

        let snapshot = parse_headers(&headers, 1785400000);

        let five_hour = snapshot.five_hour.expect("5h window present");
        assert_eq!(five_hour.status.as_deref(), Some("allowed"));
        assert_eq!(five_hour.reset_unix, Some(1785414600));
        assert_eq!(five_hour.utilization, Some(0.13));

        let seven_day = snapshot.seven_day.expect("7d window present");
        assert_eq!(seven_day.utilization, Some(0.12));
        assert_eq!(seven_day.reset_unix, Some(1785884400));

        assert_eq!(snapshot.representative_claim.as_deref(), Some("five_hour"));
        assert_eq!(snapshot.fetched_at_unix, 1785400000);
    }

    #[test]
    fn missing_headers_yield_none_windows() {
        let headers = HeaderMap::new();
        let snapshot = parse_headers(&headers, 42);
        assert!(snapshot.five_hour.is_none());
        assert!(snapshot.seven_day.is_none());
        assert!(snapshot.overage.is_none());
        assert!(snapshot.representative_claim.is_none());
    }

    #[test]
    fn partial_headers_still_parse_available_fields() {
        // Falls Anthropic mal nur einzelne Felder aendert/weglaesst, soll das
        // nicht die ganze Auswertung zum Absturz bringen.
        let headers = headers_from_pairs(&[
            ("anthropic-ratelimit-unified-5h-utilization", "0.5"),
        ]);
        let snapshot = parse_headers(&headers, 1);
        let five_hour = snapshot.five_hour.expect("5h window present");
        assert_eq!(five_hour.utilization, Some(0.5));
        assert_eq!(five_hour.reset_unix, None);
        assert!(snapshot.seven_day.is_none());
    }
}
