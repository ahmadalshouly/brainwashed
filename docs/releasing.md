# Releasing

## Making a release

1. Pick the version and set it everywhere:
   ```sh
   node scripts/set-version.mjs 0.1.0
   cargo check -p brainwashed-host   # refreshes Cargo.lock
   ```
2. Commit (`Release 0.1.0`), merge to `main`, then tag and push:
   ```sh
   git tag v0.1.0 && git push origin v0.1.0
   ```
3. The **Release** workflow builds installers for macOS (Apple silicon and Intel), Windows and Linux, attaches them to a **draft** GitHub release, and pushes the relay image to `ghcr.io/ahmadalshouly/brainwashed-relay:<version>` and `:latest`. Versions starting with `0.` are marked as pre-releases.
4. Check the draft's installers on each system, edit the notes, and click **Publish**. Running apps see the new version the next time they start.

To test the pipeline without releasing, run the Release workflow by hand (Actions > Release > Run workflow). It builds everything and keeps the installers as workflow artifacts.

## Signing

Unsigned installers work, but macOS and Windows warn users first (see [install.md](install.md)). Signing turns on by itself when the secrets exist (Settings > Secrets and variables > Actions):

**macOS** (needs an Apple Developer account, $99 a year):

| Secret | Value |
|---|---|
| `APPLE_CERTIFICATE` | Your "Developer ID Application" certificate exported as `.p12`, base64-encoded |
| `APPLE_CERTIFICATE_PASSWORD` | The `.p12` export password |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Your Name (TEAMID)` |
| `APPLE_ID` | Your Apple ID email, for notarization |
| `APPLE_PASSWORD` | An app-specific password for that Apple ID |
| `APPLE_TEAM_ID` | Your 10-character team ID |

**Windows:** an OV/EV code signing certificate, or Azure Trusted Signing. Tauri's [Windows signing guide](https://v2.tauri.app/distribute/sign/windows/) covers both; add the `certificateThumbprint` or `signCommand` to `bundle.windows` in `apps/host/src-tauri/tauri.conf.json`.

## In-app updates

Today the app only tells users a new version exists and links to the release. Installing updates from inside the app (Tauri's updater plugin) needs an update signing key:

1. `pnpm --filter @brainwashed/host tauri signer generate -w ~/.tauri/brainwashed.key`
2. Add the private key and its password as the `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secrets, and keep a backup: losing it means users must reinstall by hand.
3. Add `tauri-plugin-updater` with the public key and the endpoint `https://github.com/ahmadalshouly/brainwashed/releases/latest/download/latest.json`, and set `bundle.createUpdaterArtifacts` to `true`. tauri-action then uploads `latest.json` with each release.

## Relay

Releases publish the relay image. To run it, see [relay.md](relay.md).
