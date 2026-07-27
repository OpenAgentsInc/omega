# Runtime credential storage

Omega does not access the macOS Keychain for application runtime credentials.

The shared credentials provider stores provider API keys, OAuth sessions,
OpenAgents native-session tokens, and similar byte credentials in
`credentials/credentials.json` below the channel-specific application data
directory. Records remain release-channel namespaced. Writes use an atomic
replacement, the directory is mode `0700`, and the file is mode `0600` on
Unix.

The Nostr signing secret is stored separately as
`identity/identity.secret` below the channel-specific application data
directory. It uses the same atomic-write and owner-only permission rules.

Exo is always launched with `EXO_SECRET_BACKEND=file` and an explicit master
key path. Configuring `apple-keychain` is rejected.

The version-one identity evidence schema still calls its stable logical
identity locator a `KeyringLocator`. That serialized compatibility name does
not invoke or describe the macOS Keychain; runtime secret storage is the local
file above. Omega does not read or migrate old Keychain entries because doing
so could trigger the prompt this change removes.

These files are intentionally not encrypted at rest. This removes unstable
binary-identity prompts from development and release builds, but it makes the
security boundary the user's operating-system account and application data
directory. Omega must never log or render their contents.

Apple code signing and notarization may use a build-machine signing identity.
That packaging operation is outside the installed application's runtime and
does not give Omega access to the build machine's credential store.
