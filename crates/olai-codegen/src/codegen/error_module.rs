//! The generated client error module (`error.rs`): the `Error`/`Result` types, an `ApiError`
//! with the HTTP status and the code, message and details of the JSON error body (keys from
//! [`ErrorModuleConfig`]), and the `parse_error_response` the client methods call.

use proc_macro2::TokenStream;
use quote::quote;

use super::{
    CodeGenConfig, ErrorModuleConfig, ModelsPath, Runtime, extract_type_ident, format_tokens,
    package_models_segment, package_models_version,
};
use crate::error::{Error, Result};
use crate::parsing::CodeGenMetadata;
use crate::utils::metadata_key;
use crate::utils::strings::lower_camel;

/// The path of the generated error module, as the generated code names it.
pub(crate) const CLIENT_ERROR_MODULE: &str = "crate::codegen::error";

/// The `error.rs` file of the client output.
pub(crate) fn generate(
    config: &CodeGenConfig,
    errors: &ErrorModuleConfig,
    metadata: &CodeGenMetadata,
) -> Result<String> {
    // Code keys, in order: the configured key, its lowerCamelCase form, the fallbacks.
    let code_alias =
        Some(lower_camel(&errors.code_field)).filter(|alias| *alias != errors.code_field);
    let code_fields = std::iter::once(errors.code_field.clone())
        .chain(code_alias)
        .chain(errors.more_code_fields.iter().cloned());
    let message_fields =
        std::iter::once(&errors.message_field).chain(errors.more_message_fields.iter());
    let envelope_fields = &errors.envelope_fields;
    let envelope = (!envelope_fields.is_empty()).then(|| {
        quote! {
            let object = [#(#envelope_fields),*]
                .into_iter()
                .find_map(|key| object.and_then(|o| o.get(key)).and_then(|v| v.as_object()))
                .or(object);
        }
    });
    let details = match &errors.details_field {
        Some(field) => quote! {
            object
                .and_then(|o| o.get(#field))
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
        },
        None => quote! { ::std::vec::Vec::new() },
    };

    let code_enum = errors
        .code_enum
        .as_deref()
        .map(|name| code_enum_accessor(config, metadata, name))
        .transpose()?;

    // `CloudClient` fails with `olai_http::Error`; the reqwest-based transports with `reqwest::Error`.
    let cloud = config.uses_default_transport() || config.dual_transport();
    let cloud_cfg = if config.dual_transport() {
        quote! { #[cfg(not(target_arch = "wasm32"))] }
    } else {
        quote! {}
    };
    let transport_variant = cloud.then(|| {
        quote! {
            /// The transport failed before an answer, or after retries.
            #cloud_cfg
            Transport(::olai_http::Error),
        }
    });
    let transport_display = cloud.then(|| {
        quote! {
            #cloud_cfg
            Error::Transport(e) => write!(f, "request failed: {e}"),
        }
    });
    let transport_source = cloud.then(|| {
        quote! {
            #cloud_cfg
            Error::Transport(e) => Some(e),
        }
    });
    let transport_from = cloud.then(|| {
        quote! {
            #cloud_cfg
            impl ::core::convert::From<::olai_http::Error> for Error {
                fn from(e: ::olai_http::Error) -> Self {
                    Error::Transport(e)
                }
            }
        }
    });

    let tokens = quote! {
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
            #transport_variant
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
                    #transport_display
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
                    #transport_source
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

        #transport_from

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
                #envelope
                let text = |key: &str| {
                    object
                        .and_then(|o| o.get(key))
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                };
                let code = [#(#code_fields),*].into_iter().find_map(text);
                let message = [#(#message_fields),*]
                    .into_iter()
                    .find_map(text)
                    .unwrap_or_else(|| String::from_utf8_lossy(body).trim().to_string());
                let details = #details;
                Self {
                    status,
                    code,
                    message,
                    details,
                    body: String::from_utf8_lossy(body).into_owned(),
                }
            }

            #code_enum
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
    };
    format_tokens(tokens)
}

/// The `ApiError::error_code` method that reads the code as the configured proto enum.
fn code_enum_accessor(
    config: &CodeGenConfig,
    metadata: &CodeGenMetadata,
    name: &str,
) -> Result<TokenStream> {
    let key = metadata_key(name);
    if !metadata.enums.contains_key(&key) {
        return Err(Error::Build(format!(
            "error code enum `{name}` is not an enum of the descriptor set"
        )));
    }
    let (package, _) = key[1..].rsplit_once('.').unwrap_or_default();
    let module = ModelsPath::from_template(&config.models_path_template).try_resolve(
        package_models_segment(package).unwrap_or(package),
        package_models_version(package),
    )?;
    let enum_ident = extract_type_ident(&key);
    let parse = match config.runtime {
        Runtime::Buffa => quote! {
            <#module::#enum_ident as ::buffa::Enumeration>::from_proto_name(self.code.as_deref()?)
        },
        Runtime::Prost => quote! {
            #module::#enum_ident::from_str_name(self.code.as_deref()?)
        },
    };
    Ok(quote! {
        /// The error code as a known code; `None` for no code or an unknown one.
        pub fn error_code(&self) -> Option<#module::#enum_ident> {
            #parse
        }
    })
}
