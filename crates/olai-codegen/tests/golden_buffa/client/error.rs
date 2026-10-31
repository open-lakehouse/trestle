// @generated — do not edit by hand.
//! Errors of the generated clients.

/// The result of a client call.
pub type Result<T, E = Error> = ::core::result::Result<T, E>;
/// The error of a client call.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The server answered with an error status.
    Api(ApiError),
    /// The request got no answer, or the answer could not be read.
    Http(::reqwest::Error),
    /// The transport failed before an answer, or after retries.
    Transport(::olai_http::Error),
    /// The request URL could not be built.
    Url(::url::ParseError),
    /// A body could not be written or read as JSON.
    Json(::serde_json::Error),
}

impl Error {
    /// The HTTP status of an error answer.
    pub fn status(&self) -> Option<u16> {
        match self {
            Error::Api(e) => Some(e.status),
            _ => None,
        }
    }

    /// The error code of an error answer that has one.
    pub fn code(&self) -> Option<&str> {
        match self {
            Error::Api(e) => e.code.as_deref(),
            _ => None,
        }
    }

    /// Whether the server answered `404 Not Found`.
    pub fn is_not_found(&self) -> bool {
        self.status() == Some(404)
    }
}

impl ::std::fmt::Display for Error {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        match self {
            Error::Api(e) => ::std::fmt::Display::fmt(e, f),
            Error::Http(e) => write!(f, "request failed: {e}"),
            Error::Transport(e) => write!(f, "request failed: {e}"),
            Error::Url(e) => write!(f, "invalid URL: {e}"),
            Error::Json(e) => write!(f, "invalid JSON: {e}"),
        }
    }
}

impl ::std::error::Error for Error {
    fn source(&self) -> Option<&(dyn ::std::error::Error + 'static)> {
        match self {
            Error::Api(e) => Some(e),
            Error::Http(e) => Some(e),
            Error::Transport(e) => Some(e),
            Error::Url(e) => Some(e),
            Error::Json(e) => Some(e),
        }
    }
}

impl ::core::convert::From<ApiError> for Error {
    fn from(e: ApiError) -> Self {
        Error::Api(e)
    }
}

impl ::core::convert::From<::reqwest::Error> for Error {
    fn from(e: ::reqwest::Error) -> Self {
        Error::Http(e)
    }
}

impl ::core::convert::From<::olai_http::Error> for Error {
    fn from(e: ::olai_http::Error) -> Self {
        Error::Transport(e)
    }
}

impl ::core::convert::From<::url::ParseError> for Error {
    fn from(e: ::url::ParseError) -> Self {
        Error::Url(e)
    }
}

impl ::core::convert::From<::serde_json::Error> for Error {
    fn from(e: ::serde_json::Error) -> Self {
        Error::Json(e)
    }
}

/// An error answer of the server.
#[derive(Debug, Clone, PartialEq)]
pub struct ApiError {
    /// The HTTP status.
    pub status: u16,
    /// The error code of the body, as sent. `None` when the body has none.
    pub code: Option<String>,
    /// The message of the body, or the body text when it is not a JSON error.
    pub message: String,
    /// The details of the body.
    pub details: ::std::vec::Vec<::serde_json::Value>,
    /// The body as sent, as text.
    pub body: String,
}

impl ApiError {
    /// Read an error answer with HTTP status `status` and body `body`.
    pub fn from_body(status: u16, body: &[u8]) -> Self {
        let json = ::serde_json::from_slice::<::serde_json::Value>(body).ok();
        let object = json.as_ref().and_then(|v| v.as_object());
        let text = |key: &str| {
            object.and_then(|o| o.get(key)).and_then(|v| v.as_str()).map(str::to_string)
        };
        let code = ["error_code", "errorCode"].into_iter().find_map(text);
        let message = ["message"]
            .into_iter()
            .find_map(text)
            .unwrap_or_else(|| String::from_utf8_lossy(body).trim().to_string());
        let details = object
            .and_then(|o| o.get("details"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        Self {
            status,
            code,
            message,
            details,
            body: String::from_utf8_lossy(body).into_owned(),
        }
    }
}

impl ::std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        match &self.code {
            Some(code) => write!(f, "{code}: {}", self.message),
            None => write!(f, "HTTP {}: {}", self.status, self.message),
        }
    }
}

impl ::std::error::Error for ApiError {}
/// The error of an answer with an error status.
pub(crate) async fn parse_error_response(response: ::reqwest::Response) -> Error {
    let status = response.status().as_u16();
    match response.bytes().await {
        Ok(body) => Error::Api(ApiError::from_body(status, &body)),
        Err(e) => Error::Http(e),
    }
}
