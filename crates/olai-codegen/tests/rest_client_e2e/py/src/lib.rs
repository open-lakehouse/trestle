//! The generated Python bindings, mounted under `crate::codegen` as a binding crate mounts them.

include!(concat!(env!("OUT_DIR"), "/python_mod.rs"));

/// The directory of the generated Python package `demo`.
pub const PACKAGE_DIR: &str = concat!(env!("OUT_DIR"), "/pkg");

/// A hand-written submodule of the extension module, `demo._core.extra`, as a binding crate
/// adds one (`submodules` in the generation config).
#[pyo3::pymodule]
pub mod extra {
    #[pymodule_export]
    use super::answer;

    #[cfg(feature = "cloud")]
    #[pymodule_export]
    use super::counting_provider;
}

/// The answer, from a hand-written function of the submodule.
#[pyo3::pyfunction]
fn answer() -> u32 {
    42
}

/// A token provider made in Rust, as a binding crate hands one to `token_provider=`: the
/// tokens `t1`, `t2`, … at each request.
#[cfg(feature = "cloud")]
#[pyo3::pyfunction]
fn counting_provider() -> codegen::RustTokenProvider {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use olai_http::RequestSigner;
    use olai_http::reqwest::RequestBuilder;

    #[derive(Debug, Default)]
    struct Counting(AtomicUsize);

    impl RequestSigner for Counting {
        fn sign<'a>(
            &'a self,
            request: RequestBuilder,
        ) -> futures::future::BoxFuture<'a, olai_http::Result<RequestBuilder>> {
            let n = self.0.fetch_add(1, Ordering::SeqCst) + 1;
            Box::pin(async move { Ok(request.bearer_auth(format!("t{n}"))) })
        }
    }

    codegen::RustTokenProvider::new(std::sync::Arc::new(Counting::default()))
}

#[cfg(test)]
mod tests;
