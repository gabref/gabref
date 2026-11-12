use anyhow::{Context, Result, anyhow, bail};
use reqwest::{StatusCode, blocking::Client, redirect::Policy};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{env, process::Command, thread, time::Duration};

pub trait Api {
    fn query(&mut self, query: &str, variables: Value) -> Result<Value>;
}

pub struct GitHub {
    client: Client,
    token: String,
}

enum Failure {
    Retryable(anyhow::Error),
    Fatal(anyhow::Error),
}

#[derive(Deserialize)]
struct Envelope {
    data: Option<Value>,
    errors: Option<Vec<Value>>,
}

fn token() -> Result<String> {
    for name in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(value) = env::var(name)
            && !value.trim().is_empty()
        {
            return Ok(value);
        }
    }
    // Local convenience only: Actions supplies GH_TOKEN, with no gh dependency.
    if let Ok(output) = Command::new("gh").args(["auth", "token"]).output()
        && output.status.success()
    {
        let value =
            String::from_utf8(output.stdout).context("Invalid GitHub CLI token encoding")?;
        if !value.trim().is_empty() {
            return Ok(value.trim().to_owned());
        }
    }
    bail!("Set GH_TOKEN or authenticate locally with `gh auth login`.")
}

fn decode(envelope: Envelope) -> std::result::Result<Value, Failure> {
    if let Some(errors) = envelope.errors
        && !errors.is_empty()
    {
        let transient = errors.iter().any(|error| {
            error["type"] == "RATE_LIMITED"
                || error["message"]
                    .as_str()
                    .is_some_and(|message| message.to_lowercase().contains("something went wrong"))
        });
        let error = anyhow!("GitHub returned incomplete GraphQL data.");
        return Err(if transient {
            Failure::Retryable(error)
        } else {
            Failure::Fatal(error)
        });
    }
    envelope
        .data
        .filter(|data| !data.is_null())
        .ok_or_else(|| Failure::Fatal(anyhow!("GitHub returned no GraphQL data.")))
}

fn retry<T>(
    mut request: impl FnMut() -> std::result::Result<T, Failure>,
    mut sleep: impl FnMut(Duration),
) -> Result<T> {
    for attempt in 0..4 {
        match request() {
            Ok(value) => return Ok(value),
            Err(Failure::Retryable(_)) if attempt < 3 => {
                sleep(Duration::from_secs(2_u64.pow(attempt + 1)));
            }
            Err(Failure::Retryable(error) | Failure::Fatal(error)) => return Err(error),
        }
    }
    unreachable!("The fourth attempt always returns")
}

impl GitHub {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .user_agent("gabref-readme-stats/0.1.0")
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(90))
                .redirect(Policy::none())
                .build()?,
            token: token()?,
        })
    }

    fn request(&self, query: &str, variables: &Value) -> std::result::Result<Value, Failure> {
        let response = self
            .client
            .post("https://api.github.com/graphql")
            .bearer_auth(&self.token)
            .json(&json!({"query": query, "variables": variables}))
            .send()
            .map_err(|error| {
                if error.is_timeout() || error.is_connect() || error.is_body() {
                    Failure::Retryable(error.into())
                } else {
                    Failure::Fatal(error.into())
                }
            })?;
        let status = response.status();
        if !status.is_success() {
            let rate_limited = status == StatusCode::TOO_MANY_REQUESTS
                || (status == StatusCode::FORBIDDEN
                    && response
                        .headers()
                        .get("x-ratelimit-remaining")
                        .is_some_and(|value| value == "0"));
            let error = anyhow!("GitHub API request failed (HTTP {status}).");
            return Err(if rate_limited || status.is_server_error() {
                Failure::Retryable(error)
            } else {
                Failure::Fatal(error)
            });
        }
        let envelope = response
            .json()
            .map_err(|error| Failure::Fatal(error.into()))?;
        decode(envelope)
    }
}

impl Api for GitHub {
    fn query(&mut self, query: &str, variables: Value) -> Result<Value> {
        retry(|| self.request(query, &variables), thread::sleep)
    }
}

#[cfg(test)]
impl<F: FnMut(&str, Value) -> Result<Value>> Api for F {
    fn query(&mut self, query: &str, variables: Value) -> Result<Value> {
        self(query, variables)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_temporary_failures_with_bounded_backoff() {
        let mut attempts = 0;
        let mut delays = Vec::new();
        let result = retry(
            || {
                attempts += 1;
                if attempts < 3 {
                    Err(Failure::Retryable(anyhow!("HTTP 502")))
                } else {
                    Ok(42)
                }
            },
            |delay| delays.push(delay.as_secs()),
        )
        .unwrap();
        assert_eq!(result, 42);
        assert_eq!(attempts, 3);
        assert_eq!(delays, [2, 4]);
    }

    #[test]
    fn stops_after_four_temporary_failures() {
        let mut attempts = 0;
        let result: Result<()> = retry(
            || {
                attempts += 1;
                Err(Failure::Retryable(anyhow!("HTTP 503")))
            },
            |_| {},
        );
        assert!(result.is_err());
        assert_eq!(attempts, 4);
    }

    #[test]
    fn does_not_retry_invalid_credentials() {
        let mut attempts = 0;
        let result: Result<()> = retry(
            || {
                attempts += 1;
                Err(Failure::Fatal(anyhow!("HTTP 401")))
            },
            |_| panic!("Permanent failures must not sleep"),
        );
        assert!(result.is_err());
        assert_eq!(attempts, 1);
    }

    #[test]
    fn rejects_partial_and_missing_graphql_data() {
        assert!(
            decode(Envelope {
                data: Some(json!({"user": null})),
                errors: Some(vec![json!({})])
            })
            .is_err()
        );
        assert!(
            decode(Envelope {
                data: None,
                errors: None
            })
            .is_err()
        );
        assert!(
            decode(Envelope {
                data: Some(Value::Null),
                errors: None
            })
            .is_err()
        );
    }

    #[test]
    fn retries_graphql_rate_limits() {
        let result = decode(Envelope {
            data: None,
            errors: Some(vec![json!({"type": "RATE_LIMITED"})]),
        });
        assert!(matches!(result, Err(Failure::Retryable(_))));
    }
}
