# Release acceptance: two real computers

Run this checklist with the **draft release's own files** on a real Mac and a real Windows PC before publishing. It covers what CI can't check: downloaded-file prompts (SmartScreen, Gatekeeper), the token prompt in Settings, and syncing through real GitHub.

The same sequence also runs automatically, against a mocked GitHub with two separate profiles (`release_acceptance_two_devices` in `src-tauri/src/sync/tests/journeys.rs`). That test doesn't replace this checklist.

## Setup

- [ ] A **test** private repository (for example `MuDraft-Sync-Acceptance`), not your everyday sync repository. Initialise it with a README.
- [ ] Two fine-grained tokens, one per computer, each limited to that repository with Contents set to Read and write.
- [ ] On each computer, either export your real profile first (Settings → Profile), or use a separate user account.

## 1. Install

| Check                                                                                   | Mac | Windows |
| --------------------------------------------------------------------------------------- | --- | ------- |
| Download from the draft release; the checksum matches `SHA256SUMS.txt`                  | ☐   | ☐       |
| Install (drag to Applications / run the setup)                                          | ☐   | ☐       |
| Unsigned prompt handled as documented (Open Anyway / More info → Run anyway)            | ☐   | ☐       |
| First launch from Finder / the Start menu; the MuDraft icon shows in the Dock / taskbar | ☐   | ☐       |
| Charcoal/grey UI with blue accents; Settings → Storage shows the production data folder | ☐   | ☐       |
| Add an album; artwork downloads after consent                                           | ☐   | ☐       |
| Quit and reopen: the album, rating, and artwork are still there                         | ☐   | ☐       |
| Upgrade: installing over the previous version keeps the profile (Windows: from 1.1.0)   | n/a | ☐       |

## 2. Connect

| Check                                                                                                         | Mac | Windows |
| ------------------------------------------------------------------------------------------------------------- | --- | ------- |
| Settings → GitHub Sync: enter owner/repo and this computer's token → Connect                                  | ☐   | ☐       |
| The token is never shown again; status reads "Not Connected" until the first sync                             | ☐   | ☐       |
| Mac: first sync → **Upload this library**                                                                     | ☐   |         |
| Windows: first sync preview shows the counts → **Download remote library** (or **Merge both** if it has data) |     | ☐       |
| The token is in Keychain ("MuDraft GitHub sync") / Credential Manager, and not in the profile export          | ☐   | ☐       |

## 3. Scenario

1. [ ] **Mac:** log a listen, rate two tracks (one of them **0 stars**), and add a tag to an album. Then **Sync Now**. The summary lists 1 listen, 2 ratings, and tag changes.
2. [ ] **Windows:** **Sync Now**. The same listen, both ratings (the zero still shows as 0 stars, not unrated), and the tag appear.
3. [ ] **Windows:** add a different album and write a review. Then **Sync Now**.
4. [ ] **Mac:** **Sync Now**. The album and review arrive.
5. [ ] **Offline conflict:** turn Wi-Fi off on both computers. Edit the **same** album's review differently on each, then turn Wi-Fi back on.
6. [ ] **Mac:** **Sync Now**. It succeeds.
7. [ ] **Windows:** **Sync Now**. The conflict dialog shows the record, the field, and both versions. **Cancel** changes nothing. Then choose a version and click **Apply choices and sync**.
8. [ ] **Mac:** **Sync Now**. The chosen review is shown on both computers.
9. [ ] **Interruption:** turn Wi-Fi off and click **Sync Now**. The app shows "Can't reach GitHub", and the library is unchanged and still marked pending.
10. [ ] **Bad token:** revoke one computer's token on GitHub, then **Sync Now**. The app shows "GitHub didn't accept a token", and the library is intact. Add a new token and sync.
11. [ ] **Recovery path:** export the profile on one computer (Settings → Profile). Restore it on the other with **Restore from a profile…**. The library matches.

## Record

| Item                                 | Result |
| ------------------------------------ | ------ |
| Release version and commit           |        |
| Mac model, chip, macOS version       |        |
| Windows edition, version, and CPU    |        |
| Steps that failed (with screenshots) |        |
| Date and tester                      |        |
