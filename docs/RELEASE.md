# Releases: downloading and producing them

## Where to download

Every MuDraft release is one GitHub Release in the **source** repository, [Burrichen/MuDraft](https://github.com/Burrichen/MuDraft/releases). It is not in your separate `MuDraft-Sync` data repository. Each release has these assets:

| Asset                                     | For                                                                           |
| ----------------------------------------- | ----------------------------------------------------------------------------- |
| `MuDraft_<version>_windows_x64_setup.exe` | Windows 10/11, x64. Per-user installer; works offline. See `docs/WINDOWS.md`. |
| `MuDraft_<version>_macos_universal.dmg`   | macOS 12+, Apple Silicon and Intel. See `docs/MACOS.md`.                      |
| `SHA256SUMS.txt`                          | SHA-256 checksums of both files.                                              |

From 1.4.0 the release also carries `latest.json` and `MuDraft_<version>_macos_universal.app.tar.gz`. These are for in-app updates, not for downloading by hand (see below).

- The newest published release is always at <https://github.com/Burrichen/MuDraft/releases/latest>.
- Once a version is published, its files have stable links: `https://github.com/Burrichen/MuDraft/releases/download/v<version>/<file>`.
- A _draft_ release is visible only to people with write access to the repository, and its links work only after publishing.

### Checking a download

Compare the result with the matching line in `SHA256SUMS.txt`:

- **Windows (PowerShell):** `Get-FileHash .\MuDraft_<version>_windows_x64_setup.exe -Algorithm SHA256`
- **macOS (Terminal):** `shasum -a 256 MuDraft_<version>_macos_universal.dmg`

### Unsigned builds

Signing is optional for this personal project, and each release's notes state exactly what was signed:

- **Windows, unsigned:** SmartScreen may show "Windows protected your PC". Choose **More info → Run anyway**, but only if the checksum matches.
- **macOS, ad-hoc signed and not notarized:** approve MuDraft once in System Settings → Privacy & Security → **Open Anyway** (`docs/MACOS.md`). Never disable Gatekeeper globally.

## In-app updates

From 1.4.0, MuDraft checks <https://github.com/Burrichen/MuDraft/releases/latest/download/latest.json> when it starts. **Settings → Version → Check for Updates** checks again. A failed check (offline, GitHub down) stays silent at startup.

When a newer version exists, MuDraft shows its release notes and offers **Update and Restart**:

- **Windows:** it runs that version's setup `.exe` in passive mode.
- **macOS:** it replaces `MuDraft.app` with the `.app.tar.gz`, then relaunches.

Before installing, the Rust side checks each download against the public key in `src-tauri/tauri.conf.json`. A file that doesn't match is refused. The library is backed up as usual before the new version migrates it. Versions before 1.4.0 have no updater, so install 1.4.0 by hand once.

The private signing key is the GitHub secret `TAURI_SIGNING_PRIVATE_KEY`, with an empty `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Keep an offline backup of it. If it's lost, installed copies can no longer update themselves, and a new key needs one more manual install.

## Producing a release (maintainers)

The `Release` workflow (`.github/workflows/release.yml`) builds both installers from the **same commit** and puts them in **one draft** release. It never publishes anything.

1. **Pick the version.** Set the same `MAJOR.MINOR.PATCH` in all of these:
   - `package.json` and the root entries of `package-lock.json`
   - `src-tauri/tauri.conf.json`
   - `src-tauri/Cargo.toml`, and the `mudraft` entry of `src-tauri/Cargo.lock`
2. **Write `docs/release-notes/v<version>.md`.** The workflow appends downloads, checksums, and signing status. `node scripts/release/check-version.mjs` checks steps 1 and 2.
3. **Run the checks locally:** `npm run check`. Run `npm run e2e:build && npm run e2e` on a Mac.
4. **Push a branch named `release/v<version>`.** This starts the workflow (it can also be run by hand from the Actions tab). The workflow then:
   1. checks the version metadata;
   2. runs `npm run check` and the real Keychain / Credential Manager test on Windows and macOS;
   3. runs the live GitHub sync test, if configured (see below);
   4. builds the Windows installer and checks it on a Windows runner with `scripts/windows/verify-install.ps1`, including an upgrade from the latest published release;
   5. builds the universal DMG (`--target universal-apple-darwin`) and checks it with `scripts/macos/verify-install.sh` on an Apple Silicon runner and an Intel runner;
   6. only if everything passed, creates or updates the draft release `v<version>` with both files, `SHA256SUMS.txt`, and the in-app update files (`latest.json` and the macOS `.app.tar.gz`). The notes in `latest.json` are `docs/release-notes/v<version>.md`, which is what the update prompt shows, so keep that file short.

   If any step fails, no draft is created or updated.

5. **Accept it on real computers.** Download the draft's assets (`gh release download v<version> --repo Burrichen/MuDraft`), then work through `docs/RELEASE_ACCEPTANCE.md` on a real Mac and a real Windows PC.
6. **Merge and publish.** Merge the release branch into `main`. Then open the draft on GitHub and click **Publish release**. Publishing creates the tag `v<version>` at the commit that was built and tested. Never reuse a published version number: the workflow refuses to change a published release.

Each later version repeats these steps with a higher version number.

## Optional secrets and variables

Nothing secret is stored in the repository. `TAURI_SIGNING_PRIVATE_KEY` is **required**: the Windows and macOS build jobs fail without it (see In-app updates). Everything else here is optional.

| Name                                                                        | Kind     | Purpose                                                                                                                                   |
| --------------------------------------------------------------------------- | -------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`           | secret   | **Required.** Signs the in-app update files (the minisign private key and its password; the password may be empty).                       |
| `WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD`                       | secret   | Authenticode-sign the installer (base64 `.pfx` and its password).                                                                         |
| `WINDOWS_TIMESTAMP_URL`                                                     | variable | Timestamp server (default: DigiCert's public one).                                                                                        |
| `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY` | secret   | Sign with a "Developer ID Application" certificate (base64 `.p12`, its password, and the identity name).                                  |
| `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`                               | secret   | Notarize the signed app (`APPLE_PASSWORD` is an app-specific password). Set all six or none: with only some, the DMG stays ad-hoc signed. |
| `MUDRAFT_SYNC_TEST_OWNER`, `MUDRAFT_SYNC_TEST_REPO`                         | variable | A dedicated, empty, **private** test repository for the live sync test.                                                                   |
| `MUDRAFT_SYNC_TEST_TOKEN`                                                   | secret   | A fine-grained token for that test repository only, with Contents set to Read and write.                                                  |

Notes:

- The Developer ID path follows Tauri's documented variables but hasn't been exercised yet, because no certificate is configured.
- This source repository is public, so its Actions logs are public. Use a throwaway test repository and token for the live test. **Never** use your personal `MuDraft-Sync` repository or your own token. Secrets are masked in logs, and the tests never print them.
