//! Abstractions of networking so that custom networking implementations can be provided

pub use bytes::Bytes;
pub use http::{self, HeaderMap, Method};
use serde::{
    Serialize,
    ser::{SerializeSeq, SerializeTuple},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::{ops::Deref, path::PathBuf};
pub use url::Url;

/// A type that fetches resources for a Document.
///
/// This may be over the network via http(s), via the filesystem, or some other method.
pub trait NetProvider: Send + Sync + 'static {
    fn fetch(&self, doc_id: usize, request: Request, handler: Box<dyn NetHandler>);

    /// Whether this provider is a no-op (e.g. `DummyNetProvider`) that will never
    /// deliver resources. When true, callers must NOT register resources as
    /// "pending critical" — doing so blocks painting forever, since the
    /// completion callback never fires. Used by integrations that feed a
    /// pre-rendered DOM and perform no sub-fetches (e.g. aginxbrowser).
    fn is_noop(&self) -> bool {
        false
    }
}

/// A type that parses raw bytes from a network request into a Data and then calls
/// the NetCallack with the result.
pub trait NetHandler: Send + Sync + 'static {
    fn bytes(self: Box<Self>, resolved_url: String, bytes: Bytes);

    /// Receive the network provider's authoritative CSSOM access decision.
    /// Providers without this metadata conservatively leave sheets opaque.
    fn bytes_with_metadata(
        self: Box<Self>,
        resolved_url: String,
        bytes: Bytes,
        metadata: ResponseMetadata,
    ) {
        let _ = metadata;
        self.bytes(resolved_url, bytes);
    }

    /// The provider could not deliver a response: a network or policy error,
    /// a failed CORS check, or a request it dropped. Handlers that keep no
    /// pending state may ignore it.
    fn failed(self: Box<Self>) {}
}

/// Authority supplied by the network provider, never by document attributes.
#[derive(Debug, Clone, Copy, Default)]
pub struct ResponseMetadata {
    pub stylesheet_origin_clean: bool,
    pub image_origin_clean: bool,
    /// The response's `Cache-Control` forbids storing it
    /// ([RFC 9111 §5.2.2.5](https://www.rfc-editor.org/rfc/rfc9111#section-5.2.2.5)).
    /// A document keeps such an image only while an element holds it.
    pub no_store: bool,
}

/// A [CORS settings attribute](https://html.spec.whatwg.org/multipage/urls-and-fetching.html#cors-settings-attributes)
/// state. This describes the request's intent, never its authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CorsSettings {
    NoCors,
    Anonymous,
    UseCredentials,
}
impl CorsSettings {
    /// Every state, for consumers that look up an image under each identity.
    pub const ALL: [Self; 3] = [Self::NoCors, Self::Anonymous, Self::UseCredentials];

    /// A missing attribute is No CORS; empty and invalid values are Anonymous.
    pub fn from_attribute(value: Option<&str>) -> Self {
        match value {
            None => Self::NoCors,
            Some(value) if value.eq_ignore_ascii_case("use-credentials") => Self::UseCredentials,
            Some(_) => Self::Anonymous,
        }
    }
}

/// A callback which gets called every time a network request completes
// Q: Should we use std::task::Waker for this?
pub trait NetWaker: Send + Sync + 'static {
    fn wake(&self, client_id: usize);
}

impl<F: Fn(usize) + Send + Sync + 'static> NetWaker for F {
    fn wake(&self, doc_id: usize) {
        self(doc_id)
    }
}

/// The source that initiated a resource load, independent of its URL or body.
/// This is reporting metadata; it grants no network or origin authority.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ResourceInitiator {
    #[default]
    Other,
    Link,
    Img,
    Input,
    Css,
}

