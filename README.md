# eayar — desktop app

A small window around https://app.eayar.com (Tauri v2) that also prints straight to the shop's
printers. This repository only builds its signed installers: pushing a tag `desktop-v<version>`
runs `.github/workflows/release.yml`. It holds no data and no secrets (the updater's signing key
lives in the repository's Actions secrets).
