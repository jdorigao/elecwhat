# ElecWhat - Simple desktop WhatsApp client for Linux

<p align="center">
<img width="400" alt="elecwhat" src="https://github.com/user-attachments/assets/3a352d5b-04a8-4e90-a063-330b706d096c" />
</p>

> **Fork / license notice:** This repository is a fork of [piec/elecwhat](https://github.com/piec/elecwhat), originally by Pierre Carru. The Tauri port is a modification made in this fork (Juliano Dorigão). The project remains licensed under [GPL-3.0-or-later](LICENSE).

## Features

- Desktop notifications
- Calls / Videos calls working (you need to join WhatsApp Web Beta)
- Tray icon with unread count (aka AppIndicator), [custom tray icons](https://github.com/piec/elecwhat/issues/30#issuecomment-3470456738).
- Custom CSS
- Spellcheck
- CLI & D-Bus interface to show/hide/toggle window
- Dock unread count badge
- Preserve window position
- Custom user scripts in `~/.config/elecwhat/user-scripts/*.js`
- Custom tray icons in `~/.config/elecwhat/user-icons/*.png` by default
- Supports Wayland or X11

## Why?

I created this client after due to recurrent stability issues with other linux clients.
Also I just wanted a simple and stable client and thought it could be done without much code.

## Building from source (Tauri)

Requirements: Rust 1.90+, the Tauri Linux dependencies (`webkit2gtk-4.1`,
`gtk+-3.0`, `libayatana-appindicator3`) and `cargo-tauri`.

```sh
cargo tauri dev      # development build
cargo tauri build    # deb, rpm and AppImage
```

The Tauri build keeps the same app id (`fr.carru.elecwhat`) and reuses the
existing `~/.config/elecwhat/config.json` (JSONC), `persistent-state.json` and
`user-*` directories.

## Install

No Tauri packages have been published yet. Build them locally:

```sh
cargo tauri build
```

Bundles are written to `src-tauri/target/release/bundle/` (`.deb`, `.rpm`,
AppImage). See [Building from source](#building-from-source-tauri) for the
required dependencies. Published releases on the upstream repository predate
this Tauri port.

## Configuration

`~/.config/elecwhat/config.json` (or `$XDG_CONFIG_HOME/elecwhat/config.json`
when `XDG_CONFIG_HOME` is set):

```
{
  "log-level": "info",
  "notification-prefix": "elecwhat - ",

  "quit-on-close": false,
  "show-at-startup": false,

  "dbus": true,

  "menu-bar": true,
  "menu-bar-auto-hide": true,

  "esc-toggle-window": false,

  "css": [
    "span { font-size: small !important; }",
    "https://gist.githubusercontent.com/piec/3f9a198a7694f1df37895ee3faee9b6e/raw/whatsapp-test.css",
    "file://~/mysheet.css"
  ],

  "spellcheck": true,
  "spellcheck-languages": ["en-US", "fr"],

  "user-agent": "...",
  "open-dev-tools": false,

  "scripts": [
    # ~/.config/elecwhat/user-scripts/myscript.js
    "${userData}/user-scripts/myscripts.js"
  ],
  "icons-directory": "..."
}
```

### Tauri migration notes

- The `keys` map from the old client is no longer used. WhatsApp's own shortcuts apply, along with the Escape/Alt window and menu toggles configured above.
- User scripts run in the page context. The old `window.ew` bridge and the `state_get` command were removed, so scripts must not call them.
- The WebKitGTK webview uses its own profile, so the first Tauri run requires signing in again.
- Window geometry is stored in `~/.config/elecwhat/.window-state.json`; the old `window-bounds` entry in `persistent-state.json` is not imported. Tray toggles are saved in `persistent-state.json`.
- `app.security.csp` is `null` in `src-tauri/tauri.conf.json`. The webview loads the external `https://web.whatsapp.com` page, so a local CSP would not apply to it; origin restrictions are enforced in Rust by `on_navigation`/`on_new_window` and by the minimal remote capability instead.
- The optional `url` config key may only change the path/query on the `https://web.whatsapp.com` origin. Other origins fall back to the default because the remote IPC capability is compiled for that host.

## Custom icons

The naming scheme is based on WhatsApp Web favicon urls:

- `~/.config/elecwhat/user-icons/app.png` during startup
- `~/.config/elecwhat/user-icons/favicon.png` when there is no unread
- `~/.config/elecwhat/user-icons/f01.png` where fXX.png is the number of unread

## Not planed

- Multi account
- Complex features
- Other OS support, official apps already exist.
