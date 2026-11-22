/**
 * The Android build (`npm run web:build:android`) sets this. The desktop builds never do,
 * so everything gated on it behaves exactly as before.
 */
export const IS_ANDROID = import.meta.env.VITE_MUDRAFT_TARGET === "android";

/** The APK ships without the Stats section to keep the app light. */
export const STATS_ENABLED = !IS_ANDROID;

/** What this machine is called in the interface: "this computer" or "this device". */
export const DEVICE = IS_ANDROID ? "device" : "computer";
export const DEVICES = IS_ANDROID ? "devices" : "computers";

/** Where the GitHub token is kept, as named in the interface. */
export const SECURE_STORE = IS_ANDROID
  ? "Android Keystore"
  : "Keychain on macOS, Credential Manager on Windows";
