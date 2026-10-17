use serde::{Deserialize, de::DeserializeOwned};

use crate::{
    encoding::form,
    error::{Error, Subject},
    service::Service,
    session::Session,
    transport::{Body, Request, Response, Transport},
};

const DEFAULT_LIFETIME_SECONDS: u64 = 3600;

#[derive(Deserialize)]
pub struct Token {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

impl Token {
    pub fn into_session(self, previous: Option<&str>, now: u64) -> Session {
        Session {
            access_token: self.access_token,
            refresh_token: self.refresh_token.or_else(|| previous.map(str::to_owned)),
            expires_at: now.saturating_add(self.expires_in.unwrap_or(DEFAULT_LIFETIME_SECONDS)),
        }
    }
}

pub fn checked(service: Service, response: Response, subject: Subject) -> Result<Response, Error> {
    match response.status {
        200..=299 => Ok(response),
        401 => Err(Error::SignedOut),
        403 => Err(Error::Forbidden { service }),
        404 => Err(Error::NotFound { subject }),
        429 => Err(Error::RateLimited {
            service,
            retry_after_seconds: response.retry_after_seconds,
        }),
        status => Err(Error::Refused { service, status }),
    }
}

pub fn read<T: DeserializeOwned>(
    service: Service,
    response: &Response,
    subject: Subject,
) -> Result<T, Error> {
    serde_json::from_slice(&response.body).map_err(|_| Error::Unreadable { service, subject })
}

pub fn get<T: DeserializeOwned>(
    service: Service,
    net: &dyn Transport,
    session: &Session,
    url: String,
    subject: Subject,
) -> Result<T, Error> {
    let request = Request::get(url).with_bearer(&session.access_token);
    let response = checked(service, net.send(&request)?, subject)?;
    read(service, &response, subject)
}

pub fn token(
    service: Service,
    net: &dyn Transport,
    url: &str,
    fields: &[(&str, &str)],
) -> Result<Token, Error> {
    let request = Request::post(url.to_owned(), Body::Form(form(fields)));
    let response = net.send(&request)?;
    if matches!(response.status, 400 | 401) {
        return Err(Error::SignedOut);
    }
    let response = checked(service, response, Subject::Token)?;
    read(service, &response, Subject::Token)
}
