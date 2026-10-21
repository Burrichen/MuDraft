# Syncing MuDraft between computers with GitHub

MuDraft can keep your library the same on two or more computers, such as a Mac and a Windows PC. It does this through a **private GitHub repository that you own**. Syncing only happens when you click **Sync Now** in Settings. There's no background syncing, no MuDraft account, and no MuDraft server.

> **Privacy.** A private repository is visible only to you and to anyone you invite. It is **not end-to-end encrypted**: GitHub can read what's stored in it. MuDraft never asks for your GitHub password.

## What syncs

- **Synced:** albums, editions and tracks, your metadata edits, the Listen List and Collection, listens (dated and undated, including deletions), ratings (a 0-star rating stays different from "unrated"), reviews, liked tracks, tags, Next Up history and the current album, manual catalogue entries, your own artwork (and artwork you removed), and display preferences.
- **Not synced:** downloaded cover art (each computer downloads its own), cached MusicBrainz results, discovered discographies, the window and sidebar state, the artwork download setting, your GitHub token, and file paths.

## 1. Create the repository (once)

1. Sign in at [github.com](https://github.com).
2. Click **+** (top right) → **New repository**.
3. **Repository name:** `MuDraft-Sync`.
4. Choose **Private**.
5. Optionally tick **Add a README file**. MuDraft works either way.
6. Click **Create repository**.

Use this repository only for MuDraft. MuDraft refuses to write to a repository that already holds other files, and it refuses public repositories.

## 2. Create a token for each computer

Each computer gets its own token, and tokens are never copied between computers. On each computer:

1. On GitHub, click your profile picture → **Settings**.
2. In the left sidebar, click **Developer settings** → **Personal access tokens** → **Fine-grained tokens**.
3. Click **Generate new token**.
4. **Token name:** something like `MuDraft – Mac` or `MuDraft – Windows PC`.
5. **Expiration:** your choice. When it expires, MuDraft tells you, and you make a new one.
6. **Resource owner:** your account (the owner of `MuDraft-Sync`).
7. **Repository access:** **Only select repositories**, then choose `MuDraft-Sync`.
8. **Permissions** → **Repository permissions** → **Contents:** **Read and write**. Make sure **Metadata** shows **Read-only**. Leave every other permission at **No access**.
9. Click **Generate token**, then copy it. It starts with `github_pat_`. GitHub shows it only once.

## 3. Connect the first computer

1. In MuDraft, open **Settings → GitHub Sync**.
2. Enter the **Repository owner** (your GitHub user name) and the **Repository name** (`MuDraft-Sync`).
3. Paste the token into **Access token for this computer**.
4. Check the line that says where data will go (`github.com/<you>/MuDraft-Sync`), then click **Connect**.
5. MuDraft stores the token in the computer's secure credential store: **Keychain** on macOS, **Credential Manager** on Windows. It never shows the token again and never writes it to the library, exports, or logs.
6. The **First sync on this computer** window opens. It shows how many albums, Collection entries, listens and other records each side has. Nothing has changed yet.
   - The repository is empty, so choose **Upload this library**.

## 4. Connect the other computer

Repeat steps 2 and 3 on the other computer, with its own token and the same repository. The first-sync window then offers:

| This computer | Choices                                                                                                                             |
| ------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| Empty library | **Download remote library**: copies the synced library here.                                                                        |
| Has a library | **Merge both** (recommended): keeps everything from both. Any record that differs is shown to you to decide.                        |
| Has a library | **Download remote library…**: replaces this computer's library with GitHub's. MuDraft asks you to confirm and saves a backup first. |

MuDraft never replaces an existing library or Collection without that explicit choice.

## Everyday use

Click **Sync Now** whenever you want both sides brought together, for example after listening on one computer and before using the other.

- Independent changes, such as a rating on one computer and a listen on the other, are combined automatically.
- Before applying anything from GitHub, MuDraft backs up this computer's library. It keeps the last five sync backups in the data folder's `backups` folder.
- Success is shown only after GitHub has saved the new state **and** this computer has applied it. A summary then lists what was received and sent (albums, listens, ratings, reviews, tags, artwork).
- **Test Connection** checks the repository and reports changes waiting on GitHub without changing anything.

### Status

| Status          | Meaning                                                                             |
| --------------- | ----------------------------------------------------------------------------------- |
| Not Connected   | No repository or token yet, or the first sync hasn't run.                           |
| Up to Date      | The last sync finished on both sides and nothing is pending.                        |
| Changes Pending | This computer has unsynced changes, or Test Connection found new changes on GitHub. |
| Syncing         | A sync is running. Progress is shown below the status.                              |
| Conflicts       | The last sync found records changed differently on both sides. Nothing changed yet. |
| Offline / Error | The last sync didn't finish. Your changes are safe. The message says what to do.    |

The status is worked out again whenever MuDraft opens, so pending changes and unresolved conflicts survive a restart.

### Conflicts

If the same thing was changed differently on both computers (the same album title, review, rating, tag, or list), MuDraft doesn't guess. It lists each record and field with both values, and you choose **Keep this computer's version** or **Keep GitHub's version**. You can also use **Keep all from this computer** or **Keep all from GitHub**.

- **Cancel** leaves both sides exactly as they were. Your other changes stay pending, and choices you've already made are remembered until you finish.
- **Apply choices and sync** becomes available once every conflict has a choice.

## When something goes wrong

Your library is never deleted or reset because a sync failed.

| Message                               | What to do                                                                                                                                     |
| ------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| GitHub didn't accept a token          | The token expired or was revoked. Create a new one (step 2), then use **Enter a new token** or **Replace token…**.                             |
| Repository not available              | Check the owner and name spelling, that the repository exists, and that this computer's token has access to it with Contents "Read and write". |
| The repository is public              | On GitHub, open the repository → **Settings** → change visibility to **Private**, then sync again.                                             |
| Not a MuDraft repository              | The repository holds other files. Create a new private repository just for MuDraft.                                                            |
| Made by a newer MuDraft               | Another computer has a newer MuDraft. Update MuDraft on this computer.                                                                         |
| Can't reach GitHub                    | You're offline or GitHub is unreachable. Try again later. Changes stay pending.                                                                |
| GitHub is limiting requests           | Wait a few minutes and try again.                                                                                                              |
| Not enough free disk space            | Free some space on the drive that holds MuDraft's data, then try again. Nothing was changed.                                                   |
| Couldn't back up first                | MuDraft always backs up before applying changes, so nothing was changed. Check the data folder has space and is writable.                      |
| Couldn't apply the changes here       | This computer's library wasn't changed. Sync again to finish. The message names the backup taken beforehand.                                   |
| The repository was reset              | The repository now holds different sync data. **Disconnect**, connect again, and choose how to combine the libraries.                          |
| Sync data on GitHub failed its checks | Files in the repository were edited by hand or damaged. Nothing was changed here. Use GitHub's history to restore them.                        |

To return to an earlier state, use **Settings → Profile → Restore from a profile…** with an exported profile. Sync backups (`mudraft-…-pre-sync.sqlite3`) are full database copies in the data folder's `backups` folder.

## Removing sync from a computer

- **Remove token from this computer** deletes the token from Keychain or Credential Manager. To revoke it completely, delete it on GitHub: **Settings → Developer settings → Fine-grained tokens**.
- **Disconnect…** stops this computer syncing. Your library here and the data on GitHub stay as they are, and you can choose to remove the token at the same time. If you connect again later, MuDraft asks how to combine the libraries.
