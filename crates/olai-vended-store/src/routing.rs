//! An object store that routes each request to the store covering its path.

use std::fmt;
use std::ops::Range;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{self, BoxStream, StreamExt};
use object_store::path::Path;
use object_store::{
    CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta, ObjectStore,
    ObjectStoreExt, PutMultipartOptions, PutOptions, PutPayload, PutResult, RenameOptions,
};
use olai_storage_credentials::{StorageLocation, StorageProvider};

use crate::error::{Error, Result};
use crate::store::ScopedStore;

/// An [`ObjectStore`] that sends each request to the route with the longest matching
/// prefix.
///
/// Vended credentials cover one prefix of a bucket, so several tables in one bucket need
/// one store each. Registries such as DataFusion's resolve stores by `scheme://bucket`,
/// though, so all of them map to one key. Register a `RoutingObjectStore` under that key
/// and add a route per [`ScopedStore`]:
///
/// - Prefixes match whole path segments: a route for `db/t1` serves `db/t1/x`, never
///   `db/t10/x`. A request no route covers fails with `NotFound`.
/// - Requests are forwarded unchanged, so routes are bucket-rooted stores
///   ([`ScopedStore::root`]).
/// - Listing needs a prefix (there is no one store to ask for the whole bucket), and
///   copies and renames must stay within one route: no credential covers both ends of a
///   cross-route copy.
///
/// Clones share their routes.
#[derive(Clone, Default)]
pub struct RoutingObjectStore {
    inner: Arc<RwLock<Routes>>,
}

#[derive(Default)]
struct Routes {
    /// The bucket every route is in, once one is registered.
    bucket: Option<Bucket>,
    /// Most specific (most segments) first, so the first match is the longest.
    routes: Vec<(Path, Arc<dyn ObjectStore>)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Bucket {
    provider: StorageProvider,
    account: Option<String>,
    name: String,
}

impl Bucket {
    fn of(location: &StorageLocation) -> Self {
        Self {
            provider: location.provider(),
            account: location.account().map(str::to_owned),
            name: location.bucket().to_owned(),
        }
    }
}

impl RoutingObjectStore {
    /// A store with no routes.
    pub fn new() -> Self {
        Self::default()
    }

    /// Route every path at or below `store`'s prefix to its bucket-rooted store,
    /// replacing an existing route for the same prefix.
    ///
    /// All routes must be in one bucket: paths carry no bucket, so stores for two buckets
    /// could not be told apart.
    pub fn register(&self, store: &ScopedStore) -> Result<()> {
        let bucket = Bucket::of(store.scope().location());
        let mut inner = self.inner.write().unwrap();
        match &inner.bucket {
            Some(existing) if *existing != bucket => {
                return Err(Error::Unsupported {
                    location: store.scope().to_string(),
                    reason: format!(
                        "this router serves bucket `{}`; register stores for other buckets \
                         with another router",
                        existing.name
                    ),
                });
            }
            _ => inner.bucket = Some(bucket),
        }
        let prefix = store.prefix().clone();
        inner.routes.retain(|(p, _)| *p != prefix);
        inner.routes.push((prefix, store.root()));
        inner
            .routes
            .sort_by_key(|(p, _)| std::cmp::Reverse(p.parts().count()));
        Ok(())
    }

    /// Remove the route registered for exactly `prefix`. Returns whether there was one.
    pub fn deregister(&self, prefix: &Path) -> bool {
        let mut inner = self.inner.write().unwrap();
        let before = inner.routes.len();
        inner.routes.retain(|(p, _)| p != prefix);
        before != inner.routes.len()
    }

    /// The registered prefixes, most specific first.
    pub fn prefixes(&self) -> Vec<Path> {
        let inner = self.inner.read().unwrap();
        inner.routes.iter().map(|(p, _)| p.clone()).collect()
    }

    /// The store and prefix of the longest route covering `location`.
    fn route_with_prefix(&self, location: &Path) -> Option<(Path, Arc<dyn ObjectStore>)> {
        let inner = self.inner.read().unwrap();
        inner
            .routes
            .iter()
            .find(|(prefix, _)| location.prefix_matches(prefix))
            .map(|(p, s)| (p.clone(), Arc::clone(s)))
    }

    fn route(&self, location: &Path) -> object_store::Result<Arc<dyn ObjectStore>> {
        self.route_with_prefix(location)
            .map(|(_, store)| store)
            .ok_or_else(|| object_store::Error::NotFound {
                path: location.to_string(),
                source: format!("no route covers `{location}`").into(),
            })
    }

    fn route_prefix(&self, prefix: Option<&Path>) -> object_store::Result<Arc<dyn ObjectStore>> {
        match prefix {
            Some(prefix) => self.route(prefix),
            None => Err(not_implemented("list without a prefix")),
        }
    }

