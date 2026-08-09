use std::sync::Arc;

use futures::{AsyncReadExt as _, FutureExt as _, future::BoxFuture};
use gpui::{App, Global};
use http_client::{AsyncBody, HttpClient};

pub use lnmarkets_client::*;
pub use lnmarkets_ui::LnMarketsSettingsPage;

const MAX_TRANSPORT_RESPONSE_BYTES: u64 = 1_048_577;

pub const REST_HOSTS: &[&str] = &["api.signet.lnmarkets.com", "api.lnmarkets.com"];
pub const STREAM_HOSTS: &[&str] = &["stream.signet.lnmarkets.com", "stream.lnmarkets.com"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PluginManifest {
    pub name: &'static str,
    pub rest_hosts: &'static [&'static str],
    pub stream_hosts: &'static [&'static str],
}

pub const MANIFEST: PluginManifest = PluginManifest {
    name: "LN Markets",
    rest_hosts: REST_HOSTS,
    stream_hosts: STREAM_HOSTS,
};

#[derive(Default)]
pub struct LnMarketsPlugin;

impl Global for LnMarketsPlugin {}

pub fn init(cx: &mut App) {
    let _registrations = (
        lnmarkets_data::REGISTRATION,
        lnmarkets_trading::REGISTRATION,
    );
    cx.set_global(LnMarketsPlugin);
}

struct OmegaHttpTransport {
    http_client: Arc<dyn HttpClient>,
}

impl HttpTransport for OmegaHttpTransport {
    fn send(
        &self,
        request: http::Request<Vec<u8>>,
    ) -> BoxFuture<'static, anyhow::Result<http::Response<Vec<u8>>>> {
        let http_client = self.http_client.clone();
        async move {
            let request = request.map(AsyncBody::from);
            let response = http_client.send(request).await?;
            let (parts, body) = response.into_parts();
            let mut bytes = Vec::new();
            body.take(MAX_TRANSPORT_RESPONSE_BYTES)
                .read_to_end(&mut bytes)
                .await?;
            Ok(http::Response::from_parts(parts, bytes))
        }
        .boxed()
    }
}

pub fn http_transport(http_client: Arc<dyn HttpClient>) -> Arc<dyn HttpTransport> {
    Arc::new(OmegaHttpTransport { http_client })
}
