# MuDraft on Windows (x64)

## Installing

Download `MuDraft_<version>_windows_x64_setup.exe` from the GitHub release and run it. You don't need Node, Rust, SQLite, a terminal, or this repository.

- **Per-user install.** No administrator rights are needed. The app goes into `%LOCALAPPDATA%\MuDraft`, with a Start menu and a desktop shortcut.
- **Works offline.** The installer contains Microsoft's offline WebView2 runtime installer, which adds about 127 MB. If WebView2 is missing (rare on Windows 10/11), it is installed without a network connection. If WebView2 is already present, that step is skipped.
- **Unsigned personal release.** Unless a release says it is signed, the installer has no Authenticode signature. Windows SmartScreen or antivirus software may warn about it. MuDraft can't promise that no warnings appear. Check the file against the release's `SHA256SUMS.txt`.

## Your data

- Your profile, settings, artwork cache, and automatic backups are all in `%LOCALAPPDATA%\app.mudraft.desktop`.
- **Upgrading:** run the newer installer. Your profile is kept.
- **Uninstalling** (Settings → Apps, or `uninstall.exe`) removes the app but keeps your profile.
  - Your data is deleted only if you tick **Delete the application data** in the uninstaller.
  - Export your profile first (Settings → Profile) if you might want it back.

## Signing (maintainers)

The `Windows release` workflow signs the installer only when you provide your own code-signing certificate. Nothing secret is stored in the repository.

- `WINDOWS_CERTIFICATE`: a secret containing the `.pfx` file, base64-encoded.
- `WINDOWS_CERTIFICATE_PASSWORD`: a secret containing the `.pfx` password.
- `WINDOWS_TIMESTAMP_URL`: an optional repository variable. Defaults to a free public RFC 3161 timestamp server.

Without these secrets the build is an unsigned personal release, and the release title says so.

## How each build is checked

The installer is built on a `windows-latest` runner. `scripts/windows/verify-install.ps1` then checks on that runner:

- the silent install;
- the shortcuts and the Add/Remove Programs entry;
- that no fixtures or test hooks are installed;
- that the production data folder is used and the development override is ignored;
- that the profile survives a restart and an upgrade from an older installer;
- that uninstalling keeps the profile.

The offline WebView2 install path is **not** exercised in CI, because the runner already has WebView2.
