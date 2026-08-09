//! The only file in the application that names plugins. Everything else in
//! the app consumes plugin surfaces through `plugin_api::registry`, so adding
//! or omitting a plugin is a change to this file and its Cargo feature alone.

use gpui::App;
use plugin_api::{OmegaPlugin, PluginRegistry};

// One statement per plugin so each Cargo feature gates its own line.
#[allow(unused_mut, clippy::vec_init_then_push)]
pub fn builtin_plugins() -> Vec<Box<dyn OmegaPlugin>> {
    let mut plugins: Vec<Box<dyn OmegaPlugin>> = Vec::new();
    #[cfg(feature = "lnmarkets")]
    plugins.push(Box::new(lnmarkets::LnMarketsPlugin::new()));
    plugins
}

/// Register every built-in plugin and store the populated registry for the
/// app's lifetime.
pub fn init(cx: &mut App) {
    let mut registry = PluginRegistry::new(paths::data_dir().clone());
    for plugin in builtin_plugins() {
        registry.register_plugin(plugin.as_ref(), cx);
    }
    plugin_api::init_global(registry, cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every built-in plugin manifest must declare its complete network
    /// surface: the union of these declarations is the source of allowed
    /// plugin hosts, so an undeclared host is an unreachable host.
    #[test]
    fn builtin_plugin_manifests_declare_their_hosts_with_purposes() {
        for plugin in builtin_plugins() {
            let manifest = plugin.manifest();
            assert!(!manifest.id.is_empty());
            assert!(!manifest.name.is_empty());
            assert!(!manifest.version.is_empty());
            for host in manifest.hosts {
                assert!(
                    !host.host.is_empty(),
                    "{} declares an empty host",
                    manifest.id
                );
                assert!(
                    !host.purpose.is_empty(),
                    "{} host {} has no declared purpose",
                    manifest.id,
                    host.host
                );
                assert!(
                    !host.protocols.is_empty(),
                    "{} host {} declares no protocols",
                    manifest.id,
                    host.host
                );
            }
        }
    }
}
