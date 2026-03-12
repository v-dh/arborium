#[derive(Debug, Clone, serde::Deserialize)]
struct GitHubDeviceCode {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    expires_in: u64,
    interval: Option<u64>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct GitHubDeviceCodeResponse {
    #[serde(default)]
    device_code: String,
    #[serde(default)]
    user_code: String,
    #[serde(default)]
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: Option<String>,
    #[serde(default)]
    expires_in: u64,
    #[serde(default)]
    interval: Option<u64>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct GitHubTokenResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

#[derive(Debug, Clone)]
struct GitHubAccessToken {
    access_token: String,
    token_type: Option<String>,
    scope: Option<String>,
}

fn github_oauth_http_agent() -> ureq::Agent {
    let config = ureq::config::Config::builder()
        .http_status_as_error(false)
        .build();
    ureq::Agent::new_with_config(config)
}

fn github_request_device_code(client_id: &str) -> Result<GitHubDeviceCode, String> {
    let response = github_oauth_http_agent()
        .post(GITHUB_OAUTH_DEVICE_CODE_URL)
        .header("Accept", "application/json")
        .header("User-Agent", "Arbor")
        .send_form([("client_id", client_id), ("scope", GITHUB_OAUTH_SCOPE)])
        .map_err(|error| format!("failed to start GitHub OAuth flow: {error}"))?;

    let status = response.status();
    let body = response
        .into_body()
        .read_to_string()
        .map_err(|error| format!("failed to read GitHub OAuth response: {error}"))?;
    let payload: GitHubDeviceCodeResponse = serde_json::from_str(&body)
        .map_err(|error| format!("failed to parse GitHub OAuth response: {error}"))?;

    if !status.is_success() {
        let reason = payload
            .error
            .unwrap_or_else(|| "request_rejected".to_owned());
        let description = payload
            .error_description
            .unwrap_or_else(|| "request was rejected".to_owned());
        return Err(format!(
            "failed to start GitHub OAuth flow: {reason} ({description})"
        ));
    }

    let device_code = non_empty_trimmed_str(&payload.device_code)
        .map(str::to_owned)
        .ok_or_else(|| "GitHub OAuth response was missing device_code".to_owned())?;
    let user_code = non_empty_trimmed_str(&payload.user_code)
        .map(str::to_owned)
        .ok_or_else(|| "GitHub OAuth response was missing user_code".to_owned())?;
    let verification_uri = non_empty_trimmed_str(&payload.verification_uri)
        .map(str::to_owned)
        .ok_or_else(|| "GitHub OAuth response was missing verification_uri".to_owned())?;
    let expires_in = if payload.expires_in == 0 {
        return Err("GitHub OAuth response was missing expires_in".to_owned());
    } else {
        payload.expires_in
    };

    Ok(GitHubDeviceCode {
        device_code,
        user_code,
        verification_uri,
        verification_uri_complete: payload
            .verification_uri_complete
            .as_deref()
            .and_then(non_empty_trimmed_str)
            .map(str::to_owned),
        expires_in,
        interval: payload.interval,
    })
}

fn github_poll_device_access_token(
    client_id: &str,
    device_code: &GitHubDeviceCode,
) -> Result<GitHubAccessToken, String> {
    let deadline = Instant::now() + Duration::from_secs(device_code.expires_in.max(5));
    let mut poll_interval = Duration::from_secs(
        device_code
            .interval
            .unwrap_or(GITHUB_DEVICE_FLOW_POLL_MIN_INTERVAL.as_secs())
            .max(GITHUB_DEVICE_FLOW_POLL_MIN_INTERVAL.as_secs()),
    );

    loop {
        if Instant::now() >= deadline {
            return Err("GitHub authorization timed out before completion".to_owned());
        }

        std::thread::sleep(poll_interval);

        let payload = github_request_access_token(client_id, &device_code.device_code)?;
        if let Some(access_token) = payload
            .access_token
            .as_deref()
            .and_then(non_empty_trimmed_str)
            .map(str::to_owned)
        {
            return Ok(GitHubAccessToken {
                access_token,
                token_type: payload.token_type,
                scope: payload.scope,
            });
        }

        match payload.error.as_deref() {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                poll_interval += Duration::from_secs(5);
                continue;
            },
            Some("access_denied") => {
                return Err("GitHub authorization was denied".to_owned());
            },
            Some("expired_token") => {
                return Err("GitHub authorization code expired".to_owned());
            },
            Some(other) => {
                let description = payload
                    .error_description
                    .as_deref()
                    .and_then(non_empty_trimmed_str)
                    .unwrap_or("request failed");
                return Err(format!("GitHub OAuth failed: {other} ({description})"));
            },
            None => {
                return Err("GitHub OAuth response was missing an access token".to_owned());
            },
        }
    }
}

fn github_request_access_token(
    client_id: &str,
    device_code: &str,
) -> Result<GitHubTokenResponse, String> {
    let response = github_oauth_http_agent()
        .post(GITHUB_OAUTH_ACCESS_TOKEN_URL)
        .header("Accept", "application/json")
        .header("User-Agent", "Arbor")
        .send_form([
            ("client_id", client_id),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ])
        .map_err(|error| format!("failed to poll GitHub OAuth status: {error}"))?;

    let status = response.status();
    let body = response
        .into_body()
        .read_to_string()
        .map_err(|error| format!("failed to read GitHub OAuth token response: {error}"))?;
    let payload: GitHubTokenResponse = serde_json::from_str(&body)
        .map_err(|error| format!("failed to parse GitHub OAuth token response: {error}"))?;

    if status.is_success() || payload.error.is_some() || payload.access_token.is_some() {
        return Ok(payload);
    }

    Err("GitHub OAuth token request failed".to_owned())
}
