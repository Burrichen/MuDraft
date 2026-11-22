# MuDraft for Android (APK)

The `apk` branch adds an Android build. The Windows and macOS installers are unchanged.

## What differs on Android

- **No Stats section.** The APK is built with `VITE_MUDRAFT_TARGET=android` (`.env.android`), so
  Stats is not in the bundle. Liked takes its nav slot. A stored start page or last page of
  Stats (e.g. from a synced profile) opens Listen List instead.
- **Files** (CSV import and template, profile export/restore, custom artwork) go through the
  Android document picker. Rust copies a picked file to a private temp file (or writes a temp
  file and copies it out), so the rest of the code only sees paths (`commands/files.rs`).
  The picked file's name is shown only when the provider reveals it.
- **GitHub token** is kept in the Android Keystore (`android-native-keyring-store`).
- **TLS** uses bundled root certificates (`src/net.rs`): reqwest's platform verifier panics on
  Android without Java-side setup.
- **Updates:** the Tauri updater has no Android support. Settings → Check for Updates reads the
  same `latest.json` and opens `MuDraft_<version>_android.apk` from that release in the
  browser; the user opens the download to install it over the old version.
- **Layout:** content sits inside the system-bar insets, the keyboard lifts the page, and the
  Back button walks back through pages (`MainActivity.kt`).

## Build

JDK 21 (Gradle does not support newer), Android SDK with NDK 29, and the Android Rust targets
(`rustup target add aarch64-linux-android armv7-linux-androideabi`).

```sh
export JAVA_HOME=<jdk 21> ANDROID_HOME=~/Library/Android/sdk NDK_HOME=$ANDROID_HOME/ndk/29.0.14206865
npm ci
npm run android:build -- --target aarch64   # most phones; omit --target for all four ABIs
```

Output: `src-tauri/gen/android/app/build/outputs/apk/universal/release/`. Release APKs are signed
when `src-tauri/gen/android/keystore.properties` exists (`storeFile`, `storePassword`,
`keyAlias`, `keyPassword`; git-ignored, never commit it); otherwise they are unsigned and not
installable. `npm run android:dev` runs on a connected device or emulator.

## Release signing key

An APK can only update an installed one signed with the **same key**. Create it once and keep a
backup: `keytool -genkeypair -keystore mudraft.jks -alias mudraft -keyalg RSA -keysize 2048
-validity 10000`. The Release workflow builds and attaches `MuDraft_<version>_android.apk` only
when these repository secrets exist: `ANDROID_KEYSTORE_BASE64` (`base64 -i mudraft.jks`),
`ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`. Without them the
release is Windows and macOS only.
