use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::str::FromStr;

/// A type-safe label for resource types.
///
/// Implementations discriminate between different kinds of resources in the store.
/// Typically generated from protobuf `google.api.resource` annotations.
///
/// The label is used as a discriminant in the [`Object<L>`][crate::Object] type
/// and for routing operations to the correct backend or handler.
///
/// # Trait bounds
///
/// The supertraits constrain what a label can do, so the store can treat it as a
/// lightweight, freely-copied key:
///
/// - [`Copy`] + [`Clone`] — labels are passed by value throughout the store API;
///   being trivially copyable keeps those calls cheap and allocation-free.
/// - [`Hash`] + [`Eq`] — labels are used as map keys when routing operations and
///   grouping objects by type.
/// - [`Display`] + [`FromStr`] — labels round-trip to and from their string form
///   for serialization and wire protocols.
/// - [`Debug`] — labels appear in error messages and `tracing` output.
/// - [`Send`] + [`Sync`] + `'static` — labels cross `async` task and thread
///   boundaries and are held inside stores that may outlive any single request.
///
/// # Examples
///
/// A minimal label enum distinguishing two resource types:
///
/// ```
/// use std::fmt;
/// use std::str::FromStr;
///
/// use olai_store::Label;
///
/// #[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
/// enum Kind {
///     Folder,
///     File,
/// }
///
/// impl fmt::Display for Kind {
///     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
///         f.write_str(self.as_str())
///     }
/// }
///
/// impl FromStr for Kind {
///     type Err = String;
///
///     fn from_str(s: &str) -> Result<Self, Self::Err> {
///         match s {
///             "folder" => Ok(Kind::Folder),
///             "file" => Ok(Kind::File),
///             other => Err(format!("unknown kind: {other}")),
///         }
///     }
/// }
///
/// impl Label for Kind {
///     fn as_str(&self) -> &str {
///         match self {
///             Kind::Folder => "folder",
///             Kind::File => "file",
///         }
///     }
/// }
///
/// assert_eq!(Kind::File.as_str(), "file");
/// assert_eq!("folder".parse::<Kind>(), Ok(Kind::Folder));
/// ```
pub trait Label:
    Display + Debug + FromStr + Hash + Eq + Clone + Copy + Send + Sync + 'static
{
    /// Returns the string representation of this label.
    ///
    /// This is the same string accepted by the type's [`FromStr`] implementation,
    /// so `T::from_str(value.as_str())` round-trips back to `value`.
    fn as_str(&self) -> &str;
}
