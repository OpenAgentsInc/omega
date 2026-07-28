//! Gated media support for the public Nostr channel timeline.
//!
//! A signed event can describe a remote file. It cannot make Omega fetch or
//! display that file. The view must first move a media state from
//! [`PublicChannelMediaState::Gated`] to
//! [`PublicChannelMediaState::Loading`] in response to a reader action. Only
//! then can it call [`fetch_public_channel_media`].
//!
//! The fetch path sends no credential or referrer header. It validates each
//! redirect, bounds the bytes that it reads, calculates SHA-256 before it
//! decodes or stores the file, and refuses a supplied digest that does not
//! match. This module returns intents for native open and save actions. It does
//! not execute those actions.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    io::{Cursor, Write as _},
    path::{Path, PathBuf},
    sync::Arc,
};

use futures::{AsyncReadExt as _, Future, future::BoxFuture};
use gpui::RenderImage;
use http_client::{
    AsyncBody, HttpClient, HttpRequestExt as _, Method, RedirectPolicy, Request, StatusCode,
    http::header,
};
use image::{DynamicImage, ImageDecoder as _};
use nostr::Event;
use sha2::{Digest as _, Sha256};
use url::{Host, Url};

/// The maximum number of HTTP redirect responses for one reader action.
pub const MAX_MEDIA_REDIRECTS: usize = 5;

/// A strict image edge limit. The allocation limit below also rejects images
/// whose two otherwise-valid edges make an unsafe pixel count.
pub const MAX_MEDIA_IMAGE_EDGE: u32 = 8_192;

/// The maximum allocation requested from an image decoder.
pub const MAX_MEDIA_IMAGE_DECODE_BYTES: u64 = 128 * 1024 * 1024;

/// Metadata for one attachment that the signed event binds.
#[derive(Clone, Debug, PartialEq)]
pub struct PublicChannelAttachment {
    pub url: String,
    pub mime_type: String,
    pub digest: Option<String>,
    pub size: Option<usize>,
    pub alt: Option<String>,
    pub blurhash: Option<String>,
    pub dimensions: Option<String>,
    pub duration_seconds: Option<f64>,
    pub thumbnail_url: Option<String>,
    pub waveform: Vec<f64>,
}

/// A projection seam for timeline-owned media facts.
///
/// The timeline implements this trait for its `MediaFact`. The view can then
/// ask [`PublicChannelAttachment::try_from_media_fact`] to validate the signed
/// facts without parsing the Nostr event again.
pub trait PublicChannelMediaFact {
    fn url(&self) -> &str;
    fn mime_type(&self) -> &str;
    fn digest(&self) -> Option<&str>;
    fn size(&self) -> Option<usize>;
    fn alt(&self) -> Option<&str>;
    fn blurhash(&self) -> Option<&str>;
    fn dimensions(&self) -> Option<&str>;
    fn duration_seconds(&self) -> Option<&str>;
    fn thumbnail_url(&self) -> Option<&str>;
    fn waveform(&self) -> &[String];
}

impl PublicChannelAttachment {
    /// Validate one timeline projection before it becomes a load control.
    pub fn try_from_media_fact(
        fact: &impl PublicChannelMediaFact,
        max_bytes: usize,
    ) -> Result<Self, PublicChannelMediaUnavailableReason> {
        if max_bytes == 0
            || validate_media_url(fact.url(), None).is_err()
            || !is_safe_media_mime(&fact.mime_type().to_ascii_lowercase())
            || fact.size().is_some_and(|size| size > max_bytes)
            || fact
                .digest()
                .is_some_and(|value| !is_lower_hex_digest(value))
        {
            return Err(PublicChannelMediaUnavailableReason::UnsafeMetadata);
        }
        let duration_seconds = match fact.duration_seconds() {
            Some(value) => Some(
                value
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite() && *value >= 0.0)
                    .ok_or(PublicChannelMediaUnavailableReason::UnsafeMetadata)?,
            ),
            None => None,
        };
        let waveform = fact
            .waveform()
            .iter()
            .map(|value| value.parse::<f64>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| PublicChannelMediaUnavailableReason::UnsafeMetadata)?;
        if waveform.len() > 256 || waveform.iter().any(|value| !value.is_finite()) {
            return Err(PublicChannelMediaUnavailableReason::UnsafeMetadata);
        }
        Ok(Self {
            url: fact.url().to_owned(),
            mime_type: fact.mime_type().to_ascii_lowercase(),
            digest: fact.digest().map(str::to_owned),
            size: fact.size(),
            alt: fact.alt().map(str::to_owned),
            blurhash: fact.blurhash().map(str::to_owned),
            dimensions: fact.dimensions().map(str::to_owned),
            duration_seconds,
            thumbnail_url: fact.thumbnail_url().map(str::to_owned),
            waveform,
        })
    }
}

/// The key for state that belongs to one attachment in one channel event.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PublicChannelMediaKey {
    pub channel_id: String,
    pub event_id: String,
    pub attachment_index: usize,
}

impl PublicChannelMediaKey {
    pub fn new(
        channel_id: impl Into<String>,
        event_id: impl Into<String>,
        attachment_index: usize,
    ) -> Self {
        Self {
            channel_id: channel_id.into(),
            event_id: event_id.into(),
            attachment_index,
        }
    }
}

/// A compact media state for view matching and accessibility copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicChannelMediaLifecycle {
    Gated,
    Loading,
    Verified,
    Mismatch,
    Unavailable,
}

