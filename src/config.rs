use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiTopN {
    Max,
    Int(u32),
}

impl ApiTopN {
    pub fn is_max(&self) -> bool {
        matches!(self, Self::Max)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollingConfig {
    pub api_top_n: ApiTopN,
    pub history_top_n: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub polling: PollingConfig,
}

pub fn from_toml(input: &str) -> Result<Config, String> {
    let raw: RawConfig = toml::from_str(input).map_err(|err| err.to_string())?;
    let api_top_n = match raw.polling.api_top_n {
        RawApiTopN::Int(value) if value >= 1 => ApiTopN::Int(value),
        RawApiTopN::Int(_) => return Err("polling.api_top_n must be >= 1".to_string()),
        RawApiTopN::String(value) => {
            if value.eq_ignore_ascii_case("max") {
                ApiTopN::Max
            } else {
                let parsed = value
                    .parse::<u32>()
                    .map_err(|_| "polling.api_top_n must be 'max' or integer >= 1".to_string())?;
                if parsed == 0 {
                    return Err("polling.api_top_n must be >= 1".to_string());
                }
                ApiTopN::Int(parsed)
            }
        }
    };

    if raw.polling.history_top_n == 0 {
        return Err("polling.history_top_n must be >= 1".to_string());
    }

    Ok(Config {
        polling: PollingConfig {
            api_top_n,
            history_top_n: raw.polling.history_top_n,
        },
    })
}

#[derive(Debug, Deserialize)]
struct RawConfig {
    polling: RawPollingConfig,
}

#[derive(Debug, Deserialize)]
struct RawPollingConfig {
    api_top_n: RawApiTopN,
    history_top_n: u32,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawApiTopN {
    String(String),
    Int(u32),
}
