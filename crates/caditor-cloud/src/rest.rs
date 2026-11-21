use serde::{Deserialize, de::DeserializeOwned};
use ureq::http::Uri;

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

pub fn is_within(link: &str, base: &str) -> bool {
    let (Ok(link), Ok(base)) = (link.parse::<Uri>(), base.parse::<Uri>()) else {
        return false;
    };

    let (Some(link_authority), Some(base_authority)) = (link.authority(), base.authority()) else {
        return false;
    };
    let base_path = base.path().trim_end_matches('/');
    let under_base = link
        .path()
        .strip_prefix(base_path)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'));
    !link_authority.as_str().contains('@')
        && link.scheme() == base.scheme()
        && link_authority
            .host()
            .eq_ignore_ascii_case(base_authority.host())
        && port_of(&link) == port_of(&base)
        && under_base
}

fn port_of(uri: &Uri) -> Option<u16> {
    uri.port_u16().or_else(|| match uri.scheme_str() {
        Some("https") => Some(443),
        Some("http") => Some(80),
        _ => None,
    })
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grant {
    Code,
    Refresh,
}

impl Grant {
    fn refusal(self, service: Service) -> Error {
        match self {
            Self::Code => Error::SignInRefused { service },
            Self::Refresh => Error::SignedOut,
        }
    }
}

pub fn token(
    service: Service,
    net: &dyn Transport,
    url: &str,
    grant: Grant,
    fields: &[(&str, &str)],
) -> Result<Token, Error> {
    let request = Request::post(url.to_owned(), Body::Form(form(fields)));
    let response = net.send(&request)?;

    if matches!(response.status, 400 | 401) {
        return Err(grant.refusal(service));
    }

    let response = checked(service, response, Subject::Token)?;
    read(service, &response, Subject::Token)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FUSION: &str = "https://developer.api.autodesk.com";
    const ONSHAPE: &str = "https://cad.onshape.com/api/";

    #[test]
    fn a_link_on_the_same_host_under_the_base_path_is_within() {
        assert!(is_within(
            "https://developer.api.autodesk.com/project/v1/hubs?page[number]=1",
            FUSION
        ));
        assert!(is_within(
            "https://DEVELOPER.api.autodesk.com:443/x",
            FUSION
        ));
        assert!(is_within(
            "https://cad.onshape.com/api/v10/documents?offset=20",
            ONSHAPE
        ));
    }

    #[test]
    fn a_link_to_another_host_scheme_port_or_path_is_not_within() {
        let links = [
            "https://developer.api.autodesk.com.evil.example/x",
            "https://developer.api.autodesk.comevil.example/x",
            "https://developer.api.autodesk.com@evil.example/x",
            "https://evil.example@developer.api.autodesk.com/x",
            "https://developer.api.autodesk.com:8443/x",
            "http://developer.api.autodesk.com/x",
            "https://evil.example/https://developer.api.autodesk.com",
            "//developer.api.autodesk.com/x",
            "/project/v1/hubs",
            "not a link",
        ];

        for link in links {
            assert!(!is_within(link, FUSION), "{link}");
        }

        let links = [
            "https://cad.onshape.com.evil.example/api/x",
            "https://cad.onshape.com@evil.example/api/x",
            "https://cad.onshape.com:444/api/x",
            "http://cad.onshape.com/api/x",
            "https://cad.onshape.com/apiary/x",
            "https://cad.onshape.com/other",
        ];
        for link in links {
            assert!(!is_within(link, ONSHAPE), "{link}");
        }
    }
}
