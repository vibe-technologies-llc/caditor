use std::io;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AiError {
    #[error(
        "No API key is configured for {provider}. Add one in Preferences › General › AI assistant."
    )]
    MissingKey { provider: crate::ApiProvider },
    #[error(
        "The custom endpoint address is not valid. Use an https:// URL, or http://localhost for a local server."
    )]
    InvalidEndpoint,
    #[error(
        "caditor could not reach the AI service. Check the internet connection or endpoint address."
    )]
    Unreachable,
    #[error("The AI service did not answer in time. Try again.")]
    TimedOut,
    #[error("The connection to the AI service could not be made secure.")]
    Insecure,
    #[error(
        "The AI service rejected the API key. Check the key in Preferences › General › AI assistant."
    )]
    Unauthorised,
    #[error("The AI service rate-limited the request. Wait a moment and try again.")]
    RateLimited { retry_after: Option<u64> },
    #[error("The AI service returned HTTP {status}: {body}")]
    ApiStatus { status: u16, body: String },
    #[error("The AI service sent an answer caditor could not read.")]
    MalformedStream,
    #[error("The AI request was stopped.")]
    Cancelled,
    #[error("The connection to the AI service failed ({0:?}). Try again.")]
    Io(io::ErrorKind),
}