impl PublicChannelMediaLifecycle {
    pub fn label(self) -> &'static str {
        match self {
            Self::Gated => "Reader action required",
            Self::Loading => "Loading media",
            Self::Verified => "Verified media",
            Self::Mismatch => "Media digest mismatch",
            Self::Unavailable => "Media unavailable",
        }
    }
}

/// A bounded public reason for a failed media load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicChannelMediaUnavailableReason {
    UnsafeMetadata,
    UnsafeUrl,
    Network,
    HttpStatus,
    MissingRedirectLocation,
    TooManyRedirects,
    TooLarge,
    MimeMismatch,
    ImageDecode,
    TemporaryFile,
}

impl PublicChannelMediaUnavailableReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::UnsafeMetadata => "The signed media metadata is not safe to use.",
            Self::UnsafeUrl => "The media URL is not safe to fetch.",
            Self::Network => "The media could not be fetched.",
            Self::HttpStatus => "The media server refused the request.",
            Self::MissingRedirectLocation => "The media redirect has no safe destination.",
            Self::TooManyRedirects => "The media used too many redirects.",
            Self::TooLarge => "The media is larger than this channel permits.",
            Self::MimeMismatch => "The returned media type does not match the signed event.",
            Self::ImageDecode => "The verified image cannot be decoded safely.",
            Self::TemporaryFile => "Omega could not prepare the verified media file.",
        }
    }
}

/// The state sequence that the selected-channel view renders.
#[derive(Clone, Default)]
pub enum PublicChannelMediaState {
    #[default]
    Gated,
    Loading,
    Verified(Arc<VerifiedPublicChannelMedia>),
    Mismatch {
        expected: String,
        actual: String,
    },
    Unavailable {
        reason: PublicChannelMediaUnavailableReason,
    },
}

impl fmt::Debug for PublicChannelMediaState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gated => formatter.write_str("Gated"),
            Self::Loading => formatter.write_str("Loading"),
            Self::Verified(media) => formatter
                .debug_tuple("Verified")
                .field(&media.safe_facts())
                .finish(),
            Self::Mismatch { expected, actual } => formatter
                .debug_struct("Mismatch")
                .field("expected", expected)
                .field("actual", actual)
                .finish(),
            Self::Unavailable { reason } => formatter
                .debug_struct("Unavailable")
                .field("reason", reason)
                .finish(),
        }
    }
}

impl PublicChannelMediaState {
    pub fn lifecycle(&self) -> PublicChannelMediaLifecycle {
        match self {
            Self::Gated => PublicChannelMediaLifecycle::Gated,
            Self::Loading => PublicChannelMediaLifecycle::Loading,
            Self::Verified(_) => PublicChannelMediaLifecycle::Verified,
            Self::Mismatch { .. } => PublicChannelMediaLifecycle::Mismatch,
            Self::Unavailable { .. } => PublicChannelMediaLifecycle::Unavailable,
        }
    }

    /// Start a load after an explicit reader action.
    ///
    /// A second click or a programmatic call while the state is not gated does
    /// not start another request.
    pub fn begin_load(&mut self) -> bool {
        if matches!(self, Self::Gated) {
            *self = Self::Loading;
            true
        } else {
            false
        }
    }
}

/// How the view can present bytes after all verification gates pass.
#[derive(Clone)]
pub enum PublicChannelMediaPresentation {
    InlineImage(Arc<RenderImage>),
    OpenWithSystem,
    SaveOnly,
}

impl fmt::Debug for PublicChannelMediaPresentation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InlineImage(image) => formatter
                .debug_tuple("InlineImage")
                .field(&image.size(0))
                .finish(),
            Self::OpenWithSystem => formatter.write_str("OpenWithSystem"),
            Self::SaveOnly => formatter.write_str("SaveOnly"),
        }
    }
}

impl PublicChannelMediaPresentation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::InlineImage(_) => "Verified image",
            Self::OpenWithSystem => "Open verified media",
            Self::SaveOnly => "Save verified file",
        }
    }
}

/// A side effect that the view can execute after a second reader action.
///
/// Tests can inspect this value without calling the operating system.
#[derive(Clone, Debug)]
pub enum PublicChannelMediaIntent {
    InlineImage(Arc<RenderImage>),
    OpenWithSystem {
        path: PathBuf,
    },
    SaveAs {
        source: PathBuf,
        suggested_name: String,
    },
}

/// Public-safe facts for the event detail inspector.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedPublicChannelMediaFacts {
    pub actual_digest: String,
    pub byte_len: usize,
    pub mime_type: String,
    pub presentation: &'static str,
}

/// Media that passed the URL, byte, MIME, and digest gates.
///
/// The private temporary-file guard keeps the path valid until every clone of
/// this value is dropped.
pub struct VerifiedPublicChannelMedia {
    pub actual_digest: String,
    pub byte_len: usize,
    pub mime_type: String,
    pub presentation: PublicChannelMediaPresentation,
    artifact: Arc<VerifiedTempArtifact>,
}

impl fmt::Debug for VerifiedPublicChannelMedia {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedPublicChannelMedia")
            .field("facts", &self.safe_facts())
            .finish_non_exhaustive()
    }
}

impl VerifiedPublicChannelMedia {
    pub fn safe_facts(&self) -> VerifiedPublicChannelMediaFacts {
        VerifiedPublicChannelMediaFacts {
            actual_digest: self.actual_digest.clone(),
            byte_len: self.byte_len,
            mime_type: self.mime_type.clone(),
            presentation: self.presentation.label(),
        }
    }

