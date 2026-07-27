# Capture the Omega public demo

Omega has an explicitly opt-in public demo mode for capturing a real product
workroom without exposing normal user state. The mode stages three visible
surfaces together:

- the fictional Orbit Notes project tree;
- `src/commandPalette.ts` in the editor;
- the Sarah workroom pane with a fictional conversation and activity history.

Run the normal development build and launch path:

```sh
script/omega-public-demo
```

The script creates a fresh temporary profile, prints its location, builds Omega
with the repository's regular `release-fast` profile, and launches the checked-in
fixture. To use an already-built binary:

```sh
OMEGA_DEMO_BINARY=/absolute/path/to/omega script/omega-public-demo
```

The underlying application invocation is:

```sh
omega \
  --full-editor \
  --demo-workroom \
  --user-data-dir /a/fresh/dedicated/profile \
  /path/to/omega/assets/demo-workroom \
  /path/to/omega/assets/demo-workroom/src/commandPalette.ts
```

`--demo-workroom` is rejected unless both `--full-editor` and
`--user-data-dir` are present. The dedicated editor flag makes the mode choice
explicit; the demo flag cannot silently leave zero base. In demo mode the
workroom uses a fixed in-memory projection, does not initialize its live
supervisor or account binding, and cannot send messages. The fixture contains
no credentials, real accounts, private paths, or real customer data. Normal
launches never enable the projection.

For a homepage capture, use a clean 1600×1000 window (or the requested matching
16:10 export size), keep the left project dock and right Sarah dock open, and
make sure `src/commandPalette.ts` is the active editor tab. Hide unrelated
desktop notifications and crop only to the Omega window. The visible `PUBLIC
DEMO` label is intentional provenance.

Before publishing, verify:

1. The title bar identifies Omega.
2. `Orbit Notes`, `commandPalette.ts`, and the `Sarah` pane are visible at once.
3. The workroom says `PUBLIC DEMO` and `fictional data`.
4. No home-directory segment, account name, notification, or unrelated project
   is visible.
