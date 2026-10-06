# Releasing

Releases come from git tags. Nothing in `Cargo.toml` is bumped by hand.

```sh
git tag v0.2.0-rc.1      # a `-` makes it a prerelease
git push origin v0.2.0-rc.1
```

The push runs `.github/workflows/release.yml`. It builds:

| File | What |
| --- | --- |
| `graphing-<v>-linux-x86_64.tar.gz` | The binary, built on Ubuntu 22.04 so older glibc runs it |
| `graphing-<v>-windows-x64.zip` | `graphing.exe`, portable |
| `graphing-<v>-macos-universal.zip` | `graphing.app` for Apple silicon and Intel |
| `SHA256SUMS.txt` | Checksums of the above |

Then it publishes a GitHub release titled `graphing <v>`, with notes
generated from the commits since the last tag.

## Where the version comes from

`crates/graphing-app/build.rs` sets `GRAPHING_VERSION`, shown by
`graphing --version` and in Settings:

1. `GRAPHING_VERSION` from the environment (the release workflow sets it
   from the tag);
2. else `git describe` against the nearest `v*` tag, so a local build
   says `0.2.0-rc.1`, or `0.2.0-rc.1-4-gabc1234` four commits later;
3. else Cargo's version with `-dev` (a source tarball without git).

## Dry runs

Actions > Release > Run workflow builds every file as artifacts for a
version you type and publishes nothing.

## Signing

Nothing is signed yet; betas do not need it.

- **Windows:** unsigned builds show a SmartScreen warning ("More info,
  Run anyway").
- **macOS:** users right-click the app and choose Open the first time.

The macOS job signs and notarizes as soon as these repository secrets
exist, with no workflow change:

| Secret | What |
| --- | --- |
| `APPLE_CERTIFICATE` | Developer ID Application certificate, `.p12`, base64 |
| `APPLE_CERTIFICATE_PASSWORD` | Its password |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Name (TEAMID)` |
| `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | For notarization (an app-specific password) |

## Windows console

Release builds are windowed apps, so opening graphing shows no console.
From a terminal, the CLI commands (`graphing render ...`) attach to that
terminal's console and print as usual.
