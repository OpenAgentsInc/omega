use std::fmt;
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow, bail};
use credentials_provider::CredentialsProvider;
use futures::AsyncReadExt as _;
use gpui::AsyncApp;
use http_client::{AsyncBody, HttpClient, Method, Request};
use secp256k1::{PublicKey, Secp256k1, SecretKey};
use serde::{Deserialize, Serialize};
use sha3::{Digest as _, Keccak256};
use zeroize::{Zeroize as _, Zeroizing};

use crate::Network;

pub const AGENT_WALLET_SCHEMA_VERSION: u16 = 1;
pub const AGENT_WALLET_AUTHORITY_COPY: &str =
    "Omega can trade on this account; Omega cannot withdraw.";

const TESTNET_CREDENTIAL_KEY: &str = "omega://nautilus/hyperliquid/testnet/agent-wallet-v1";
const MAINNET_CREDENTIAL_KEY: &str = "omega://nautilus/hyperliquid/mainnet/agent-wallet-v1";
const TESTNET_INFO_URL: &str = "https://api.hyperliquid-testnet.xyz/info";

fn info_url(network: Network) -> Result<&'static str> {
    match network {
        Network::Testnet => Ok(TESTNET_INFO_URL),
        Network::Mainnet => bail!(
            "Hyperliquid mainnet connections are disabled until the mainnet graduation gate passes"
        ),
    }
}

pub fn credential_key(network: Network) -> &'static str {
    match network {
        Network::Testnet => TESTNET_CREDENTIAL_KEY,
        Network::Mainnet => MAINNET_CREDENTIAL_KEY,
    }
}

fn agent_name(network: Network) -> &'static str {
    match network {
        Network::Testnet => "omega-testnet",
        Network::Mainnet => "omega-mainnet",
    }
}

fn validate_address(label: &str, address: &str) -> Result<()> {
    let Some(hex) = address.strip_prefix("0x") else {
        bail!("{label} must start with 0x");
    };
    if hex.len() != 40 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("{label} must be a 20-byte hexadecimal address");
    }
    Ok(())
}

