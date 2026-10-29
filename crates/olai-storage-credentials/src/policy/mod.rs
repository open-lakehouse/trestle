//! Downscoping documents: the pure, cloud-specific restrictions a server attaches when it
//! mints a credential for one [`StorageScope`](crate::StorageScope).
//!
//! - [`s3`]: the inline session policy for STS `AssumeRole`.
//! - [`azure`]: SAS permissions and resource scoping.
//! - [`gcs`]: the Credential Access Boundary for a downscoped token exchange.
//!
//! These only build the documents. Calling the cloud (STS, user-delegation keys, token
//! exchange) is the minter's job. Every builder takes the scope's bucket and path from a
//! parsed location and emits them through `serde_json` (and, for GCS, CEL string
//! escaping), so a crafted path cannot change the document's structure.

pub mod azure;
pub mod gcs;
pub mod s3;
