use gpui::{AsyncApp, actions};
use release_channel::ReleaseChannel;

actions!(
    cli,
    [
        /// Registers the current Omega URL scheme handler.
        #[action(deprecated_aliases = ["cli::RegisterZedScheme"])]
        RegisterAppScheme
    ]
);

pub async fn register_app_scheme(cx: &AsyncApp) -> anyhow::Result<()> {
    let scheme = cx.update(|cx| ReleaseChannel::global(cx).protocol_scheme());
    cx.update(|cx| cx.register_url_scheme(scheme)).await
}