    pub fn intent(&self) -> PublicChannelMediaIntent {
        match &self.presentation {
            PublicChannelMediaPresentation::InlineImage(image) => {
                PublicChannelMediaIntent::InlineImage(image.clone())
            }
            PublicChannelMediaPresentation::OpenWithSystem => {
                PublicChannelMediaIntent::OpenWithSystem {
                    path: self.artifact.path().to_path_buf(),
                }
            }
            PublicChannelMediaPresentation::SaveOnly => self.save_intent(),
        }
    }

    pub fn save_intent(&self) -> PublicChannelMediaIntent {
        PublicChannelMediaIntent::SaveAs {
            source: self.artifact.path().to_path_buf(),
            suggested_name: format!("nostr-media.{}", mime_extension(&self.mime_type)),
        }
    }

    pub fn temporary_path(&self) -> &Path {
        self.artifact.path()
    }
}

struct VerifiedTempArtifact {
    file: tempfile::NamedTempFile,
}

impl VerifiedTempArtifact {
    fn path(&self) -> &Path {
        self.file.path()
    }
}

#[cfg(test)]
pub(crate) fn verified_media_state_for_test() -> PublicChannelMediaState {
    let artifact = tempfile::NamedTempFile::new().expect("test media artifact");
    PublicChannelMediaState::Verified(Arc::new(VerifiedPublicChannelMedia {
        actual_digest: "00".repeat(32),
        byte_len: 0,
        mime_type: "text/plain".to_string(),
        presentation: PublicChannelMediaPresentation::SaveOnly,
        artifact: Arc::new(VerifiedTempArtifact { file: artifact }),
    }))
}

/// Parse the safe inline `imeta` records in a signed event.
///
/// An attachment URL must also occur in the event content. This prevents a tag
/// that is not part of the visible message from causing an unexpected media
/// control. Invalid entries are ignored. The signed message remains valid.
pub fn parse_inline_attachments(
    event: &Event,
    max_count: usize,
    max_bytes: usize,
) -> Vec<PublicChannelAttachment> {
    if max_count == 0 || max_bytes == 0 {
        return Vec::new();
    }

    let content_urls: BTreeSet<String> = event
        .content
        .split_whitespace()
        .map(|value| value.trim_end_matches([')', ',', '.', ';', '!', '?']))
        .filter(|value| validate_media_url(value, None).is_ok())
        .map(str::to_owned)
        .collect();

    event
        .tags
        .iter()
        .filter_map(|tag| {
            let fields = tag.as_slice();
            if fields.first().map(String::as_str) != Some("imeta") {
                return None;
            }
            let fields: BTreeMap<&str, &str> = fields[1..]
                .iter()
                .filter_map(|field| field.split_once(' '))
                .collect();
            let url = *fields.get("url")?;
            let mime_type = fields.get("m")?.to_ascii_lowercase();
            if !content_urls.contains(url) || !is_safe_media_mime(&mime_type) {
                return None;
            }
            let size = fields
                .get("size")
                .and_then(|value| value.parse::<usize>().ok());
            if fields.contains_key("size") && size.is_none_or(|size| size > max_bytes) {
                return None;
            }
            let digest = fields
                .get("x")
                .filter(|value| is_lower_hex_digest(value))
                .map(|value| (*value).to_owned());
            let duration_seconds = fields
                .get("duration")
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|value| value.is_finite() && *value >= 0.0);
            let waveform = fields
                .get("waveform")
                .map(|value| {
                    value
                        .split_whitespace()
                        .filter_map(|sample| sample.parse::<f64>().ok())
                        .filter(|sample| sample.is_finite())
                        .take(256)
                        .collect()
                })
                .unwrap_or_default();
            Some(PublicChannelAttachment {
                url: url.to_owned(),
                mime_type,
                digest,
                size,
                alt: fields.get("alt").map(|value| (*value).to_owned()),
                blurhash: fields.get("blurhash").map(|value| (*value).to_owned()),
                dimensions: fields.get("dim").map(|value| (*value).to_owned()),
                duration_seconds,
                thumbnail_url: fields.get("thumb").map(|value| (*value).to_owned()),
                waveform,
            })
        })
        .take(max_count)
        .collect()
}

/// Fetch and verify one attachment after the view accepted a reader action.
pub async fn fetch_public_channel_media(
    http_client: Arc<dyn HttpClient>,
    attachment: PublicChannelAttachment,
    max_bytes: usize,
) -> PublicChannelMediaState {
    fetch_public_channel_media_with_policy(
        http_client,
        attachment,
        max_bytes,
        validate_public_media_host,
    )
    .await
}

