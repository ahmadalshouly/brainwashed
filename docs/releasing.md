# Releasing

## Making a release

1. Pick the version and set it everywhere:
   ```sh
   node scripts/set-version.mjs 0.1.0
   cargo check -p brainwashed-cli   # refreshes Cargo.lock
   ```
2. Commit (`Release 0.1.0`) and merge to `main`. Then either publish a release on GitHub with the tag `v0.1.0` (which creates the tag), or push the tag yourself:
   ```sh
   git tag v0.1.0 && git push origin v0.1.0
   ```
3. The **Release** workflow builds `brainwashed` for macOS (Apple silicon and Intel), Windows and Linux (x64 and Arm), attaches the archives to the release (making a **draft** first if you pushed only a tag), and pushes the relay image to `ghcr.io/ahmadalshouly/brainwashed-relay:<version>` and `:latest`. Versions starting with `0.` are marked as pre-releases.
4. Publish the draft if there is one. The install scripts pick up the newest published release, and running hosts show the new version on the admin page.

To test the pipeline without releasing, run the Release workflow by hand (Actions > Release > Run workflow). It builds everything and keeps the archives as workflow artifacts.

## Signing

The `brainwashed` binaries aren't signed. The install scripts download them with `curl` (which doesn't mark files as downloaded from the internet) and PowerShell (`install.ps1` clears that mark), so neither macOS Gatekeeper nor Windows SmartScreen blocks them. Signing (Apple Developer ID, an Authenticode certificate or Azure Trusted Signing) can be added to the `cli` job later if people download the archives by hand.

## Relay

Releases publish the relay image. To run it, see [relay.md](relay.md).
