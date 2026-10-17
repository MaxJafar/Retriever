# Updates

Open **Settings → Updates** (or **Check for Updates** in the Mac app menu).
Automatic checks are enabled by default. When a newer compatible release is
published, **Update available** appears in the existing status bar. Choose
**Download update**, then install when current work is finished. Downloads can
be cancelled; reminders and skipped versions are saved locally. Preview releases
are enabled initially on alpha builds and can be turned off.

Windows opens the verified installer and quits Retriever after its local engine
checkpoints and stops. The installer creates a versioned app directory and
updates Start-menu shortcuts. Select **Open Retriever** at the end. Older app
directories are retained for rollback; application data is outside them.
If Retriever already owns folder integration, the installer retargets that
recorded handler while retaining its original restore values. Newer choices
made by another app are preserved.

Mac opens the verified disk image after its local engine checkpoints and stops.
Drag Retriever into Applications, replace the previous app and reopen it. This is
a guided native installation, not an unattended bundle replacement. Shared-index
clients never stop another computer's coordinator.

App updates do not reset the library, metadata, model bundle or cache. Existing
custom library/shared-index settings are persisted outside the application in
`Retriever Candidate/installation.json`; that historical folder name is retained
for compatibility. Finish file operations and pause indexing before installing.
Busy engines refuse shutdown rather than interrupting a transfer or scan.

## Release contract

`release/repository.json` selects one public GitHub repository. Checks use its
[Releases API](https://docs.github.com/en/rest/releases/releases), without an
account token, library contents, search queries or device identifiers. GitHub
receives the normal request IP and a Retriever version user agent. No check runs
when automatic checking is disabled. A manual check contacts GitHub explicitly.

- Automatic network checks are spaced six hours apart; ETags avoid redownloading
  unchanged release metadata. Rate-limit responses are respected even manually.
- Drafts, older/equal versions and disabled previews are excluded. Historical
  `alpha.N-native` versions compare by numeric alpha component.
- Expected assets are `Retriever-VERSION-windows-x64-setup.exe` and
  `Retriever-VERSION-macos-arm64.dmg`, with a tag of `vVERSION`.
- Each asset must have GitHub's SHA-256 `digest`, exact size and an HTTPS URL in
  the configured repository. Release metadata is fetched again before download.
- Download redirects are limited to GitHub's release asset hosts. Partial files
  remain temporary. Size and SHA-256 are checked before committing a download and
  again before opening it. A published repository is the trust boundary; protect
  maintainer access and sign release binaries for OS-level publisher validation.
- The shutdown route requires both the library API token and a separate local
  update token, rejects active work, then checkpoints SQLite. Shared pairing
  credentials alone cannot stop the service. Errors leave the old app available.

The first upgrade from a build without this protocol requires a manual restart.
Preserve its custom `coordinator.json` as `installation.json` in the parent of
the normal state directory before replacing that older custom deployment.
Never merge, rename or reset the library merely to install an application update.
