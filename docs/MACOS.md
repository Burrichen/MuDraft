# MuDraft on macOS

## Requirements

macOS 12 Monterey or later, on Apple Silicon (M1 or later) or Intel. There is one download for both: the app inside is a universal binary.

## Installing

1. Open the MuDraft release on GitHub (see `docs/RELEASE.md`) and download `MuDraft_<version>_macos_universal.dmg`. Also download `SHA256SUMS.txt`.
2. Optional, recommended: check the download. In Terminal, run `shasum -a 256 ~/Downloads/MuDraft_<version>_macos_universal.dmg` and compare the result with the line for that file in `SHA256SUMS.txt`.
3. Double-click the `.dmg`. In the window that opens, drag **MuDraft** onto **Applications**.
4. Eject the disk image (in Finder's sidebar, or drag it to the Trash).

## First launch: approving an app that isn't notarized

Unless a release says it is signed with a Developer ID and notarized, MuDraft is only _ad-hoc signed_. Apple hasn't checked it, so macOS blocks the first launch. Approve it once, for MuDraft only:

1. Open MuDraft from Applications. macOS says it can't verify MuDraft (for example "“MuDraft” Not Opened"). Click **Done**. Don't click **Move to Trash**.
2. Open **System Settings → Privacy & Security** and scroll to **Security**.
3. Next to the message about MuDraft being blocked, click **Open Anyway**. Confirm with your password or Touch ID.
4. Click **Open** in the dialog that appears.

On macOS 14 Sonoma and earlier, you can instead Control-click MuDraft in Applications, choose **Open**, then **Open** again.

After that, MuDraft opens normally. Do this only for a file whose checksum matches the release. **Never turn Gatekeeper off for the whole Mac**, for example with `spctl --master-disable`. Approving MuDraft once is all it needs.

## Your data

- Your profile, settings, artwork cache, and automatic backups are in `~/Library/Application Support/app.mudraft.desktop`.
- A GitHub Sync token, if you add one, is in your login keychain under **MuDraft GitHub sync**. It is never written to the profile or exports.
- **Upgrading:** drag the new MuDraft onto Applications and choose **Replace**. Your profile is kept.
- **Uninstalling:** move MuDraft from Applications to the Trash. Your profile is kept. To remove it too, delete the folder above, after exporting your profile (Settings → Profile) if you might want it back.

## How each build is checked

The release workflow builds the `.dmg` on a GitHub-hosted Mac (`--target universal-apple-darwin`). `scripts/macos/verify-install.sh` then checks the same file on an **Apple Silicon** runner and an **Intel** runner:

- the disk image checksum, and that it contains MuDraft.app and an Applications shortcut;
- the bundle identifier, version, minimum macOS, and icon;
- that the executable contains both arm64 and x86_64;
- the code signature (ad-hoc, or Developer ID when configured), and Gatekeeper's verdict;
- that no test hooks or fixtures are included;
- the first launch through LaunchServices, as from Finder, and that the production data folder is used;
- that the profile survives a restart and an upgrade;
- on Apple Silicon, the Intel slice running under Rosetta;
- that removing the app keeps the profile.

A downloaded copy carries macOS's quarantine flag, so the approval prompt above can only be checked by hand. See `docs/RELEASE_ACCEPTANCE.md`.
