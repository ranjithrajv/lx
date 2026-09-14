// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{Context, Result};

/// Shared HTTP client factory — DRY for `github`/`gitlab`/`gitea`/etc.
pub fn new_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(crate::constants::USER_AGENT)
        .connect_timeout(std::time::Duration::from_secs(
            crate::constants::CONNECT_TIMEOUT_SECS,
        ))
        .timeout(std::time::Duration::from_secs(
            crate::constants::READ_TIMEOUT_SECS,
        ))
        .build()
        .context("failed to build HTTP client")
}

/// Attempts (including the first) for [`send_get_with_retry`].
const MAX_ATTEMPTS: u32 = 3;

/// `GET url` with the given headers, retrying on network errors and 5xx
/// responses with exponential backoff (250ms, 500ms). Run hundreds of times
/// a day across independent CI jobs hitting five different forge APIs, a
/// single transient blip shouldn't fail the build. 4xx responses are never
/// retried — retrying a "not found" just delays the same error by two
/// request's worth of time.
pub fn send_get_with_retry_headers(
    http: &reqwest::blocking::Client,
    url: &str,
    headers: &[(&'static str, String)],
) -> reqwest::Result<reqwest::blocking::Response> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        let mut req = http.get(url);
        for (k, v) in headers {
            req = req.header(*k, v.clone());
        }
        let result = req.send();
        let should_retry = match &result {
            Ok(resp) => resp.status().is_server_error(),
            Err(e) => !e.is_builder(),
        };
        if !should_retry || attempt >= MAX_ATTEMPTS {
            return result;
        }
        std::thread::sleep(std::time::Duration::from_millis(
            250 * (1u64 << (attempt - 1)),
        ));
    }
}

/// Convenience wrapper for the common case: a single optional auth header.
pub fn send_get_with_retry(
    http: &reqwest::blocking::Client,
    url: &str,
    auth: Option<(&'static str, String)>,
) -> reqwest::Result<reqwest::blocking::Response> {
    let headers: Vec<_> = auth.into_iter().collect();
    send_get_with_retry_headers(http, url, &headers)
}

/// RFC 3986 percent-encoding (unreserved = alphanumerics plus `-_.~`) used for
/// query and path values by every forge client.
pub fn urlencode(s: &str) -> String {
    use percent_encoding::{utf8_percent_encode, AsciiSet};
    const UNRESERVED: &AsciiSet = &percent_encoding::CONTROLS
        .add(b' ')
        .add(b'!')
        .add(b'"')
        .add(b'#')
        .add(b'$')
        .add(b'%')
        .add(b'&')
        .add(b'\'')
        .add(b'(')
        .add(b')')
        .add(b'*')
        .add(b'+')
        .add(b',')
        .add(b'/')
        .add(b':')
        .add(b';')
        .add(b'<')
        .add(b'=')
        .add(b'>')
        .add(b'?')
        .add(b'@')
        .add(b'[')
        .add(b'\\')
        .add(b']')
        .add(b'^')
        .add(b'`')
        .add(b'{')
        .add(b'|')
        .add(b'}');
    utf8_percent_encode(s, UNRESERVED).to_string()
}

/// Encode `/` inside a path segment (GitLab/Gerrit project ids).
pub fn encode_path_segment(s: &str) -> String {
    s.replace('/', "%2F")
}
