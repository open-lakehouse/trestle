//! The generated `codegen/error.rs` of the Python bindings: the error/result types the binding
//! methods return, and the PyO3 exception classes a client error is raised as (a base class,
//! [`PythonOptions::error_base_class`], and one subclass per kind of HTTP error status).
//!
//! [`PythonOptions::error_base_class`]: crate::codegen::PythonOptions::error_base_class

use quote::{format_ident, quote};

use crate::codegen::{BindingsConfig, PythonOptions, format_tokens_static};

/// The exception subclasses: `(name, statuses, doc)`. An empty `statuses` means every 5xx.
const ERROR_CLASSES: &[(&str, &[u16], &str)] = &[
    (
        "BadRequestError",
        &[400],
        "The server answered `400 Bad Request`: the request is not valid.",
    ),
    (
        "UnauthenticatedError",
        &[401],
        "The server answered `401 Unauthorized`: the request has no valid credentials.",
    ),
    (
        "PermissionDeniedError",
        &[403],
        "The server answered `403 Forbidden`: the caller may not do this.",
    ),
    (
        "NotFoundError",
        &[404],
        "The server answered `404 Not Found`: the resource does not exist.",
    ),
    (
        "ConflictError",
        &[409],
        "The server answered `409 Conflict`: the resource exists or changed.",
    ),
    (
        "RateLimitError",
        &[429],
        "The server answered `429 Too Many Requests`.",
    ),
    (
        "ServerError",
        &[],
        "The server answered with a `5xx` status: it failed to do the request.",
    ),
];

/// The exception classes: the base class, then its subclasses.
pub(crate) fn class_idents(python: &PythonOptions) -> Vec<proc_macro2::Ident> {
    std::iter::once(python.error_base_class.as_str())
        .chain(ERROR_CLASSES.iter().map(|(name, _, _)| *name))
        .map(|name| format_ident!("{}", name))
        .collect()
}