async fn fetch_public_channel_media_with_policy<F, Fut>(
    http_client: Arc<dyn HttpClient>,
    attachment: PublicChannelAttachment,
    max_bytes: usize,
    validate_host: F,
) -> PublicChannelMediaState
where
    F: Fn(Url) -> Fut,
    Fut: Future<Output = Result<(), PublicChannelMediaUnavailableReason>>,
{
    let mut current = match validate_media_url(&attachment.url, None) {
        Ok(url) => url,
        Err(reason) => return PublicChannelMediaState::Unavailable { reason },
    };
    let mut redirects = 0;

    let bytes = loop {
        if let Err(reason) = validate_host(current.clone()).await {
            return PublicChannelMediaState::Unavailable { reason };
        }
        let request = match Request::builder()
            .method(Method::GET)
            .uri(current.as_str())
            .header(header::ACCEPT, attachment.mime_type.as_str())
            .follow_redirects(RedirectPolicy::NoFollow)
            .body(AsyncBody::empty())
        {
            Ok(request) => request,
            Err(_) => {
                return PublicChannelMediaState::Unavailable {
                    reason: PublicChannelMediaUnavailableReason::UnsafeUrl,
                };
            }
        };
        let response = match http_client.send(request).await {
            Ok(response) => response,
            Err(_) => {
                return PublicChannelMediaState::Unavailable {
                    reason: PublicChannelMediaUnavailableReason::Network,
                };
            }
        };
        if is_redirect(response.status()) {
            if redirects >= MAX_MEDIA_REDIRECTS {
                return PublicChannelMediaState::Unavailable {
                    reason: PublicChannelMediaUnavailableReason::TooManyRedirects,
                };
            }
            let Some(location) = response
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok())
            else {
                return PublicChannelMediaState::Unavailable {
                    reason: PublicChannelMediaUnavailableReason::MissingRedirectLocation,
                };
            };
            current = match validate_media_url(location, Some(&current)) {
                Ok(url) => url,
                Err(reason) => return PublicChannelMediaState::Unavailable { reason },
            };
            redirects += 1;
            continue;
        }
        if !response.status().is_success() {
            return PublicChannelMediaState::Unavailable {
                reason: PublicChannelMediaUnavailableReason::HttpStatus,
            };
        }
        if response
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|length| length > max_bytes as u64)
        {
            return PublicChannelMediaState::Unavailable {
                reason: PublicChannelMediaUnavailableReason::TooLarge,
            };
        }
        if response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| !mime_matches(value, &attachment.mime_type))
        {
            return PublicChannelMediaState::Unavailable {
                reason: PublicChannelMediaUnavailableReason::MimeMismatch,
            };
        }
        let mut body = response.into_body().take(
            u64::try_from(max_bytes)
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        );
        let mut bytes = Vec::with_capacity(max_bytes.min(64 * 1024));
        if body.read_to_end(&mut bytes).await.is_err() {
            return PublicChannelMediaState::Unavailable {
                reason: PublicChannelMediaUnavailableReason::Network,
            };
        }
        if bytes.len() > max_bytes {
            return PublicChannelMediaState::Unavailable {
                reason: PublicChannelMediaUnavailableReason::TooLarge,
            };
        }
        break bytes;
    };

    let actual_digest = format!("{:x}", Sha256::digest(&bytes));
    if let Some(expected) = attachment.digest.as_ref()
        && expected != &actual_digest
    {
        return PublicChannelMediaState::Mismatch {
            expected: expected.clone(),
            actual: actual_digest,
        };
    }

    let byte_len = bytes.len();
    let presentation = match presentation_for(&attachment.mime_type, bytes.clone()).await {
        Ok(presentation) => presentation,
        Err(reason) => return PublicChannelMediaState::Unavailable { reason },
    };
    let artifact = match create_verified_temp_file(&attachment.mime_type, bytes).await {
        Ok(artifact) => artifact,
        Err(reason) => return PublicChannelMediaState::Unavailable { reason },
    };
    PublicChannelMediaState::Verified(Arc::new(VerifiedPublicChannelMedia {
        actual_digest,
        byte_len,
        mime_type: attachment.mime_type,
        presentation,
        artifact: Arc::new(artifact),
    }))
}

fn validate_media_url(
    value: &str,
    previous: Option<&Url>,
) -> Result<Url, PublicChannelMediaUnavailableReason> {
    let mut url = previous
        .map_or_else(|| Url::parse(value), |base| base.join(value))
        .map_err(|_| PublicChannelMediaUnavailableReason::UnsafeUrl)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || matches!(url.host(), Some(Host::Domain(host)) if host.eq_ignore_ascii_case("localhost")
            || host.to_ascii_lowercase().ends_with(".localhost"))
        || matches!(url.host(), Some(Host::Ipv4(_)) | Some(Host::Ipv6(_)))
        || previous.is_some_and(|previous| previous.scheme() == "https" && url.scheme() == "http")
    {
        return Err(PublicChannelMediaUnavailableReason::UnsafeUrl);
    }
    url.set_fragment(None);
    Ok(url)
}

fn validate_public_media_host(
    url: Url,
) -> BoxFuture<'static, Result<(), PublicChannelMediaUnavailableReason>> {
    Box::pin(async move {
        let host = url
            .host_str()
            .ok_or(PublicChannelMediaUnavailableReason::UnsafeUrl)?
            .to_owned();
        let port = url
            .port_or_known_default()
            .ok_or(PublicChannelMediaUnavailableReason::UnsafeUrl)?;
        smol::unblock(move || {
            http_proxy::PinnedHost::resolve(&host, port)
                .map(|_| ())
                .map_err(|_| PublicChannelMediaUnavailableReason::UnsafeUrl)
        })
        .await
    })
}

fn is_redirect(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    )
}

fn mime_matches(response: &str, signed: &str) -> bool {
    response
        .split(';')
        .next()
        .map(str::trim)
        .is_some_and(|response| response.eq_ignore_ascii_case(signed))
}

fn is_safe_media_mime(value: &str) -> bool {
    matches!(
        value,
        "image/avif"
            | "image/gif"
            | "image/jpeg"
            | "image/png"
            | "image/webp"
            | "audio/aac"
            | "audio/flac"
            | "audio/mpeg"
            | "audio/mp4"
            | "audio/ogg"
            | "audio/wav"
            | "audio/webm"
            | "video/mp4"
            | "video/ogg"
            | "video/webm"
            | "application/json"
            | "application/pdf"
            | "text/csv"
            | "text/plain"
    )
}

fn is_lower_hex_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
}