#[non_exhaustive]
#[derive(Debug, Clone)]
/// A request type loosely representing <https://fetch.spec.whatwg.org/#requests>
pub struct Request {
    pub url: Url,
    pub method: Method,
    pub content_type: Option<String>,
    pub headers: HeaderMap,
    pub body: Body,
    pub signal: Option<AbortSignal>,
    pub stylesheet: Option<CorsSettings>,
    /// An image request and its CORS setting. Pixel readback authority comes
    /// only from the provider's response metadata.
    pub image: Option<CorsSettings>,
    pub initiator: ResourceInitiator,
}
impl Request {
    /// A get request to the specified Url and an empty body
    pub fn get(url: Url) -> Self {
        Self {
            url,
            method: Method::GET,
            content_type: None,
            headers: HeaderMap::new(),
            body: Body::Empty,
            signal: None,
            stylesheet: None,
            image: None,
            initiator: ResourceInitiator::Other,
        }
    }

    pub fn image(mut self, mode: CorsSettings) -> Self {
        self.image = Some(mode);
        self.initiator = ResourceInitiator::Img;
        self
    }

    pub fn stylesheet(mut self, mode: CorsSettings) -> Self {
        self.stylesheet = Some(mode);
        self.initiator = ResourceInitiator::Link;
        self
    }

    pub fn initiator(mut self, initiator: ResourceInitiator) -> Self {
        self.initiator = initiator;
        self
    }

    pub fn signal(mut self, signal: AbortSignal) -> Self {
        self.signal = Some(signal);
        self
    }
}

#[derive(Debug, Clone)]
pub enum Body {
    Bytes(Bytes),
    Form(FormData),
    Empty,
}

/// A list of form entries used for form submission
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FormData(pub Vec<Entry>);
impl FormData {
    /// Creates a new empty FormData
    pub fn new() -> Self {
        FormData(Vec::new())
    }
}
impl Serialize for FormData {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut seq_serializer = serializer.serialize_seq(Some(self.len()))?;
        for entry in &self.0 {
            seq_serializer.serialize_element(entry)?;
        }
        seq_serializer.end()
    }
}
impl Deref for FormData {
    type Target = Vec<Entry>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A single form entry consisting of a name and value
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub value: EntryValue,
}
impl Serialize for Entry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut serializer = serializer.serialize_tuple(2)?;
        serializer.serialize_element(&self.name)?;
        match &self.value {
            EntryValue::String(s) => serializer.serialize_element(s)?,
            EntryValue::File(p) => serializer
                .serialize_element(p.file_name().and_then(|n| n.to_str()).unwrap_or_default())?,
            EntryValue::EmptyFile => serializer.serialize_element("")?,
            EntryValue::FileContents(file) => serializer.serialize_element(&file.name)?,
        }
        serializer.end()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum EntryValue {
    String(String),
    File(PathBuf),
    EmptyFile,
    /// Host-prepared bytes, never a filesystem path.
    FileContents(FormFile),
}
impl AsRef<str> for EntryValue {
    fn as_ref(&self) -> &str {
        match self {
            EntryValue::String(s) => s,
            EntryValue::File(p) => p.file_name().and_then(|n| n.to_str()).unwrap_or_default(),
            EntryValue::EmptyFile => "",
            EntryValue::FileContents(file) => &file.name,
        }
    }
}

impl From<&str> for EntryValue {
    fn from(value: &str) -> Self {
        EntryValue::String(value.to_string())
    }
}
impl From<PathBuf> for EntryValue {
    fn from(value: PathBuf) -> Self {
        EntryValue::File(value)
    }
}

/// A default noop NetProvider
#[derive(Default)]
pub struct DummyNetProvider;
impl NetProvider for DummyNetProvider {
    fn fetch(&self, _doc_id: usize, _request: Request, _handler: Box<dyn NetHandler>) {}
    fn is_noop(&self) -> bool {
        true
    }
}

/// The AbortController interface represents a controller object that
/// allows you to abort one or more Web requests as and when desired.
///
/// <https://developer.mozilla.org/en-US/docs/Web/API/AbortController>
#[derive(Debug, Default)]
pub struct AbortController {
    pub signal: AbortSignal,
}