/// The `error.rs` file of the Python bindings.
pub(crate) fn generate(bindings: &BindingsConfig, python: &PythonOptions) -> String {
    let error = format_ident!("{}", bindings.py_error_type);
    let result = format_ident!("{}", bindings.py_result_type);
    let client_crate = format_ident!("{}", bindings.client_crate_name);
    let base_name = &python.error_base_class;
    let base = format_ident!("{}", python.error_base_class);
    let module_arg = super::module_arg(python, None);
    let error_doc = format!(
        " The error of a binding call: a client error (raised as a `{base_name}`) or a Python error."
    );

    let subclasses = ERROR_CLASSES.iter().map(|(name, _, doc)| {
        let ident = format_ident!("{}", name);
        let doc = format!(" {doc}");
        quote! {
            #[doc = #doc]
            #[::pyo3::pyclass(extends = #base, frozen, name = #name, #module_arg)]
            pub struct #ident;

            #[::pyo3::pymethods]
            impl #ident {
                #[new]
                #[pyo3(signature = (message, code = None, status = None, details = None))]
                fn new(
                    py: Python<'_>,
                    message: String,
                    code: Option<String>,
                    status: Option<u16>,
                    details: Option<Bound<'_, PyList>>,
                ) -> PyClassInitializer<Self> {
                    PyClassInitializer::from(#base::new(py, message, code, status, details))
                        .add_subclass(Self)
                }
            }
        }
    });
    let status_arms = ERROR_CLASSES.iter().map(|(name, statuses, _)| {
        let ident = format_ident!("{}", name);
        let pattern = if statuses.is_empty() {
            quote! { Some(500..=599) }
        } else {
            let statuses = statuses.iter();
            quote! { #(Some(#statuses))|* }
        };
        quote! { #pattern => Bound::new(py, base.add_subclass(#ident))?.into_any(), }
    });
    let tokens = quote! {
        //! Errors of the Python bindings.

        use pyo3::exceptions::PyException;
        use pyo3::prelude::*;
        use pyo3::types::{PyDict, PyList, PyString};

        /// The client error type.
        type ClientError = #client_crate::error::Error;

        /// The result of a binding call.
        pub type #result<T> = ::core::result::Result<T, #error>;

        #[doc = #error_doc]
        #[derive(Debug)]
        pub enum #error {
            /// A client error.
            Client(ClientError),
            /// A Python error.
            Python(PyErr),
        }

        impl ::core::convert::From<ClientError> for #error {
            fn from(e: ClientError) -> Self {
                #error::Client(e)
            }
        }

        impl ::core::convert::From<PyErr> for #error {
            fn from(e: PyErr) -> Self {
                #error::Python(e)
            }
        }

        impl ::core::convert::From<::url::ParseError> for #error {
            fn from(e: ::url::ParseError) -> Self {
                #error::Client(ClientError::from(e))
            }
        }

        impl ::core::convert::From<#error> for PyErr {
            fn from(e: #error) -> Self {
                match e {
                    #error::Python(e) => e,
                    #error::Client(e) => {
                        Python::attach(|py| client_exception(py, e).unwrap_or_else(|err| err))
                    }
                }
            }
        }

        /// An error answer of the server, or a request that got no answer.
        #[::pyo3::pyclass(extends = PyException, subclass, frozen, name = #base_name, #module_arg)]
        pub struct #base {
            message: String,
            code: Option<String>,
            status: Option<u16>,
            details: Py<PyList>,
        }

        #[::pyo3::pymethods]
        impl #base {
            #[new]
            #[pyo3(signature = (message, code = None, status = None, details = None))]
            fn new(
                py: Python<'_>,
                message: String,
                code: Option<String>,
                status: Option<u16>,
                details: Option<Bound<'_, PyList>>,
            ) -> Self {
                Self {
                    message,
                    code,
                    status,
                    details: details.map_or_else(|| PyList::empty(py).unbind(), Bound::unbind),
                }
            }

            /// Sets `args` to the message, as `BaseException.__init__` (which takes no keyword
            /// arguments) would.
            #[pyo3(signature = (message, code = None, status = None, details = None))]
            #[allow(unused_variables)]
            fn __init__(
                slf: &Bound<'_, Self>,
                message: String,
                code: Option<String>,
                status: Option<u16>,
                details: Option<Bound<'_, PyList>>,
            ) -> PyResult<()> {
                slf.setattr("args", (message,))
            }

            /// The error message.
            #[getter]
            fn message(&self) -> &str {
                &self.message
            }

            /// The error code of the answer, such as `NOT_FOUND`, if any.
            #[getter]
            fn code(&self) -> Option<&str> {
                self.code.as_deref()
            }

            /// The HTTP status of the answer. `None` when the request got no answer.
            #[getter]
            fn status(&self) -> Option<u16> {
                self.status
            }

            /// The details of the answer.
            #[getter]
            fn details<'py>(&self, py: Python<'py>) -> Bound<'py, PyList> {
                self.details.bind(py).clone()
            }

            fn __str__(&self) -> String {
                match (&self.code, self.status) {
                    (Some(code), _) => format!("{code}: {}", self.message),
                    (None, Some(status)) => format!("HTTP {status}: {}", self.message),
                    (None, None) => self.message.clone(),
                }
            }

            fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
                let this = slf.get();
                let py = slf.py();
                let mut parts = vec![PyString::new(py, &this.message).repr()?.to_string()];
                if let Some(code) = &this.code {
                    parts.push(format!("code={}", PyString::new(py, code).repr()?));
                }
                if let Some(status) = this.status {
                    parts.push(format!("status={status}"));
                }
                Ok(format!("{}({})", slf.get_type().name()?, parts.join(", ")))
            }

            /// For `pickle`.
            #[allow(clippy::type_complexity)]
            fn __reduce__<'py>(
                slf: &Bound<'py, Self>,
            ) -> (
                Bound<'py, ::pyo3::types::PyType>,
                (String, Option<String>, Option<u16>, Py<PyList>),
            ) {
                let this = slf.get();
                let args = (
                    this.message.clone(),
                    this.code.clone(),
                    this.status,
                    this.details.clone_ref(slf.py()),
                );
                (slf.get_type(), args)
            }
        }

        #(#subclasses)*

        /// The exception of a client error: the subclass of its HTTP status.
        fn client_exception(py: Python<'_>, e: ClientError) -> PyResult<PyErr> {
            let (message, code, status, details) = match e {
                ClientError::Api(api) => (api.message, api.code, Some(api.status), api.details),
                other => {
                    // Python has no cause chain, so append the causes `Display` does not show.
                    let mut message = other.to_string();
                    let mut cause = ::std::error::Error::source(&other).and_then(::std::error::Error::source);
                    while let Some(error) = cause {
                        message.push_str(": ");
                        message.push_str(&error.to_string());
                        cause = error.source();
                    }
                    (message, None, None, ::std::vec::Vec::new())
                }
            };
            let details = details
                .iter()
                .map(|d| json_value(py, d))
                .collect::<PyResult<::std::vec::Vec<_>>>()?;
            let args = (message.clone(),);
            let base = PyClassInitializer::from(#base {
                message,
                code,
                status,
                details: PyList::new(py, details)?.unbind(),
            });
            let exception = match status {
                #(#status_arms)*
                _ => Bound::new(py, base)?.into_any(),
            };
            // As `__init__` does for an exception made in Python.
            exception.setattr("args", args)?;
            Ok(PyErr::from_value(exception))
        }

        /// A JSON value as a Python object.
        fn json_value<'py>(py: Python<'py>, value: &::serde_json::Value) -> PyResult<Bound<'py, PyAny>> {
            use ::serde_json::Value;
            Ok(match value {
                Value::Null => py.None().into_bound(py),
                Value::Bool(b) => b.into_pyobject(py)?.to_owned().into_any(),
                Value::Number(n) => match (n.as_i64(), n.as_u64()) {
                    (Some(i), _) => i.into_pyobject(py)?.into_any(),
                    (None, Some(u)) => u.into_pyobject(py)?.into_any(),
                    (None, None) => n.as_f64().unwrap_or(f64::NAN).into_pyobject(py)?.into_any(),
                },
                Value::String(s) => PyString::new(py, s).into_any(),
                Value::Array(items) => PyList::new(
                    py,
                    items
                        .iter()
                        .map(|item| json_value(py, item))
                        .collect::<PyResult<::std::vec::Vec<_>>>()?,
                )?
                .into_any(),
                Value::Object(map) => {
                    let dict = PyDict::new(py);
                    for (key, item) in map {
                        dict.set_item(key, json_value(py, item)?)?;
                    }
                    dict.into_any()
                }
            })
        }
    };
    format_tokens_static(tokens)
}