pub(crate) fn ethereum_address(public_key: &PublicKey) -> Result<String> {
    let serialized = public_key.serialize_uncompressed();
    let coordinates = serialized
        .get(1..)
        .context("secp256k1 public key has no coordinates")?;
    let digest = Keccak256::digest(coordinates);
    let address = digest
        .get(12..)
        .context("Keccak-256 digest is shorter than an Ethereum address")?;
    Ok(format!("0x{}", hex::encode(address)))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentApprovalStatus {
    Pending,
    Approved { valid_until_ms: i64 },
    Expired { valid_until_ms: i64 },
    Revoked,
    UnknownMode { raw: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentWalletHaltReason {
    Expired {
        valid_until_ms: i64,
    },
    Revoked,
    UnknownMode {
        raw: String,
    },
    NetworkMismatch {
        expected: Network,
        observed: Network,
    },
}

impl AgentWalletHaltReason {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Expired { .. } => "agent_wallet_expired",
            Self::Revoked => "agent_wallet_revoked",
            Self::UnknownMode { .. } => "agent_wallet_unknown_mode",
            Self::NetworkMismatch { .. } => "agent_wallet_network_mismatch",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentWalletSummary {
    pub network: Network,
    pub owner_address: String,
    pub agent_address: String,
    pub agent_name: String,
    pub approval: AgentApprovalStatus,
}

impl AgentWalletSummary {
    pub fn halt_reason(&self, expected_network: Network) -> Option<AgentWalletHaltReason> {
        if self.network != expected_network {
            return Some(AgentWalletHaltReason::NetworkMismatch {
                expected: expected_network,
                observed: self.network,
            });
        }
        match &self.approval {
            AgentApprovalStatus::Pending | AgentApprovalStatus::Approved { .. } => None,
            AgentApprovalStatus::Expired { valid_until_ms } => {
                Some(AgentWalletHaltReason::Expired {
                    valid_until_ms: *valid_until_ms,
                })
            }
            AgentApprovalStatus::Revoked => Some(AgentWalletHaltReason::Revoked),
            AgentApprovalStatus::UnknownMode { raw } => {
                Some(AgentWalletHaltReason::UnknownMode { raw: raw.clone() })
            }
        }
    }
}

#[derive(Deserialize, Serialize)]
struct StoredAgentWalletWire {
    schema_version: u16,
    network: Network,
    owner_address: String,
    agent_address: String,
    agent_name: String,
    private_key: String,
    created_at_ms: i64,
    approval: AgentApprovalStatus,
}

impl Drop for StoredAgentWalletWire {
    fn drop(&mut self) {
        self.private_key.zeroize();
    }
}

pub struct StoredAgentWallet {
    pub network: Network,
    pub owner_address: String,
    pub agent_address: String,
    pub agent_name: String,
    private_key: Zeroizing<String>,
    pub created_at_ms: i64,
    pub approval: AgentApprovalStatus,
}

impl fmt::Debug for StoredAgentWallet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredAgentWallet")
            .field("network", &self.network)
            .field("owner_address", &self.owner_address)
            .field("agent_address", &self.agent_address)
            .field("agent_name", &self.agent_name)
            .field("private_key", &"[REDACTED]")
            .field("created_at_ms", &self.created_at_ms)
            .field("approval", &self.approval)
            .finish()
    }
}

impl StoredAgentWallet {
    pub fn generate(
        network: Network,
        owner_address: impl Into<String>,
        created_at_ms: i64,
    ) -> Result<Self> {
        let owner_address = owner_address.into().to_ascii_lowercase();
        validate_address("Hyperliquid owner address", &owner_address)?;
        let secp = Secp256k1::new();
        let secret_key = SecretKey::new(&mut rand::rng());
        let public_key = PublicKey::from_secret_key(&secp, &secret_key);
        let agent_address = ethereum_address(&public_key)?;
        let private_key = Zeroizing::new(format!("0x{}", secret_key.display_secret()));
        Ok(Self {
            network,
            owner_address,
            agent_address,
            agent_name: agent_name(network).to_owned(),
            private_key,
            created_at_ms,
            approval: AgentApprovalStatus::Pending,
        })
    }

    pub fn summary(&self) -> AgentWalletSummary {
        AgentWalletSummary {
            network: self.network,
            owner_address: self.owner_address.clone(),
            agent_address: self.agent_address.clone(),
            agent_name: self.agent_name.clone(),
            approval: self.approval.clone(),
        }
    }

    pub fn private_key_bytes(&self) -> Vec<u8> {
        self.private_key.as_bytes().to_vec()
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(&StoredAgentWalletWire {
            schema_version: AGENT_WALLET_SCHEMA_VERSION,
            network: self.network,
            owner_address: self.owner_address.clone(),
            agent_address: self.agent_address.clone(),
            agent_name: self.agent_name.clone(),
            private_key: self.private_key.to_string(),
            created_at_ms: self.created_at_ms,
            approval: self.approval.clone(),
        })
        .context("encode Hyperliquid agent-wallet record")
    }

    pub fn decode(mut bytes: Vec<u8>, expected_network: Network) -> Result<Self> {
        let result = (|| {
            let mut wire: StoredAgentWalletWire =
                serde_json::from_slice(&bytes).context("decode Hyperliquid agent-wallet record")?;
            if wire.schema_version != AGENT_WALLET_SCHEMA_VERSION {
                bail!("unsupported Hyperliquid agent-wallet schema version");
            }
            if wire.network != expected_network {
                bail!("stored Hyperliquid agent-wallet network does not match");
            }
            validate_address("stored owner address", &wire.owner_address)?;
            validate_address("stored agent address", &wire.agent_address)?;
            if wire.agent_name != agent_name(wire.network) {
                bail!("stored Hyperliquid agent-wallet name does not match its network");
            }
            let private_key = Zeroizing::new(std::mem::take(&mut wire.private_key));
            if !private_key.starts_with("0x") || private_key.len() != 66 {
                bail!("stored Hyperliquid agent-wallet private key is malformed");
            }
            Ok(Self {
                network: wire.network,
                owner_address: std::mem::take(&mut wire.owner_address),
                agent_address: std::mem::take(&mut wire.agent_address),
                agent_name: std::mem::take(&mut wire.agent_name),
                private_key,
                created_at_ms: wire.created_at_ms,
                approval: wire.approval.clone(),
            })
        })();
        bytes.zeroize();
        result
    }
}

pub async fn load_agent_wallet(
    provider: &Arc<dyn CredentialsProvider>,
    network: Network,
    cx: &AsyncApp,
) -> Result<Option<StoredAgentWallet>> {
    let Some((_owner, bytes)) = provider
        .read_credentials(credential_key(network), cx)
        .await?
    else {
        return Ok(None);
    };
    StoredAgentWallet::decode(bytes, network).map(Some)
}

pub async fn generate_and_store_agent_wallet(
    provider: &Arc<dyn CredentialsProvider>,
    network: Network,
    owner_address: impl Into<String>,
    created_at_ms: i64,
    cx: &AsyncApp,
) -> Result<AgentWalletSummary> {
    let wallet = StoredAgentWallet::generate(network, owner_address, created_at_ms)?;
    let mut encoded = wallet.encode()?;
    let write_result = provider
        .write_credentials(credential_key(network), &wallet.owner_address, &encoded, cx)
        .await;
    encoded.zeroize();
    write_result.context("store Hyperliquid agent wallet in the platform credential store")?;
    Ok(wallet.summary())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtraAgent {
    name: String,
    address: String,
    #[serde(rename = "validUntil")]
    valid_until_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfficialOpenOrder {
    pub coin: String,
    pub oid: u64,
    pub cloid: Option<String>,
    pub side: String,
    pub sz: String,
    pub limit_px: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OfficialPosition {
    pub coin: String,
    pub size: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OfficialVenueState {
    pub network: Network,
    pub open_orders: Vec<OfficialOpenOrder>,
    pub positions: Vec<OfficialPosition>,
}

impl OfficialVenueState {
    pub fn is_zero_exposure(&self) -> bool {
        self.network == Network::Testnet && self.open_orders.is_empty() && self.positions.is_empty()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClearinghouseState {
    asset_positions: Vec<AssetPosition>,
}

#[derive(Deserialize)]
struct AssetPosition {
    position: PositionWire,
}

#[derive(Deserialize)]
struct PositionWire {
    coin: String,
    szi: String,
}

fn decimal_is_zero(value: &str) -> Result<bool> {
    let unsigned = value
        .strip_prefix('-')
        .or_else(|| value.strip_prefix('+'))
        .unwrap_or(value);
    if unsigned.is_empty()
        || unsigned
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && byte != b'.')
        || unsigned.bytes().filter(|byte| *byte == b'.').count() > 1
    {
        bail!("Hyperliquid position size is not a decimal");
    }
    Ok(unsigned.bytes().all(|byte| byte == b'0' || byte == b'.'))
}

pub fn evaluate_official_venue_state(
    open_orders_response: &[u8],
    clearinghouse_response: &[u8],
) -> Result<OfficialVenueState> {
    let open_orders = serde_json::from_slice::<Vec<OfficialOpenOrder>>(open_orders_response)
        .context("decode Hyperliquid openOrders response")?;
    let clearinghouse = serde_json::from_slice::<ClearinghouseState>(clearinghouse_response)
        .context("decode Hyperliquid clearinghouseState response")?;
    let mut positions = Vec::new();
    for asset in clearinghouse.asset_positions {
        if !decimal_is_zero(&asset.position.szi)? {
            positions.push(OfficialPosition {
                coin: asset.position.coin,
                size: asset.position.szi,
            });
        }
    }
    Ok(OfficialVenueState {
        network: Network::Testnet,
        open_orders,
        positions,
    })
}

async fn info_response(
    http_client: Arc<dyn HttpClient>,
    network: Network,
    body: Vec<u8>,
) -> Result<Vec<u8>> {
    let request = Request::builder()
        .method(Method::POST)
        .uri(info_url(network)?)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .body(AsyncBody::from(body))?;
    let mut response = http_client.send(request).await?;
    if !response.status().is_success() {
        bail!("Hyperliquid info probe returned {}", response.status());
    }
    let mut response_body = Vec::new();
    response.body_mut().read_to_end(&mut response_body).await?;
    Ok(response_body)
}

pub async fn probe_official_venue_state(
    http_client: Arc<dyn HttpClient>,
    network: Network,
    owner_address: &str,
) -> Result<OfficialVenueState> {
    validate_address("Hyperliquid owner address", owner_address)?;
    let open_orders = info_response(
        http_client.clone(),
        network,
        serde_json::to_vec(&serde_json::json!({
            "type": "openOrders",
            "user": owner_address,
        }))?,
    )
    .await?;
    let clearinghouse = info_response(
        http_client,
        network,
        serde_json::to_vec(&serde_json::json!({
            "type": "clearinghouseState",
            "user": owner_address,
        }))?,
    )
    .await?;
    evaluate_official_venue_state(&open_orders, &clearinghouse)
}

pub fn evaluate_extra_agents(
    wallet: &StoredAgentWallet,
    response: &[u8],
    now_ms: i64,
) -> AgentApprovalStatus {
    let agents = match serde_json::from_slice::<Vec<ExtraAgent>>(response) {
        Ok(agents) => agents,
        Err(_) => {
            return AgentApprovalStatus::UnknownMode {
                raw: "extra_agents_response".to_owned(),
            };
        }
    };
    let Some(agent) = agents.into_iter().find(|agent| {
        agent.address.eq_ignore_ascii_case(&wallet.agent_address) && agent.name == wallet.agent_name
    }) else {
        return AgentApprovalStatus::Revoked;
    };
    if agent.valid_until_ms <= now_ms {
        AgentApprovalStatus::Expired {
            valid_until_ms: agent.valid_until_ms,
        }
    } else {
        AgentApprovalStatus::Approved {
            valid_until_ms: agent.valid_until_ms,
        }
    }
}

pub async fn refresh_agent_wallet_approval(
    http_client: Arc<dyn HttpClient>,
    provider: &Arc<dyn CredentialsProvider>,
    network: Network,
    now_ms: i64,
    cx: &AsyncApp,
) -> Result<AgentWalletSummary> {
    let info_url = info_url(network)?;
    let mut wallet = load_agent_wallet(provider, network, cx)
        .await?
        .ok_or_else(|| anyhow!("Hyperliquid testnet agent wallet is not configured"))?;
    let body = serde_json::to_vec(&serde_json::json!({
        "type": "extraAgents",
        "user": wallet.owner_address,
    }))?;
    let request = Request::builder()
        .method(Method::POST)
        .uri(info_url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .body(AsyncBody::from(body))?;
    let mut response = http_client.send(request).await?;
    if !response.status().is_success() {
        bail!(
            "Hyperliquid extraAgents probe returned {}",
            response.status()
        );
    }
    let mut response_body = Vec::new();
    response.body_mut().read_to_end(&mut response_body).await?;
    wallet.approval = evaluate_extra_agents(&wallet, &response_body, now_ms);
    response_body.zeroize();
    let mut encoded = wallet.encode()?;
    let write_result = provider
        .write_credentials(credential_key(network), &wallet.owner_address, &encoded, cx)
        .await;
    encoded.zeroize();
    write_result?;
    Ok(wallet.summary())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wallet(network: Network) -> StoredAgentWallet {
        StoredAgentWallet::generate(network, "0x1111111111111111111111111111111111111111", 1_000)
            .expect("generate wallet")
    }

    #[test]
    fn generated_wallets_are_network_named_and_debug_redacted() {
        let testnet = wallet(Network::Testnet);
        let mainnet = wallet(Network::Mainnet);
        assert_eq!(testnet.agent_name, "omega-testnet");
        assert_eq!(mainnet.agent_name, "omega-mainnet");
        assert_ne!(
            credential_key(Network::Testnet),
            credential_key(Network::Mainnet)
        );
        let debug = format!("{testnet:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains(testnet.private_key.as_str()));
    }

    #[test]
    fn mainnet_approval_probe_has_no_endpoint() {
        assert_eq!(
            info_url(Network::Testnet).expect("testnet URL"),
            TESTNET_INFO_URL
        );
        assert!(info_url(Network::Mainnet).is_err());
    }

    #[test]
    fn official_venue_state_requires_zero_orders_and_positions() {
        let zero = evaluate_official_venue_state(
            br#"[]"#,
            br#"{"assetPositions":[{"position":{"coin":"BTC","szi":"0.000"}}]}"#,
        )
        .expect("zero venue state");
        assert!(zero.is_zero_exposure());

        let exposed = evaluate_official_venue_state(
            br#"[{"coin":"BTC","oid":7,"cloid":"0x01","side":"B","sz":"0.001","limitPx":"60000"}]"#,
            br#"{"assetPositions":[{"position":{"coin":"BTC","szi":"-0.001"}}]}"#,
        )
        .expect("exposed venue state");
        assert!(!exposed.is_zero_exposure());
        assert_eq!(exposed.open_orders[0].oid, 7);
        assert_eq!(exposed.positions[0].size, "-0.001");
    }

    #[test]
    fn records_refuse_cross_network_decode() {
        let encoded = wallet(Network::Testnet).encode().expect("encode wallet");
        assert!(StoredAgentWallet::decode(encoded, Network::Mainnet).is_err());
    }

    #[test]
    fn extra_agents_surface_valid_until_and_halt_on_expiry_or_unknown_mode() {
        let wallet = wallet(Network::Testnet);
        let approved = serde_json::to_vec(&serde_json::json!([{
            "name": wallet.agent_name,
            "address": wallet.agent_address,
            "validUntil": 2_000,
        }]))
        .expect("encode extraAgents fixture");
        assert_eq!(
            evaluate_extra_agents(&wallet, &approved, 1_500),
            AgentApprovalStatus::Approved {
                valid_until_ms: 2_000,
            }
        );
        assert_eq!(
            evaluate_extra_agents(&wallet, &approved, 2_000),
            AgentApprovalStatus::Expired {
                valid_until_ms: 2_000,
            }
        );
        assert!(matches!(
            evaluate_extra_agents(&wallet, b"{\"mode\":\"future\"}", 1_500),
            AgentApprovalStatus::UnknownMode { .. }
        ));
    }

    #[test]
    fn approval_halts_are_typed_and_network_bound() {
        let summary = AgentWalletSummary {
            network: Network::Testnet,
            owner_address: "0x1111111111111111111111111111111111111111".to_owned(),
            agent_address: "0x2222222222222222222222222222222222222222".to_owned(),
            agent_name: "omega-testnet".to_owned(),
            approval: AgentApprovalStatus::UnknownMode {
                raw: "future".to_owned(),
            },
        };
        assert_eq!(
            summary
                .halt_reason(Network::Testnet)
                .expect("unknown mode halts")
                .code(),
            "agent_wallet_unknown_mode"
        );
        assert!(matches!(
            summary.halt_reason(Network::Mainnet),
            Some(AgentWalletHaltReason::NetworkMismatch { .. })
        ));
    }
}
