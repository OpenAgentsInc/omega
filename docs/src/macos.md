---
title: Zed on macOS
description: "Zed is developed primarily on macOS, making it a first-class platform with full feature support."
---

# Zed on macOS

Zed is developed primarily on macOS, making it a first-class platform with full feature support.

## Installing Zed

Download Zed from the [download page](https://zed.dev/download). The download is a `.dmg` file—open it and drag Zed to your Applications folder.

For the preview build, which receives updates about a week ahead of stable, visit the [preview releases page](https://zed.dev/releases/preview).

After installation, Zed checks for updates automatically and prompts you when a new version is available.

### Homebrew

You can also install Zed using Homebrew:

```sh
brew install --cask zed
```

For the preview version:

```sh
brew install --cask zed@preview
```

### Building from Source

To build Zed from source, see the [macOS development documentation](./development/macos.md).

## System Requirements

- macOS 10.15.7 (Catalina) or later
- Apple Silicon (M1/M2/M3/M4) or Intel processor

Zed uses Metal for GPU-accelerated rendering, which is available on all supported macOS versions.

## Installing the CLI

Omega includes a command-line tool for opening files and projects from
Terminal. Add the bundled executable to your `PATH`, or create a symlink:

```sh
ln -s /Applications/Omega.app/Contents/MacOS/cli /usr/local/bin/omega

omega .                    # Open current folder
omega file.txt             # Open a file
omega project/ file.txt    # Open a folder and a file
```

See the [CLI Reference](./reference/cli.md) for all available options.

## Uninstall

1. Quit Zed if it's running
2. Drag Zed from Applications to the Trash
3. Optionally, remove your settings and extensions:

```sh
rm -rf ~/.config/zed
rm -rf ~/Library/Application\ Support/Zed
rm -rf ~/Library/Caches/Zed
rm -rf ~/Library/Logs/Zed
rm -rf ~/Library/Saved\ Application\ State/dev.zed.Zed.savedState
```

If you installed the CLI, remove it with:

```sh
rm /usr/local/bin/zed
```

## Troubleshooting

### Zed won't open or shows "damaged" warning

If macOS reports that Zed is damaged or can't be opened, it's likely a Gatekeeper issue. Try:

1. Right-click (or Control-click) on Zed in Applications
2. Select "Open" from the context menu
3. Click "Open" in the dialog that appears

This tells macOS to trust the application.

If that doesn't work, remove the quarantine attribute:

```sh
xattr -cr /Applications/Zed.app
```

### CLI command not found

If the `zed` command isn't available after installation:

1. Check that `/usr/local/bin` is in your PATH
2. Check that the `omega` symlink points to the bundled `cli` executable
3. Open a new terminal window to reload your PATH

### Can't install CLI {#cant-install-cli}

Creating a symlink in `/usr/local/bin` may require administrator privileges.
Instead, add an alias pointing to the `cli` binary bundled inside the app:

```sh
# Default install
alias omega="/Applications/Omega.app/Contents/MacOS/cli"

# User install
alias omega="$HOME/Applications/Omega.app/Contents/MacOS/cli"
```

Add the line that matches your install to your shell configuration file. Use `~/.zshrc` for Zsh (the default on modern macOS) or `~/.bashrc` for Bash.

After you restart your shell, you will be able to use `zed` from your terminal:

```sh
zed .              # Open current folder
zed file.txt       # Open a file
```

### GPU or rendering issues

Zed uses Metal for rendering. If you experience graphical glitches:

1. Ensure macOS is up to date
2. Restart your Mac to reset the GPU state
3. Check Activity Monitor for GPU pressure from other apps

### High memory or CPU usage

If Zed uses more resources than expected:

1. Check for runaway language servers in the terminal output ({#action omega::OpenLog})
2. Try disabling extensions one by one to identify conflicts
3. For large projects, consider using [project settings](./reference/all-settings.md#file-scan-exclusions) to exclude unnecessary folders from indexing

For additional help, see the [Troubleshooting guide](./troubleshooting.md) or visit the [Zed Discord](https://discord.gg/zed-community).
