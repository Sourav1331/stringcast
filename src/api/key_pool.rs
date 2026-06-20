use super::providers::ProviderKind;
use crate::storage::ApiKeyConfig;
use std::str::FromStr;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStatus {
    Active,
    RateLimited,
    Invalid,
    Disabled,
}

#[derive(Debug, Clone)]
pub struct ApiKey {
    pub id: String,
    pub provider: ProviderKind,
    pub alias: Option<String>,
    pub status: KeyStatus,
    pub rate_limit_until: Option<Instant>,
    pub consecutive_errors: u8,
    pub last_used: Option<Instant>,
    pub requests_total: u64,
    pub requests_success: u64,
}

#[derive(Debug, Clone, Default)]
pub struct KeyPool {
    keys: Vec<ApiKey>,
}

impl KeyPool {
    pub fn new(keys: Vec<ApiKey>) -> Self {
        Self { keys }
    }

    pub fn keys(&self) -> &[ApiKey] {
        &self.keys
    }

    pub fn next_available_key(&mut self, provider: ProviderKind, now: Instant) -> Option<&ApiKey> {
        self.expire_rate_limits(now);

        let selected_index = self
            .keys
            .iter()
            .enumerate()
            .filter(|(_, key)| key.provider == provider && key.status == KeyStatus::Active)
            .min_by_key(|(_, key)| {
                key.last_used
                    .unwrap_or(now.checked_sub(Duration::from_secs(86_400)).unwrap_or(now))
            })
            .map(|(index, _)| index)?;

        let key = &mut self.keys[selected_index];
        key.last_used = Some(now);
        key.requests_total += 1;
        Some(key)
    }

    pub fn mark_success(&mut self, key_id: &str) {
        if let Some(key) = self.find_mut(key_id) {
            key.consecutive_errors = 0;
            key.requests_success += 1;
            if key.status == KeyStatus::RateLimited {
                key.status = KeyStatus::Active;
                key.rate_limit_until = None;
            }
        }
    }

    pub fn mark_rate_limited(&mut self, key_id: &str, until: Instant) {
        if let Some(key) = self.find_mut(key_id) {
            key.status = KeyStatus::RateLimited;
            key.rate_limit_until = Some(until);
            key.consecutive_errors = key.consecutive_errors.saturating_add(1);
        }
    }

    pub fn mark_invalid(&mut self, key_id: &str) {
        if let Some(key) = self.find_mut(key_id) {
            key.status = KeyStatus::Invalid;
            key.consecutive_errors = key.consecutive_errors.saturating_add(1);
        }
    }

    pub fn disable(&mut self, key_id: &str) {
        if let Some(key) = self.find_mut(key_id) {
            key.status = KeyStatus::Disabled;
        }
    }

    fn expire_rate_limits(&mut self, now: Instant) {
        for key in &mut self.keys {
            if key.status == KeyStatus::RateLimited && key.rate_limit_until <= Some(now) {
                key.status = KeyStatus::Active;
                key.rate_limit_until = None;
            }
        }
    }

    fn find_mut(&mut self, key_id: &str) -> Option<&mut ApiKey> {
        self.keys.iter_mut().find(|key| key.id == key_id)
    }
}

impl TryFrom<&ApiKeyConfig> for ApiKey {
    type Error = String;

    fn try_from(config: &ApiKeyConfig) -> Result<Self, Self::Error> {
        Ok(Self {
            id: config.id.clone(),
            provider: ProviderKind::from_str(&config.provider)
                .map_err(|error| format!("{error:?}"))?,
            alias: config.alias.clone(),
            status: KeyStatus::from_config_str(&config.status)
                .ok_or_else(|| format!("unknown key status: {}", config.status))?,
            rate_limit_until: None,
            consecutive_errors: 0,
            last_used: None,
            requests_total: 0,
            requests_success: 0,
        })
    }
}

impl KeyPool {
    pub fn from_config(configs: &[ApiKeyConfig]) -> Result<Self, String> {
        let keys = configs
            .iter()
            .map(ApiKey::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::new(keys))
    }
}

impl KeyStatus {
    pub fn from_config_str(value: &str) -> Option<Self> {
        match value {
            "Active" | "active" => Some(Self::Active),
            "RateLimited" | "rate_limited" => Some(Self::RateLimited),
            "Invalid" | "invalid" => Some(Self::Invalid),
            "Disabled" | "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(id: &str, provider: ProviderKind, last_used: Option<Instant>) -> ApiKey {
        ApiKey {
            id: id.to_string(),
            provider,
            alias: None,
            status: KeyStatus::Active,
            rate_limit_until: None,
            consecutive_errors: 0,
            last_used,
            requests_total: 0,
            requests_success: 0,
        }
    }

    #[test]
    fn picks_least_recently_used_active_key() {
        let now = Instant::now();
        let mut pool = KeyPool::new(vec![
            key(
                "old",
                ProviderKind::Gemini,
                Some(now - Duration::from_secs(60)),
            ),
            key(
                "new",
                ProviderKind::Gemini,
                Some(now - Duration::from_secs(5)),
            ),
        ]);

        let selected = pool.next_available_key(ProviderKind::Gemini, now).unwrap();

        assert_eq!(selected.id, "old");
    }

    #[test]
    fn picks_never_used_key_without_underflowing_early_instant() {
        let now = Instant::now();
        let mut pool = KeyPool::new(vec![key("new", ProviderKind::Gemini, None)]);

        let selected = pool.next_available_key(ProviderKind::Gemini, now).unwrap();

        assert_eq!(selected.id, "new");
    }

    #[test]
    fn skips_rate_limited_key_until_cooldown_expires() {
        let now = Instant::now();
        let mut pool = KeyPool::new(vec![
            key("limited", ProviderKind::Gemini, None),
            key("active", ProviderKind::Gemini, None),
        ]);
        pool.mark_rate_limited("limited", now + Duration::from_secs(60));

        let selected = pool.next_available_key(ProviderKind::Gemini, now).unwrap();

        assert_eq!(selected.id, "active");
    }

    #[test]
    fn expired_rate_limit_becomes_available() {
        let now = Instant::now();
        let mut pool = KeyPool::new(vec![key("limited", ProviderKind::Gemini, None)]);
        pool.mark_rate_limited("limited", now - Duration::from_secs(1));

        let selected = pool.next_available_key(ProviderKind::Gemini, now).unwrap();

        assert_eq!(selected.id, "limited");
        assert_eq!(selected.status, KeyStatus::Active);
    }

    #[test]
    fn builds_from_key_config_metadata() {
        let pool = KeyPool::from_config(&[ApiKeyConfig {
            id: "key-1".to_string(),
            provider: "openai".to_string(),
            alias: Some("Main".to_string()),
            status: "Active".to_string(),
            created_at: "2026-05-31T00:00:00Z".to_string(),
        }])
        .unwrap();

        assert_eq!(pool.keys()[0].provider, ProviderKind::OpenAi);
        assert_eq!(pool.keys()[0].status, KeyStatus::Active);
        assert_eq!(pool.keys()[0].alias.as_deref(), Some("Main"));
    }
}