impl AbortController {
    /// The abort() method of the AbortController interface aborts
    /// an asynchronous operation before it has completed.
    /// This is able to abort fetch requests.
    ///
    /// <https://developer.mozilla.org/en-US/docs/Web/API/AbortController/abort>
    pub fn abort(self) {
        self.signal.0.store(true, Ordering::SeqCst);
    }
}

/// The AbortSignal interface represents a signal object that allows you to
/// communicate with an asynchronous operation (such as a fetch request) and
/// abort it if required via an AbortController object.
///
/// <https://developer.mozilla.org/en-US/docs/Web/API/AbortSignal>
#[derive(Debug, Default, Clone)]
pub struct AbortSignal(Arc<AtomicBool>);

impl AbortSignal {
    /// The aborted read-only property returns a value that indicates whether
    /// the asynchronous operations the signal is communicating with are
    /// aborted (true) or not (false).
    ///
    /// <https://developer.mozilla.org/en-US/docs/Web/API/AbortSignal/aborted>
    pub fn aborted(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Immutable file snapshot supplied by an embedder after its own authorization
/// and asynchronous I/O. The name is presentation metadata, never a read target.
#[derive(Clone, PartialEq)]
pub struct FormFile {
    /// File API millisecond timestamp retained from its authorized source.
    pub last_modified: f64,
    pub name: String,
    pub content_type: String,
    pub bytes: std::sync::Arc<[u8]>,
}
impl std::fmt::Debug for FormFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FormFile")
            .field("size", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

/// Entry materialization exceeded its aggregate byte or count budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormDataLimit;
impl std::fmt::Display for FormDataLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Form entry data limit exceeded")
    }
}
impl std::error::Error for FormDataLimit {}

#[cfg(test)]
mod initiator_tests {
    use super::*;
    #[test]
    fn initiating_source_is_independent_of_url_extension_and_response_kind() {
        let url = Url::parse("https://page.test/no-extension").unwrap();
        assert_eq!(
            Request::get(url.clone()).initiator,
            ResourceInitiator::Other
        );
        assert_eq!(
            Request::get(url.clone())
                .image(CorsSettings::NoCors)
                .initiator,
            ResourceInitiator::Img
        );
        assert_eq!(
            Request::get(url.clone())
                .image(CorsSettings::NoCors)
                .initiator(ResourceInitiator::Input)
                .initiator,
            ResourceInitiator::Input
        );
        assert_eq!(
            Request::get(url.clone())
                .image(CorsSettings::NoCors)
                .initiator(ResourceInitiator::Css)
                .initiator,
            ResourceInitiator::Css
        );
        assert_eq!(
            Request::get(url.clone())
                .stylesheet(CorsSettings::NoCors)
                .initiator,
            ResourceInitiator::Link
        );
        assert_eq!(
            Request::get(url)
                .stylesheet(CorsSettings::NoCors)
                .initiator(ResourceInitiator::Css)
                .initiator,
            ResourceInitiator::Css
        );
    }
}

#[cfg(test)]
mod cors_settings_tests {
    use super::*;
    #[test]
    fn cors_settings_follow_the_html_attribute_states() {
        for (value, expected) in [
            (None, CorsSettings::NoCors),
            (Some(""), CorsSettings::Anonymous),
            (Some("anonymous"), CorsSettings::Anonymous),
            (Some("bogus"), CorsSettings::Anonymous),
            (Some("use-credentials"), CorsSettings::UseCredentials),
            (Some("Use-Credentials"), CorsSettings::UseCredentials),
        ] {
            assert_eq!(CorsSettings::from_attribute(value), expected, "{value:?}");
        }
        let url = Url::parse("https://page.test/a.png").unwrap();
        let request = Request::get(url).image(CorsSettings::Anonymous);
        assert_eq!(request.image, Some(CorsSettings::Anonymous));
        assert_eq!(request.initiator, ResourceInitiator::Img);
    }
}