async fn presentation_for(
    mime_type: &str,
    bytes: Vec<u8>,
) -> Result<PublicChannelMediaPresentation, PublicChannelMediaUnavailableReason> {
    let format = match mime_type {
        "image/gif" => image::ImageFormat::Gif,
        "image/jpeg" => image::ImageFormat::Jpeg,
        "image/png" => image::ImageFormat::Png,
        "image/webp" => image::ImageFormat::WebP,
        "image/avif" => return Ok(PublicChannelMediaPresentation::OpenWithSystem),
        value if value.starts_with("audio/") || value.starts_with("video/") => {
            return Ok(PublicChannelMediaPresentation::OpenWithSystem);
        }
        _ => return Ok(PublicChannelMediaPresentation::SaveOnly),
    };
    smol::unblock(move || decode_static_image(bytes, format))
        .await
        .map(PublicChannelMediaPresentation::InlineImage)
}

fn decode_static_image(
    bytes: Vec<u8>,
    format: image::ImageFormat,
) -> Result<Arc<RenderImage>, PublicChannelMediaUnavailableReason> {
    let mut decoder = image::ImageReader::with_format(Cursor::new(bytes), format)
        .into_decoder()
        .map_err(|_| PublicChannelMediaUnavailableReason::ImageDecode)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_MEDIA_IMAGE_EDGE);
    limits.max_image_height = Some(MAX_MEDIA_IMAGE_EDGE);
    limits.max_alloc = Some(MAX_MEDIA_IMAGE_DECODE_BYTES);
    decoder
        .set_limits(limits)
        .map_err(|_| PublicChannelMediaUnavailableReason::ImageDecode)?;
    let (width, height) = decoder.dimensions();
    let decoded_bytes = u64::from(width)
        .saturating_mul(u64::from(height))
        .saturating_mul(4);
    if width > MAX_MEDIA_IMAGE_EDGE
        || height > MAX_MEDIA_IMAGE_EDGE
        || decoded_bytes > MAX_MEDIA_IMAGE_DECODE_BYTES
    {
        return Err(PublicChannelMediaUnavailableReason::ImageDecode);
    }
    let orientation = decoder
        .orientation()
        .map_err(|_| PublicChannelMediaUnavailableReason::ImageDecode)?;
    let mut decoded = DynamicImage::from_decoder(decoder)
        .map_err(|_| PublicChannelMediaUnavailableReason::ImageDecode)?;
    decoded.apply_orientation(orientation);
    let mut buffer = decoded.into_rgba8();
    for pixel in buffer.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Ok(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

async fn create_verified_temp_file(
    mime_type: &str,
    bytes: Vec<u8>,
) -> Result<VerifiedTempArtifact, PublicChannelMediaUnavailableReason> {
    let suffix = format!(".{}", mime_extension(mime_type));
    smol::unblock(move || {
        let mut file = tempfile::Builder::new()
            .prefix("omega-public-media-")
            .suffix(&suffix)
            .tempfile_in(paths::temp_dir())
            .map_err(|_| PublicChannelMediaUnavailableReason::TemporaryFile)?;
        file.write_all(&bytes)
            .and_then(|_| file.as_file_mut().flush())
            .map_err(|_| PublicChannelMediaUnavailableReason::TemporaryFile)?;
        Ok(VerifiedTempArtifact { file })
    })
    .await
}

fn mime_extension(mime_type: &str) -> &'static str {
    match mime_type {
        "image/avif" => "avif",
        "image/gif" => "gif",
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "audio/aac" => "aac",
        "audio/flac" => "flac",
        "audio/mpeg" => "mp3",
        "audio/mp4" => "m4a",
        "audio/ogg" => "ogg",
        "audio/wav" => "wav",
        "audio/webm" => "webm",
        "video/mp4" => "mp4",
        "video/ogg" => "ogv",
        "video/webm" => "webm",
        "application/json" => "json",
        "application/pdf" => "pdf",
        "text/csv" => "csv",
        "text/plain" => "txt",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use base64::Engine as _;
    use http_client::{FakeHttpClient, Response};
    use nostr::JsonUtil as _;

    use super::*;

    const MEDIA_EVENT: &str = r#"{
      "content":"Media https://cdn.example/fixture.png",
      "created_at":1700000020,
      "kind":9,
      "tags":[
        ["h","openagents-public"],
        ["imeta","url https://cdn.example/fixture.png","m image/png","x cc55a82c33dc2c4d4a499dac1eda25cb334f18619cb966165070c6ac49573066","size 27","alt Agent Chat fixture","dim 16x16","duration 2.5","waveform 0 0.5 bad 1"]
      ],
      "pubkey":"4d4b6cd1361032ca9bd2aeb9d900aa4d45d9ead80ac9423374c451a7254d0766",
      "id":"57caecba0838abaee23d6a6bb0e70c842ddf289640dc7591f32786e25e8825e4",
      "sig":"90e3fea8e34c83bfd7af8e120bf7f7fcd16a17af34b3965934514187425e74dbf33eaa995252685e50d2c57497aba44e19b708daf44d37e9f48e3d0496391dc9"
    }"#;

    const FIXTURE_PAYLOAD_BASE64: &str = "YWdlbnQtY2hhdC1tZWRpYS1maXh0dXJlLXYx";
    const FIXTURE_DIGEST: &str = "cc55a82c33dc2c4d4a499dac1eda25cb334f18619cb966165070c6ac49573066";

    #[derive(Default)]
    struct TestMediaFact {
        url: String,
        mime_type: String,
        digest: Option<String>,
        size: Option<usize>,
        alt: Option<String>,
        blurhash: Option<String>,
        dimensions: Option<String>,
        duration_seconds: Option<String>,
        thumbnail_url: Option<String>,
        waveform: Vec<String>,
    }

    impl PublicChannelMediaFact for TestMediaFact {
        fn url(&self) -> &str {
            &self.url
        }

        fn mime_type(&self) -> &str {
            &self.mime_type
        }

        fn digest(&self) -> Option<&str> {
            self.digest.as_deref()
        }

        fn size(&self) -> Option<usize> {
            self.size
        }

        fn alt(&self) -> Option<&str> {
            self.alt.as_deref()
        }

        fn blurhash(&self) -> Option<&str> {
            self.blurhash.as_deref()
        }

        fn dimensions(&self) -> Option<&str> {
            self.dimensions.as_deref()
        }

        fn duration_seconds(&self) -> Option<&str> {
            self.duration_seconds.as_deref()
        }

        fn thumbnail_url(&self) -> Option<&str> {
            self.thumbnail_url.as_deref()
        }

        fn waveform(&self) -> &[String] {
            &self.waveform
        }
    }

    fn fixture_attachment() -> PublicChannelAttachment {
        PublicChannelAttachment {
            url: "https://media.example/file.txt".to_owned(),
            mime_type: "text/plain".to_owned(),
            digest: Some(FIXTURE_DIGEST.to_owned()),
            size: Some(27),
            alt: Some("fixture".to_owned()),
            blurhash: None,
            dimensions: None,
            duration_seconds: None,
            thumbnail_url: None,
            waveform: Vec::new(),
        }
    }

    async fn allow_test_host(_url: Url) -> Result<(), PublicChannelMediaUnavailableReason> {
        Ok(())
    }

    fn response(
        status: StatusCode,
        mime_type: Option<&str>,
        body: impl Into<AsyncBody>,
    ) -> Response<AsyncBody> {
        let mut response = Response::builder().status(status);
        if let Some(mime_type) = mime_type {
            response = response.header(header::CONTENT_TYPE, mime_type);
        }
        response.body(body.into()).expect("test response")
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::new_rgba8(width, height)
            .write_to(&mut encoded, image::ImageFormat::Png)
            .expect("encode PNG");
        encoded.into_inner()
    }

    #[test]
    fn attachment_parser_matches_the_shared_agent_chat_contract() {
        let event = Event::from_json(MEDIA_EVENT).expect("the shared signed fixture event");
        let attachments = parse_inline_attachments(&event, 4, 25 * 1024 * 1024);
        assert_eq!(attachments.len(), 1);
        assert_eq!(
            attachments[0],
            PublicChannelAttachment {
                url: "https://cdn.example/fixture.png".to_owned(),
                mime_type: "image/png".to_owned(),
                digest: Some(FIXTURE_DIGEST.to_owned()),
                size: Some(27),
                alt: Some("Agent Chat fixture".to_owned()),
                blurhash: None,
                dimensions: Some("16x16".to_owned()),
                duration_seconds: Some(2.5),
                thumbnail_url: None,
                waveform: vec![0.0, 0.5, 1.0],
            }
        );
    }

    #[test]
    fn attachment_parser_refuses_hidden_unsafe_oversized_and_excess_entries() {
        let mut event = Event::from_json(MEDIA_EVENT).expect("fixture");
        event.tags = nostr::Tags::parse(vec![
            vec!["imeta", "url https://hidden.example/a.png", "m image/png"],
            vec![
                "imeta",
                "url https://cdn.example/fixture.png",
                "m application/x-executable",
            ],
            vec![
                "imeta",
                "url https://cdn.example/fixture.png",
                "m image/png",
                "size 100",
            ],
            vec![
                "imeta",
                "url https://cdn.example/fixture.png",
                "m image/png",
            ],
            vec![
                "imeta",
                "url https://cdn.example/fixture.png",
                "m image/png",
            ],
        ])
        .expect("test tags");
        let attachments = parse_inline_attachments(&event, 1, 99);
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].mime_type, "image/png");
    }

    #[test]
    fn projected_media_facts_are_revalidated_once() {
        let fact = TestMediaFact {
            url: "https://media.example/file.webm".to_owned(),
            mime_type: "VIDEO/WEBM".to_owned(),
            digest: Some(FIXTURE_DIGEST.to_owned()),
            size: Some(27),
            duration_seconds: Some("2.5".to_owned()),
            waveform: vec!["0".to_owned(), "0.5".to_owned(), "1".to_owned()],
            ..Default::default()
        };
        let attachment =
            PublicChannelAttachment::try_from_media_fact(&fact, 100).expect("safe fact");
        assert_eq!(attachment.mime_type, "video/webm");
        assert_eq!(attachment.duration_seconds, Some(2.5));
        assert_eq!(attachment.waveform, vec![0.0, 0.5, 1.0]);

        for fact in [
            TestMediaFact {
                url: "file:///tmp/a".to_owned(),
                mime_type: "video/webm".to_owned(),
                ..Default::default()
            },
            TestMediaFact {
                url: "https://media.example/a".to_owned(),
                mime_type: "application/x-executable".to_owned(),
                ..Default::default()
            },
            TestMediaFact {
                url: "https://media.example/a".to_owned(),
                mime_type: "video/webm".to_owned(),
                size: Some(101),
                ..Default::default()
            },
            TestMediaFact {
                url: "https://media.example/a".to_owned(),
                mime_type: "video/webm".to_owned(),
                digest: Some("NOT-A-DIGEST".to_owned()),
                ..Default::default()
            },
        ] {
            assert_eq!(
                PublicChannelAttachment::try_from_media_fact(&fact, 100),
                Err(PublicChannelMediaUnavailableReason::UnsafeMetadata)
            );
        }
    }

    #[test]
    fn media_keys_keep_channel_event_and_attachment_state_independent() {
        let keys = BTreeSet::from([
            PublicChannelMediaKey::new("agent-chat", "event", 0),
            PublicChannelMediaKey::new("agent-lab", "event", 0),
            PublicChannelMediaKey::new("agent-chat", "other", 0),
            PublicChannelMediaKey::new("agent-chat", "event", 1),
        ]);
        assert_eq!(keys.len(), 4);
    }

    #[test]
    fn only_a_gated_state_can_begin_a_load() {
        let mut state = PublicChannelMediaState::Gated;
        assert!(state.begin_load());
        assert_eq!(state.lifecycle(), PublicChannelMediaLifecycle::Loading);
        assert!(!state.begin_load());
        assert_eq!(state.lifecycle().label(), "Loading media");
    }

    #[test]
    fn unsafe_urls_fail_before_the_http_client_is_called() {
        smol::block_on(async {
            let requests = Arc::new(Mutex::new(0));
            let captured = requests.clone();
            let client = FakeHttpClient::create(move |_| {
                *captured.lock().expect("request count") += 1;
                async { unreachable!("an unsafe URL must not reach HTTP") }
            });
            for url in [
                "file:///etc/passwd",
                "https://user:secret@example.com/file",
                "https://localhost/file",
                "https://127.0.0.1/file",
                "https://example.com/file#fragment",
            ] {
                let mut attachment = fixture_attachment();
                attachment.url = url.to_owned();
                let state = fetch_public_channel_media_with_policy(
                    client.clone(),
                    attachment,
                    100,
                    allow_test_host,
                )
                .await;
                assert_eq!(state.lifecycle(), PublicChannelMediaLifecycle::Unavailable);
            }
            assert_eq!(*requests.lock().expect("request count"), 0);
        });
    }

    #[test]
    fn reader_action_precedes_the_first_request_and_fetch_omits_private_headers() {
        smol::block_on(async {
            let payload = base64::engine::general_purpose::STANDARD
                .decode(FIXTURE_PAYLOAD_BASE64)
                .expect("fixture payload");
            let request_facts = Arc::new(Mutex::new(Vec::new()));
            let captured = request_facts.clone();
            let client = FakeHttpClient::create(move |request| {
                captured.lock().expect("facts").push((
                    request.uri().to_string(),
                    request.headers().clone(),
                    request.extensions().get::<RedirectPolicy>().cloned(),
                ));
                let payload = payload.clone();
                async move {
                    Ok(response(
                        StatusCode::OK,
                        Some("text/plain; charset=utf-8"),
                        payload,
                    ))
                }
            });

            let mut state = PublicChannelMediaState::Gated;
            assert_eq!(request_facts.lock().expect("facts").len(), 0);
            assert!(state.begin_load());
            assert_eq!(request_facts.lock().expect("facts").len(), 0);
            state = fetch_public_channel_media_with_policy(
                client,
                fixture_attachment(),
                100,
                allow_test_host,
            )
            .await;

            let PublicChannelMediaState::Verified(media) = state else {
                panic!("matching bytes must verify");
            };
            assert_eq!(media.actual_digest, FIXTURE_DIGEST);
            assert_eq!(media.byte_len, 27);
            assert!(media.temporary_path().exists());
            assert!(matches!(
                media.intent(),
                PublicChannelMediaIntent::SaveAs { .. }
            ));
            let facts = request_facts.lock().expect("facts");
            assert_eq!(facts.len(), 1);
            let (_, headers, redirect_policy) = &facts[0];
            assert!(!headers.contains_key(header::AUTHORIZATION));
            assert!(!headers.contains_key(header::COOKIE));
            assert!(!headers.contains_key(header::REFERER));
            assert_eq!(redirect_policy, &Some(RedirectPolicy::NoFollow));
        });
    }

    #[test]
    fn digest_mismatch_discards_bytes_without_a_temp_artifact() {
        smol::block_on(async {
            let before = media_temp_files();
            let client = FakeHttpClient::create(|_| async {
                Ok(response(
                    StatusCode::OK,
                    Some("text/plain"),
                    b"changed".as_slice(),
                ))
            });
            let state = fetch_public_channel_media_with_policy(
                client,
                fixture_attachment(),
                100,
                allow_test_host,
            )
            .await;
            let PublicChannelMediaState::Mismatch { expected, actual } = state else {
                panic!("changed bytes must mismatch");
            };
            assert_eq!(expected, FIXTURE_DIGEST);
            assert_ne!(actual, expected);
            assert_eq!(media_temp_files(), before);
        });
    }

    fn media_temp_files() -> BTreeSet<String> {
        std::fs::read_dir(paths::temp_dir())
            .expect("temp dir")
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.starts_with("omega-public-media-"))
            .collect()
    }

    #[test]
    fn redirects_are_revalidated_bounded_and_cannot_downgrade_https() {
        smol::block_on(async {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let captured = calls.clone();
            let client = FakeHttpClient::create(move |request| {
                captured
                    .lock()
                    .expect("calls")
                    .push(request.uri().to_string());
                async move {
                    Ok(Response::builder()
                        .status(StatusCode::FOUND)
                        .header(header::LOCATION, "/again")
                        .body(AsyncBody::empty())
                        .expect("redirect"))
                }
            });
            let state = fetch_public_channel_media_with_policy(
                client,
                fixture_attachment(),
                100,
                allow_test_host,
            )
            .await;
            assert!(matches!(
                state,
                PublicChannelMediaState::Unavailable {
                    reason: PublicChannelMediaUnavailableReason::TooManyRedirects
                }
            ));
            assert_eq!(calls.lock().expect("calls").len(), MAX_MEDIA_REDIRECTS + 1);

            assert_eq!(
                validate_media_url(
                    "http://media.example/file",
                    Some(&Url::parse("https://media.example/start").expect("base"))
                ),
                Err(PublicChannelMediaUnavailableReason::UnsafeUrl)
            );
        });
    }

    #[test]
    fn missing_redirect_mime_mismatch_http_failure_and_size_are_bounded() {
        smol::block_on(async {
            for (response, expected) in [
                (
                    Response::builder()
                        .status(StatusCode::FOUND)
                        .body(AsyncBody::empty())
                        .expect("redirect"),
                    PublicChannelMediaUnavailableReason::MissingRedirectLocation,
                ),
                (
                    response(StatusCode::OK, Some("application/pdf"), b"data".as_slice()),
                    PublicChannelMediaUnavailableReason::MimeMismatch,
                ),
                (
                    response(StatusCode::NOT_FOUND, None, AsyncBody::empty()),
                    PublicChannelMediaUnavailableReason::HttpStatus,
                ),
            ] {
                let response = Arc::new(Mutex::new(Some(response)));
                let captured = response.clone();
                let client = FakeHttpClient::create(move |_| {
                    let response = captured.lock().expect("response").take().expect("one call");
                    async move { Ok(response) }
                });
                let state = fetch_public_channel_media_with_policy(
                    client,
                    fixture_attachment(),
                    100,
                    allow_test_host,
                )
                .await;
                assert!(matches!(
                    state,
                    PublicChannelMediaState::Unavailable { reason } if reason == expected
                ));
            }

            for declared_length in [None, Some("1")] {
                let client = FakeHttpClient::create(move |_| async move {
                    let mut builder = Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, "text/plain");
                    if let Some(length) = declared_length {
                        builder = builder.header(header::CONTENT_LENGTH, length);
                    }
                    Ok(builder.body(vec![0; 11].into()).expect("response"))
                });
                let mut attachment = fixture_attachment();
                attachment.digest = None;
                let state =
                    fetch_public_channel_media_with_policy(client, attachment, 10, allow_test_host)
                        .await;
                assert!(matches!(
                    state,
                    PublicChannelMediaState::Unavailable {
                        reason: PublicChannelMediaUnavailableReason::TooLarge
                    }
                ));
            }
        });
    }

    #[test]
    fn verified_audio_uses_a_native_open_intent_without_calling_the_os() {
        smol::block_on(async {
            let payload = b"not decoded by omega".to_vec();
            let client = FakeHttpClient::create(move |_| {
                let payload = payload.clone();
                async move { Ok(response(StatusCode::OK, Some("audio/mpeg"), payload)) }
            });
            let mut attachment = fixture_attachment();
            attachment.mime_type = "audio/mpeg".to_owned();
            attachment.digest = None;
            let state =
                fetch_public_channel_media_with_policy(client, attachment, 100, allow_test_host)
                    .await;
            let PublicChannelMediaState::Verified(media) = state else {
                panic!("audio must verify without an in-process decoder");
            };
            assert!(matches!(
                media.intent(),
                PublicChannelMediaIntent::OpenWithSystem { .. }
            ));
            assert!(matches!(
                media.save_intent(),
                PublicChannelMediaIntent::SaveAs { .. }
            ));
        });
    }

    #[test]
    fn verified_png_gets_one_bounded_static_frame_and_bad_images_do_not_render() {
        smol::block_on(async {
            let encoded = png(2, 3);
            let client = FakeHttpClient::create(move |_| {
                let encoded = encoded.clone();
                async move { Ok(response(StatusCode::OK, Some("image/png"), encoded)) }
            });
            let mut attachment = fixture_attachment();
            attachment.mime_type = "image/png".to_owned();
            attachment.digest = None;
            let state = fetch_public_channel_media_with_policy(
                client,
                attachment.clone(),
                1024,
                allow_test_host,
            )
            .await;
            let PublicChannelMediaState::Verified(media) = state else {
                panic!("valid PNG must verify");
            };
            let PublicChannelMediaPresentation::InlineImage(image) = &media.presentation else {
                panic!("PNG must render inline");
            };
            assert_eq!(image.frame_count(), 1);
            assert_eq!(image.as_bytes(0).map(<[u8]>::len), Some(24));

            let client = FakeHttpClient::create(|_| async {
                Ok(response(
                    StatusCode::OK,
                    Some("image/png"),
                    b"not a png".as_slice(),
                ))
            });
            let state =
                fetch_public_channel_media_with_policy(client, attachment, 100, allow_test_host)
                    .await;
            assert!(matches!(
                state,
                PublicChannelMediaState::Unavailable {
                    reason: PublicChannelMediaUnavailableReason::ImageDecode
                }
            ));
        });
    }

    #[test]
    fn decoder_rejects_an_image_over_the_strict_edge_limit() {
        let state = decode_static_image(png(MAX_MEDIA_IMAGE_EDGE + 1, 1), image::ImageFormat::Png);
        assert_eq!(
            state.expect_err("oversized edge"),
            PublicChannelMediaUnavailableReason::ImageDecode
        );
    }
}