    /// The store serving both `from` and `to`, which must share a route.
    fn route_pair(&self, from: &Path, to: &Path) -> object_store::Result<Arc<dyn ObjectStore>> {
        let (from_prefix, store) =
            self.route_with_prefix(from)
                .ok_or_else(|| object_store::Error::NotFound {
                    path: from.to_string(),
                    source: format!("no route covers `{from}`").into(),
                })?;
        match self.route_with_prefix(to) {
            Some((to_prefix, _)) if to_prefix == from_prefix => Ok(store),
            _ => Err(not_implemented("copy or rename across routes")),
        }
    }
}

fn not_implemented(operation: &str) -> object_store::Error {
    object_store::Error::NotImplemented {
        operation: operation.to_owned(),
        implementer: "RoutingObjectStore".to_owned(),
    }
}

impl fmt::Debug for RoutingObjectStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner = self.inner.read().unwrap();
        f.debug_struct("RoutingObjectStore")
            .field("bucket", &inner.bucket)
            .field(
                "prefixes",
                &inner
                    .routes
                    .iter()
                    .map(|(p, _)| p.as_ref())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl fmt::Display for RoutingObjectStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.inner.read().unwrap().bucket {
            Some(bucket) => write!(f, "RoutingObjectStore({})", bucket.name),
            None => f.write_str("RoutingObjectStore"),
        }
    }
}

#[async_trait]
impl ObjectStore for RoutingObjectStore {
    async fn put_opts(
        &self,
        location: &Path,
        payload: PutPayload,
        opts: PutOptions,
    ) -> object_store::Result<PutResult> {
        self.route(location)?
            .put_opts(location, payload, opts)
            .await
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        opts: PutMultipartOptions,
    ) -> object_store::Result<Box<dyn MultipartUpload>> {
        self.route(location)?
            .put_multipart_opts(location, opts)
            .await
    }

    async fn get_opts(
        &self,
        location: &Path,
        options: GetOptions,
    ) -> object_store::Result<GetResult> {
        self.route(location)?.get_opts(location, options).await
    }

    async fn get_ranges(
        &self,
        location: &Path,
        ranges: &[Range<u64>],
    ) -> object_store::Result<Vec<Bytes>> {
        self.route(location)?.get_ranges(location, ranges).await
    }

    fn delete_stream(
        &self,
        locations: BoxStream<'static, object_store::Result<Path>>,
    ) -> BoxStream<'static, object_store::Result<Path>> {
        let this = self.clone();
        locations
            .map(move |location| {
                let this = this.clone();
                async move {
                    let location = location?;
                    this.route(&location)?.delete(&location).await?;
                    Ok(location)
                }
            })
            .buffered(10)
            .boxed()
    }

    fn list(&self, prefix: Option<&Path>) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        match self.route_prefix(prefix) {
            Ok(store) => store.list(prefix),
            Err(e) => stream::once(async move { Err(e) }).boxed(),
        }
    }

    fn list_with_offset(
        &self,
        prefix: Option<&Path>,
        offset: &Path,
    ) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        match self.route_prefix(prefix) {
            Ok(store) => store.list_with_offset(prefix, offset),
            Err(e) => stream::once(async move { Err(e) }).boxed(),
        }
    }

    async fn list_with_delimiter(&self, prefix: Option<&Path>) -> object_store::Result<ListResult> {
        self.route_prefix(prefix)?.list_with_delimiter(prefix).await
    }

    async fn copy_opts(
        &self,
        from: &Path,
        to: &Path,
        options: CopyOptions,
    ) -> object_store::Result<()> {
        self.route_pair(from, to)?
            .copy_opts(from, to, options)
            .await
    }

    async fn rename_opts(
        &self,
        from: &Path,
        to: &Path,
        options: RenameOptions,
    ) -> object_store::Result<()> {
        self.route_pair(from, to)?
            .rename_opts(from, to, options)
            .await
    }
}

#[cfg(test)]
mod tests {
    use futures::TryStreamExt;
    use olai_storage_credentials::{AccessLevel, StorageCredentials, VendedCredential};

    use super::*;
    use crate::store::{StoreOptions, build_store};

    #[derive(Debug)]
    struct NeverVends;

    #[async_trait]
    impl crate::CredentialSource for NeverVends {
        async fn fetch(&self) -> Result<VendedCredential> {
            panic!("routing must not vend");
        }
    }

    /// A scoped store for `url` over its own in-memory backend, seeded with one object.
    async fn scoped(url: &str, seeded: &str) -> ScopedStore {
        let seed = VendedCredential::new(
            StorageCredentials::None,
            url.parse().unwrap(),
            AccessLevel::ReadWrite,
        );
        let store = build_store(&seed, Arc::new(NeverVends), &StoreOptions::new())
            .await
            .unwrap();
        store
            .root()
            .put(&Path::from(seeded), PutPayload::from(seeded.to_owned()))
            .await
            .unwrap();
        store
    }

