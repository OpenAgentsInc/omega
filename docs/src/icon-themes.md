---
title: Icon Themes
description: "Zed comes with a built-in icon theme, with more icon themes available as extensions."
---

# Icon Themes

Zed comes with a built-in icon theme, with more icon themes available as extensions.

## Selecting an Icon Theme

Choose an installed icon theme through the Settings Editor or the
`icon_theme` value in your settings file.

## Installing more Icon Themes

Omega's single-experience product does not ship an extensions browser.

## Configuring Icon Themes

Your selected icon theme is stored in your settings file.
You can open your settings file from the command palette with {#action omega::OpenSettingsFile} (bound to {#kb omega::OpenSettingsFile}).

Just like with themes, Zed allows for configuring different icon themes for light and dark mode.
You can set the mode to `"light"` or `"dark"` to ignore the current system mode.

```json [settings]
{
  "icon_theme": {
    "mode": "system",
    "light": "Light Icon Theme",
    "dark": "Dark Icon Theme"
  }
}
```

## Icon Theme Development

See: [Developing Zed Icon Themes](./extensions/icon-themes.md)
