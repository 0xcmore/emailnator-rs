use reqwest::header::{
    CONTENT_TYPE,
    SET_COOKIE,
};
use reqwest::redirect::Policy;
use reqwest::{
    Client,
    Error as ReqwestErrorKind,
    StatusCode,
};
use sonic_rs::{
    Deserialize,
    Error as SonicErrorKind,
    Serialize,
};
use std::time::Duration;
use thiserror::Error;
use urlencoding::decode;

use crate::header::{
    APPLICATION_JSON,
    USER_AGENT,
    X_XSRF_TOKEN,
    XSRF_COOKIE_PREFIX,
    XSRF_COOKIE_SEPARATOR,
};
use crate::url::{
    GENERATE_EMAIL,
    HOMEPAGE,
    INBOX,
};

mod header;
mod url;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Request(#[from] ReqwestErrorKind),
    #[error(transparent)]
    Parse(#[from] SonicErrorKind),
    #[error("Rate limited by Cloudflare. Retry later")]
    RateLimited,
    #[error("No email kinds provided. Must provide at least one")]
    NoEmailKinds,
    #[error("Count cannot be zero. Must provide at least one")]
    ZeroCount,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EmailKind {
    Domain,
    PlusGmail,
    DotGmail,
    GoogleMail,
}

#[derive(Debug, Deserialize)]
pub struct Inbox {
    #[serde(rename = "messageData")]
    pub message: Vec<MailHeader>,
}

#[derive(Debug, Deserialize)]
pub struct MailHeader {
    #[serde(rename = "messageID")]
    pub id: String,
    pub from: String,
    pub subject: String,
}

pub struct Emailnator {
    http: Client,
    xsrf: String,
}

impl Emailnator {
    pub async fn new() -> Result<Self, Error> {
        let http = Client::builder()
            .user_agent(USER_AGENT)
            .cookie_store(true)
            .http2_prior_knowledge()
            .redirect(Policy::limited(2))
            .gzip(true)
            .tcp_keepalive(Duration::from_secs(80))
            .build()?;

        let xsrf = http
            .get(HOMEPAGE)
            .send()
            .await?
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .find_map(|cookie| {
                decode(
                    cookie
                        .to_str()
                        .ok()?
                        .strip_prefix(XSRF_COOKIE_PREFIX)?
                        .split(XSRF_COOKIE_SEPARATOR)
                        .next()?,
                )
                .ok()
                .map(|cookie| cookie.into_owned())
            })
            .ok_or(Error::RateLimited)?;

        Ok(Self { http, xsrf })
    }

    pub async fn create_emails(
        &self,
        kinds: &[EmailKind],
        count: u32,
    ) -> Result<Vec<String>, Error> {
        if kinds.is_empty() {
            return Err(Error::NoEmailKinds);
        }
        if count == 0 {
            return Err(Error::ZeroCount);
        }

        #[derive(Serialize)]
        struct GetEmail<'a> {
            email: &'a [EmailKind],
            #[serde(rename = "emailNo")]
            count: u32,
        }

        #[derive(Deserialize)]
        struct GeneratedEmails {
            email: Vec<String>,
        }

        let payload = GetEmail {
            email: kinds,
            count,
        };

        let body = sonic_rs::to_vec(&payload)?;

        let response = self
            .http
            .post(GENERATE_EMAIL)
            .header(CONTENT_TYPE, APPLICATION_JSON)
            .header(X_XSRF_TOKEN, &self.xsrf)
            .body(body)
            .send()
            .await?;

        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            return Err(Error::RateLimited);
        }

        let response = response.bytes().await?;
        let response: GeneratedEmails = sonic_rs::from_slice(&response)?;

        Ok(response.email)
    }

    pub async fn fetch_inbox(
        &self,
        email: &str,
    ) -> Result<Inbox, Error> {
        #[derive(Serialize)]
        struct GetInbox<'a> {
            email: &'a str,
        }

        let payload = GetInbox { email };

        let body = sonic_rs::to_vec(&payload)?;

        let response = self
            .http
            .post(INBOX)
            .header(CONTENT_TYPE, APPLICATION_JSON)
            .header(X_XSRF_TOKEN, &self.xsrf)
            .body(body)
            .send()
            .await?;

        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            return Err(Error::RateLimited);
        }

        let response = response.bytes().await?;
        let response: Inbox = sonic_rs::from_slice(&response)?;

        Ok(response)
    }

    pub async fn read_message(
        &self,
        email: &str,
        id: &str,
    ) -> Result<String, Error> {
        #[derive(Serialize)]
        struct GetMessage<'a> {
            email: &'a str,
            #[serde(rename = "messageID")]
            id: &'a str,
        }

        let payload = GetMessage { email, id };

        let body = sonic_rs::to_vec(&payload)?;

        let response = self
            .http
            .post(INBOX)
            .header(CONTENT_TYPE, APPLICATION_JSON)
            .header(X_XSRF_TOKEN, &self.xsrf)
            .body(body)
            .send()
            .await?;

        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            return Err(Error::RateLimited);
        }

        let response = response.text().await?;
        Ok(response)
    }
}