    async fn body(store: &dyn ObjectStore, path: &str) -> object_store::Result<String> {
        let bytes = store.get(&Path::from(path)).await?.bytes().await?;
        Ok(String::from_utf8(bytes.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn routes_to_the_longest_matching_prefix() {
        let router = RoutingObjectStore::new();
        router
            .register(&scoped("memory:///db", "db/outer").await)
            .unwrap();
        router
            .register(&scoped("memory:///db/t1", "db/t1/inner").await)
            .unwrap();

        assert_eq!(body(&router, "db/t1/inner").await.unwrap(), "db/t1/inner");
        assert_eq!(body(&router, "db/outer").await.unwrap(), "db/outer");
        // `db/t10` is not below `db/t1`: it falls through to `db`, which lacks it.
        assert!(matches!(
            body(&router, "db/t10/inner").await,
            Err(object_store::Error::NotFound { .. })
        ));
        assert_eq!(router.prefixes(), [Path::from("db/t1"), Path::from("db")]);
    }

    #[tokio::test]
    async fn sibling_prefix_is_not_routed() {
        let router = RoutingObjectStore::new();
        router
            .register(&scoped("memory:///tab", "tab/a").await)
            .unwrap();
        let err = body(&router, "table2/a").await.unwrap_err();
        assert!(matches!(err, object_store::Error::NotFound { .. }), "{err}");
    }

    #[tokio::test]
    async fn re_registering_a_prefix_replaces_its_store() {
        let router = RoutingObjectStore::new();
        router
            .register(&scoped("memory:///t", "t/old").await)
            .unwrap();
        router
            .register(&scoped("memory:///t", "t/new").await)
            .unwrap();
        assert_eq!(router.prefixes().len(), 1);
        assert!(body(&router, "t/old").await.is_err());
        assert_eq!(body(&router, "t/new").await.unwrap(), "t/new");

        assert!(router.deregister(&Path::from("t")));
        assert!(body(&router, "t/new").await.is_err());
    }

    #[tokio::test]
    async fn listing_needs_a_prefix_and_stays_in_its_route() {
        let router = RoutingObjectStore::new();
        router
            .register(&scoped("memory:///t1", "t1/a").await)
            .unwrap();
        let listed: Vec<_> = router
            .list(Some(&Path::from("t1")))
            .try_collect()
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        let err = router.list(None).try_collect::<Vec<_>>().await.unwrap_err();
        assert!(
            matches!(err, object_store::Error::NotImplemented { .. }),
            "{err}"
        );
    }

    #[tokio::test]
    async fn copies_stay_within_one_route() {
        let router = RoutingObjectStore::new();
        router
            .register(&scoped("memory:///t1", "t1/a").await)
            .unwrap();
        router
            .register(&scoped("memory:///t2", "t2/b").await)
            .unwrap();

        router
            .copy(&Path::from("t1/a"), &Path::from("t1/copy"))
            .await
            .unwrap();
        assert_eq!(body(&router, "t1/copy").await.unwrap(), "t1/a");

        let err = router
            .copy(&Path::from("t1/a"), &Path::from("t2/a"))
            .await
            .unwrap_err();
        assert!(
            matches!(err, object_store::Error::NotImplemented { .. }),
            "{err}"
        );
    }

    #[tokio::test]
    async fn deletes_are_routed() {
        let router = RoutingObjectStore::new();
        let t1 = scoped("memory:///t1", "t1/a").await;
        router.register(&t1).unwrap();
        router.delete(&Path::from("t1/a")).await.unwrap();
        assert!(t1.root().head(&Path::from("t1/a")).await.is_err());
    }

    #[tokio::test]
    async fn routes_must_share_a_bucket() {
        let router = RoutingObjectStore::new();
        let seed = |url: &str| {
            VendedCredential::new(
                StorageCredentials::None,
                url.parse().unwrap(),
                AccessLevel::Read,
            )
        };
        let options = StoreOptions::new();
        #[cfg(feature = "aws")]
        let options = options.without_region_discovery();
        for (url, ok) in [
            ("s3://b1/t1", true),
            ("s3://b1/t2", true),
            ("s3://b2/t1", false),
            ("gs://b1/t1", false),
        ] {
            let Ok(store) = build_store(&seed(url), Arc::new(NeverVends), &options).await else {
                // The backend for this provider is not compiled in.
                continue;
            };
            assert_eq!(router.register(&store).is_ok(), ok, "{url}");
        }
    }
}
