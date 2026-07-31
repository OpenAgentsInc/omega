use std::{collections::BTreeSet, future::Future, pin::Pin};

use nostr::{Event, JsonUtil as _};
use omega_identity::{
    AccountRef, AccountSelectionToken, NostrPublicKeyHex, ReceiptRef, SigningResult,
    UnsignedEventTemplate,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HostPlatform {
    Desktop,
    Web,
    Android,
    Ios,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HostSignerCapability {
    DesktopLocal,
    Nip46,
    Nip07,
    Nip55,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostAdapterKind {
    DesktopFile,
    Nip46Remote,
    BrowserExtension,
    AndroidSignerApplication,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostSecretCustody {
    OmegaPrivateFile,
    RemoteSigner,
    BrowserExtension,
    AndroidSignerApplication,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostSecretPersistence {
    OmegaPrivateFile,
    Forbidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostCapabilityAvailability {
    Admitted,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostCapabilityDescriptor {
    pub platform: HostPlatform,
    pub capability: HostSignerCapability,
    pub adapter: HostAdapterKind,
    pub custody: HostSecretCustody,
    pub persistence: HostSecretPersistence,
    pub availability: HostCapabilityAvailability,
}

impl HostCapabilityDescriptor {
    pub fn label(self) -> &'static str {
        match self.capability {
            HostSignerCapability::DesktopLocal => "Local key in Omega private files",
            HostSignerCapability::Nip46 => "NIP-46 remote signer",
            HostSignerCapability::Nip07 => "NIP-07 browser extension",
            HostSignerCapability::Nip55 => "NIP-55 Android signer application",
        }
    }
}

pub struct HostCapabilityMatrix;

impl HostCapabilityMatrix {
    pub fn descriptor(
        platform: HostPlatform,
        capability: HostSignerCapability,
    ) -> HostCapabilityDescriptor {
        let (adapter, custody, persistence) = match capability {
            HostSignerCapability::DesktopLocal => (
                HostAdapterKind::DesktopFile,
                HostSecretCustody::OmegaPrivateFile,
                HostSecretPersistence::OmegaPrivateFile,
            ),
            HostSignerCapability::Nip46 => (
                HostAdapterKind::Nip46Remote,
                HostSecretCustody::RemoteSigner,
                HostSecretPersistence::Forbidden,
            ),
            HostSignerCapability::Nip07 => (
                HostAdapterKind::BrowserExtension,
                HostSecretCustody::BrowserExtension,
                HostSecretPersistence::Forbidden,
            ),
            HostSignerCapability::Nip55 => (
                HostAdapterKind::AndroidSignerApplication,
                HostSecretCustody::AndroidSignerApplication,
                HostSecretPersistence::Forbidden,
            ),
        };
        let availability = if matches!(
            (platform, capability),
            (HostPlatform::Desktop, HostSignerCapability::DesktopLocal)
                | (HostPlatform::Desktop, HostSignerCapability::Nip46)
                | (HostPlatform::Web, HostSignerCapability::Nip07)
                | (HostPlatform::Web, HostSignerCapability::Nip46)
                | (HostPlatform::Android, HostSignerCapability::Nip55)
                | (HostPlatform::Ios, HostSignerCapability::Nip46)
        ) {
            HostCapabilityAvailability::Admitted
        } else {
            HostCapabilityAvailability::Unsupported
        };
        HostCapabilityDescriptor {
            platform,
            capability,
            adapter,
            custody,
            persistence,
            availability,
        }
    }

    pub fn admitted(platform: HostPlatform) -> Vec<HostCapabilityDescriptor> {
        [
            HostSignerCapability::DesktopLocal,
            HostSignerCapability::Nip46,
            HostSignerCapability::Nip07,
            HostSignerCapability::Nip55,
        ]
        .into_iter()
        .map(|capability| Self::descriptor(platform, capability))
        .filter(|descriptor| descriptor.availability == HostCapabilityAvailability::Admitted)
        .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostAccountFence {
    pub account_ref: AccountRef,
    pub owner_public_key_hex: NostrPublicKeyHex,
    pub generation: u64,
}

impl HostAccountFence {
    pub fn from_selection(selection: &AccountSelectionToken) -> Self {
        Self {
            account_ref: selection.account_ref.clone(),
            owner_public_key_hex: selection.identity.public_key_hex().clone(),
            generation: selection.generation,
        }
    }

    pub fn verify(&self, selection: &AccountSelectionToken) -> Result<(), HostAdmissionError> {
        if self.account_ref != selection.account_ref
            || self.owner_public_key_hex != *selection.identity.public_key_hex()
            || self.generation == 0
            || self.generation != selection.generation
        {
            return Err(HostAdmissionError::StaleAccountFence);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostAdapterRef(String);

impl HostAdapterRef {
    pub fn new(value: impl Into<String>) -> Result<Self, HostAdmissionError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 256
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':')
            })
        {
            return Err(HostAdmissionError::InvalidAdapterReference);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostCapabilityAdmissionRequest {
    pub fence: HostAccountFence,
    pub platform: HostPlatform,
    pub capability: HostSignerCapability,
    pub adapter: HostAdapterKind,
    pub adapter_ref: HostAdapterRef,
    pub host_public_key_hex: NostrPublicKeyHex,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostAdapterCapabilityReport {
    pub platform: HostPlatform,
    pub adapter: HostAdapterKind,
    pub adapter_ref: HostAdapterRef,
    pub signer_public_key_hex: NostrPublicKeyHex,
    pub capabilities: BTreeSet<HostSignerCapability>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostAdapterOperation {
    GetPublicKey,
    SignEvent(UnsignedEventTemplate),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostAdapterRequest {
    pub request_ref: ReceiptRef,
    pub fence: HostAccountFence,
    pub capability: HostSignerCapability,
    pub operation: HostAdapterOperation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostAdapterResult {
    PublicKey {
        request_ref: ReceiptRef,
        public_key: NostrPublicKeyHex,
    },
    SignedEvent(SigningResult),
}

pub trait HostSignerAdapter: Send + Sync {
    fn capability_report(&self) -> HostAdapterCapabilityReport;

    fn execute(
        &self,
        request: HostAdapterRequest,
    ) -> Pin<
        Box<dyn Future<Output = Result<HostAdapterResult, HostAdapterCallError>> + Send + 'static>,
    >;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostAuthority {
    PersonSigner,
    DeviceMirror,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedHostSignerCapability {
    fence: HostAccountFence,
    platform: HostPlatform,
    capability: HostSignerCapability,
    adapter: HostAdapterKind,
    adapter_ref: HostAdapterRef,
    host_public_key_hex: NostrPublicKeyHex,
}

impl AdmittedHostSignerCapability {
    pub fn authority(&self) -> HostAuthority {
        HostAuthority::PersonSigner
    }

    pub fn capability(&self) -> HostSignerCapability {
        self.capability
    }

    pub fn adapter_ref(&self) -> &HostAdapterRef {
        &self.adapter_ref
    }

    pub fn authorize(
        &self,
        selection: &AccountSelectionToken,
        capability: HostSignerCapability,
        host_public_key_hex: &NostrPublicKeyHex,
    ) -> Result<(), HostAdmissionError> {
        self.fence.verify(selection)?;
        if self.capability != capability || self.host_public_key_hex != *host_public_key_hex {
            return Err(HostAdmissionError::CapabilityFenceMismatch);
        }
        Ok(())
    }
}

pub fn admit_host_signer_capability(
    selection: &AccountSelectionToken,
    request: HostCapabilityAdmissionRequest,
    report: &HostAdapterCapabilityReport,
) -> Result<AdmittedHostSignerCapability, HostAdmissionError> {
    request.fence.verify(selection)?;
    let descriptor = HostCapabilityMatrix::descriptor(request.platform, request.capability);
    if descriptor.availability != HostCapabilityAvailability::Admitted {
        return Err(HostAdmissionError::UnsupportedCapability);
    }
    if descriptor.adapter != request.adapter {
        return Err(HostAdmissionError::WrongAdapter);
    }
    if report.platform != request.platform
        || report.adapter != request.adapter
        || report.adapter_ref != request.adapter_ref
        || report.signer_public_key_hex != request.host_public_key_hex
        || report.signer_public_key_hex != *selection.identity.public_key_hex()
        || report.capabilities != BTreeSet::from([request.capability])
    {
        return Err(HostAdmissionError::CapabilityReportMismatch);
    }
    Ok(AdmittedHostSignerCapability {
        fence: request.fence,
        platform: request.platform,
        capability: request.capability,
        adapter: request.adapter,
        adapter_ref: request.adapter_ref,
        host_public_key_hex: request.host_public_key_hex,
    })
}

pub async fn get_admitted_host_public_key(
    adapter: &dyn HostSignerAdapter,
    admitted: &AdmittedHostSignerCapability,
    selection: &AccountSelectionToken,
    request_ref: ReceiptRef,
) -> Result<NostrPublicKeyHex, HostAdapterCallError> {
    admitted
        .authorize(
            selection,
            admitted.capability,
            &admitted.host_public_key_hex,
        )
        .map_err(HostAdapterCallError::Admission)?;
    verify_adapter_report(adapter, admitted)?;
    let expected_request_ref = request_ref.clone();
    match adapter
        .execute(HostAdapterRequest {
            request_ref,
            fence: admitted.fence.clone(),
            capability: admitted.capability,
            operation: HostAdapterOperation::GetPublicKey,
        })
        .await?
    {
        HostAdapterResult::PublicKey {
            request_ref,
            public_key,
        } if request_ref == expected_request_ref && public_key == admitted.host_public_key_hex => {
            Ok(public_key)
        }
        _ => Err(HostAdapterCallError::MismatchedResponse),
    }
}

pub async fn sign_with_admitted_host(
    adapter: &dyn HostSignerAdapter,
    admitted: &AdmittedHostSignerCapability,
    selection: &AccountSelectionToken,
    request_ref: ReceiptRef,
    event: UnsignedEventTemplate,
) -> Result<Event, HostAdapterCallError> {
    admitted
        .authorize(
            selection,
            admitted.capability,
            &admitted.host_public_key_hex,
        )
        .map_err(HostAdapterCallError::Admission)?;
    verify_adapter_report(adapter, admitted)?;
    let expected_request_ref = request_ref.clone();
    let result = match adapter
        .execute(HostAdapterRequest {
            request_ref,
            fence: admitted.fence.clone(),
            capability: admitted.capability,
            operation: HostAdapterOperation::SignEvent(event.clone()),
        })
        .await?
    {
        HostAdapterResult::SignedEvent(result) => result,
        HostAdapterResult::PublicKey { .. } => {
            return Err(HostAdapterCallError::MismatchedResponse);
        }
    };
    let signed = Event::from_json(&result.signed_event_json)
        .map_err(|_| HostAdapterCallError::MismatchedResponse)?;
    signed
        .verify()
        .map_err(|_| HostAdapterCallError::MismatchedResponse)?;
    let tags = signed
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .collect::<Vec<_>>();
    if result.request_ref != expected_request_ref
        || result.identity != selection.identity
        || signed.pubkey.to_hex() != selection.identity.public_key_hex().as_str()
        || signed.id.to_hex() != result.event_id
        || signed.sig.to_string() != result.signature
        || signed.kind.as_u16() != event.kind
        || signed.created_at.as_secs() != event.created_at
        || signed.content.as_bytes() != event.content.as_bytes()
        || tags != event.tags
    {
        return Err(HostAdapterCallError::MismatchedResponse);
    }
    Ok(signed)
}

fn verify_adapter_report(
    adapter: &dyn HostSignerAdapter,
    admitted: &AdmittedHostSignerCapability,
) -> Result<(), HostAdapterCallError> {
    let report = adapter.capability_report();
    if report.platform != admitted.platform
        || report.adapter != admitted.adapter
        || report.adapter_ref != admitted.adapter_ref
        || report.signer_public_key_hex != admitted.host_public_key_hex
        || report.capabilities != BTreeSet::from([admitted.capability])
    {
        return Err(HostAdapterCallError::CapabilityChanged);
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceMirrorAuthorityReceipt {
    pub fence: HostAccountFence,
    pub device_public_key_hex: NostrPublicKeyHex,
    pub pairing_transcript_sha256: String,
}

impl DeviceMirrorAuthorityReceipt {
    pub fn new(
        selection: &AccountSelectionToken,
        device_public_key_hex: NostrPublicKeyHex,
        pairing_transcript_sha256: impl Into<String>,
    ) -> Result<Self, HostAdmissionError> {
        let pairing_transcript_sha256 = pairing_transcript_sha256.into();
        if pairing_transcript_sha256.len() != 64
            || !pairing_transcript_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(HostAdmissionError::InvalidPairingTranscript);
        }
        Ok(Self {
            fence: HostAccountFence::from_selection(selection),
            device_public_key_hex,
            pairing_transcript_sha256,
        })
    }

    pub fn authority(&self) -> HostAuthority {
        HostAuthority::DeviceMirror
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostAdmissionError {
    StaleAccountFence,
    InvalidAdapterReference,
    UnsupportedCapability,
    WrongAdapter,
    CapabilityReportMismatch,
    CapabilityFenceMismatch,
    InvalidPairingTranscript,
}

impl std::fmt::Display for HostAdmissionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::StaleAccountFence => "the owner account generation changed",
            Self::InvalidAdapterReference => "the host adapter reference is invalid",
            Self::UnsupportedCapability => "the signer capability is unsupported on this host",
            Self::WrongAdapter => "the signer capability was presented by the wrong host adapter",
            Self::CapabilityReportMismatch => {
                "the host did not explicitly report the exact signer capability"
            }
            Self::CapabilityFenceMismatch => "the signer capability fence does not match",
            Self::InvalidPairingTranscript => "the device pairing transcript is invalid",
        })
    }
}

impl std::error::Error for HostAdmissionError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostAdapterCallError {
    Admission(HostAdmissionError),
    CapabilityChanged,
    MismatchedResponse,
    HostUnavailable,
    UserRejected,
}

impl std::fmt::Display for HostAdapterCallError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Admission(_) => "the host signer admission is no longer valid",
            Self::CapabilityChanged => "the host signer capability report changed",
            Self::MismatchedResponse => "the host signer returned a different operation result",
            Self::HostUnavailable => "the host signer is unavailable",
            Self::UserRejected => "the host signer request was rejected",
        })
    }
}

impl std::error::Error for HostAdapterCallError {}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, SecretKey, Tag, Timestamp};
    use omega_identity::{IdentityRef, PublicIdentity};

    fn selection(generation: u64) -> AccountSelectionToken {
        let keys = Keys::new(SecretKey::from_hex(&"1".repeat(64)).expect("secret key"));
        AccountSelectionToken {
            account_ref: AccountRef::new("omega-account-host").expect("account"),
            identity: PublicIdentity::from_public_key_hex(
                IdentityRef::new("omega-host-owner").expect("identity"),
                keys.public_key().to_hex(),
            )
            .expect("public identity"),
            generation,
        }
    }

    fn selection_for_keys(keys: &Keys, generation: u64) -> AccountSelectionToken {
        AccountSelectionToken {
            account_ref: AccountRef::new("omega-account-host-keys").expect("account"),
            identity: PublicIdentity::from_public_key_hex(
                IdentityRef::new("omega-host-owner-keys").expect("identity"),
                keys.public_key().to_hex(),
            )
            .expect("public identity"),
            generation,
        }
    }

    struct StaticHostAdapter {
        report: HostAdapterCapabilityReport,
        result: HostAdapterResult,
    }

    impl HostSignerAdapter for StaticHostAdapter {
        fn capability_report(&self) -> HostAdapterCapabilityReport {
            self.report.clone()
        }

        fn execute(
            &self,
            _request: HostAdapterRequest,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<HostAdapterResult, HostAdapterCallError>>
                    + Send
                    + 'static,
            >,
        > {
            let result = self.result.clone();
            Box::pin(async move { Ok(result) })
        }
    }

    #[test]
    fn platform_matrix_is_exact_and_does_not_imply_secret_storage() {
        assert_eq!(
            HostCapabilityMatrix::admitted(HostPlatform::Desktop)
                .into_iter()
                .map(|descriptor| descriptor.capability)
                .collect::<Vec<_>>(),
            vec![
                HostSignerCapability::DesktopLocal,
                HostSignerCapability::Nip46
            ]
        );
        assert_eq!(
            HostCapabilityMatrix::admitted(HostPlatform::Web)
                .into_iter()
                .map(|descriptor| descriptor.capability)
                .collect::<Vec<_>>(),
            vec![HostSignerCapability::Nip46, HostSignerCapability::Nip07]
        );
        assert_eq!(
            HostCapabilityMatrix::admitted(HostPlatform::Android)
                .into_iter()
                .map(|descriptor| descriptor.capability)
                .collect::<Vec<_>>(),
            vec![HostSignerCapability::Nip55]
        );
        assert_eq!(
            HostCapabilityMatrix::admitted(HostPlatform::Ios)
                .into_iter()
                .map(|descriptor| descriptor.capability)
                .collect::<Vec<_>>(),
            vec![HostSignerCapability::Nip46]
        );
        for platform in [HostPlatform::Web, HostPlatform::Android, HostPlatform::Ios] {
            assert!(
                HostCapabilityMatrix::admitted(platform)
                    .iter()
                    .all(|descriptor| descriptor.persistence == HostSecretPersistence::Forbidden)
            );
        }
    }

    #[test]
    fn admission_is_account_generation_capability_and_adapter_fenced() {
        let current_selection = selection(3);
        let host_key = current_selection.identity.public_key_hex().clone();
        let adapter_ref = HostAdapterRef::new("browser.nostr-provider").expect("adapter");
        let request = HostCapabilityAdmissionRequest {
            fence: HostAccountFence::from_selection(&current_selection),
            platform: HostPlatform::Web,
            capability: HostSignerCapability::Nip07,
            adapter: HostAdapterKind::BrowserExtension,
            adapter_ref: adapter_ref.clone(),
            host_public_key_hex: host_key.clone(),
        };
        let report = HostAdapterCapabilityReport {
            platform: HostPlatform::Web,
            adapter: HostAdapterKind::BrowserExtension,
            adapter_ref,
            signer_public_key_hex: host_key.clone(),
            capabilities: BTreeSet::from([HostSignerCapability::Nip07]),
        };
        let admitted =
            admit_host_signer_capability(&current_selection, request, &report).expect("admitted");
        admitted
            .authorize(&current_selection, HostSignerCapability::Nip07, &host_key)
            .expect("authorized");
        assert_eq!(
            admitted.authorize(&selection(4), HostSignerCapability::Nip07, &host_key),
            Err(HostAdmissionError::StaleAccountFence)
        );
        assert_eq!(
            admitted.authorize(&current_selection, HostSignerCapability::Nip46, &host_key),
            Err(HostAdmissionError::CapabilityFenceMismatch)
        );

        let mut unreported = report;
        unreported.capabilities.clear();
        let request = HostCapabilityAdmissionRequest {
            fence: HostAccountFence::from_selection(&current_selection),
            platform: HostPlatform::Web,
            capability: HostSignerCapability::Nip07,
            adapter: HostAdapterKind::BrowserExtension,
            adapter_ref: unreported.adapter_ref.clone(),
            host_public_key_hex: host_key,
        };
        assert_eq!(
            admit_host_signer_capability(&current_selection, request, &unreported),
            Err(HostAdmissionError::CapabilityReportMismatch)
        );
    }

    #[test]
    fn device_pairing_authority_never_becomes_person_signer_authority() {
        let selection = selection(1);
        let device_key = NostrPublicKeyHex::new("3".repeat(64)).expect("device key");
        let mirror = DeviceMirrorAuthorityReceipt::new(&selection, device_key, "a".repeat(64))
            .expect("mirror receipt");
        assert_eq!(mirror.authority(), HostAuthority::DeviceMirror);

        let android_nip55 =
            HostCapabilityMatrix::descriptor(HostPlatform::Android, HostSignerCapability::Nip55);
        assert_eq!(
            android_nip55.availability,
            HostCapabilityAvailability::Admitted
        );
        assert_eq!(android_nip55.persistence, HostSecretPersistence::Forbidden);
        assert_ne!(mirror.authority(), HostAuthority::PersonSigner);
    }

    #[test]
    fn explicitly_reported_nip55_adapter_can_only_return_the_exact_signed_event() {
        smol::block_on(async {
            let keys = Keys::generate();
            let selection = selection_for_keys(&keys, 2);
            let signer_public_key_hex = selection.identity.public_key_hex().clone();
            let adapter_ref = HostAdapterRef::new("android.nip55.signer").expect("adapter");
            let report = HostAdapterCapabilityReport {
                platform: HostPlatform::Android,
                adapter: HostAdapterKind::AndroidSignerApplication,
                adapter_ref: adapter_ref.clone(),
                signer_public_key_hex: signer_public_key_hex.clone(),
                capabilities: BTreeSet::from([HostSignerCapability::Nip55]),
            };
            let admitted = admit_host_signer_capability(
                &selection,
                HostCapabilityAdmissionRequest {
                    fence: HostAccountFence::from_selection(&selection),
                    platform: HostPlatform::Android,
                    capability: HostSignerCapability::Nip55,
                    adapter: HostAdapterKind::AndroidSignerApplication,
                    adapter_ref,
                    host_public_key_hex: signer_public_key_hex,
                },
                &report,
            )
            .expect("admitted NIP-55 adapter");
            let template = UnsignedEventTemplate {
                created_at: 42,
                kind: 1,
                tags: vec![vec!["t".to_string(), "omega".to_string()]],
                content: "exact content".to_string(),
            };
            let signed = EventBuilder::new(Kind::from(1), "exact content")
                .tags(vec![Tag::parse(["t", "omega"]).expect("tag")])
                .custom_created_at(Timestamp::from_secs(42))
                .sign_with_keys(&keys)
                .expect("signed event");
            let adapter = StaticHostAdapter {
                report: report.clone(),
                result: HostAdapterResult::SignedEvent(SigningResult {
                    request_ref: ReceiptRef::new("android-sign-event").expect("request"),
                    identity: selection.identity.clone(),
                    event_id: signed.id.to_hex(),
                    signature: signed.sig.to_string(),
                    signed_event_json: signed.as_json(),
                }),
            };
            let verified = sign_with_admitted_host(
                &adapter,
                &admitted,
                &selection,
                ReceiptRef::new("android-sign-event").expect("request"),
                template.clone(),
            )
            .await
            .expect("verified host signature");
            assert_eq!(verified, signed);

            let wrong = EventBuilder::new(Kind::from(1), "different content")
                .tags(vec![Tag::parse(["t", "omega"]).expect("tag")])
                .custom_created_at(Timestamp::from_secs(42))
                .sign_with_keys(&keys)
                .expect("wrong event");
            let wrong_adapter = StaticHostAdapter {
                report,
                result: HostAdapterResult::SignedEvent(SigningResult {
                    request_ref: ReceiptRef::new("android-sign-event").expect("request"),
                    identity: selection.identity.clone(),
                    event_id: wrong.id.to_hex(),
                    signature: wrong.sig.to_string(),
                    signed_event_json: wrong.as_json(),
                }),
            };
            assert_eq!(
                sign_with_admitted_host(
                    &wrong_adapter,
                    &admitted,
                    &selection,
                    ReceiptRef::new("android-sign-event").expect("request"),
                    template,
                )
                .await,
                Err(HostAdapterCallError::MismatchedResponse)
            );
        });
    }
}
