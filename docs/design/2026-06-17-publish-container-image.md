# Design Document: Publish whitespace Container Image to GHCR

**Author:** Scott Idler
**Date:** 2026-06-17
**Status:** Phase 1 implemented (Dockerfile + workflow); Phases 2-4 are release-ops pending
**Review Passes Completed:** 5/5 (+ external review: architect / staff-engineer)

## Summary

`tatari-tv/base-images` wants to consume the `whitespace` binary the same way it
consumes `otto`: by `COPY --from`ing it out of a published multi-arch container
image. whitespace currently publishes only release tarballs, so a
`FROM ghcr.io/tatari-tv/whitespace:...` reference fails. This doc describes adding
a `Dockerfile` and a `docker` release job that mirror `otto-rs/otto`, so each
tagged release also pushes a multi-arch image to `ghcr.io/tatari-tv/whitespace`.

## Problem Statement

### Background

base-images builds its `python` and `rust` toolchain images by pulling build-time
CLIs out of pre-published images rather than reinstalling them. For otto:

```dockerfile
FROM ghcr.io/otto-rs/otto:${OTTO_VERSION} AS otto
...
COPY --from=otto /usr/local/bin/otto /usr/local/bin/
```

base-images already stages the whitespace equivalent in both
`python/Dockerfile` and `rust/Dockerfile`:

```dockerfile
ARG WHITESPACE_VERSION="0.1.6"
FROM ghcr.io/tatari-tv/whitespace:${WHITESPACE_VERSION} AS whitespace
...
COPY --from=whitespace /usr/local/bin/whitespace /usr/local/bin/
```

These references currently break: no `ghcr.io/tatari-tv/whitespace` image exists.
The base-images working tree already pins `WHITESPACE_VERSION="0.1.6"` (no digest)
in both `python/Dockerfile` and `rust/Dockerfile` - i.e. the consumer is staged
for an image that does not exist yet. This sharpens the rollout ordering (see
Risks): base-images must not merge that pin until whitespace publishes `0.1.6` and
the package is publicly pullable.

### Problem

whitespace's `release-and-publish.yml` cross-builds linux amd64/arm64 + macOS,
produces tarballs and checksums, and creates a GitHub Release - but never
produces a container image. base-images cannot consume whitespace until that
image exists and is publicly pullable.

### Goals

- Publish a multi-arch (`linux/amd64`, `linux/arm64`) image to
  `ghcr.io/tatari-tv/whitespace` on every `v*` tag.
- Land the binary at `/usr/local/bin/whitespace` so `COPY --from` consumers work.
- Reuse the binaries already built by the `build-linux` matrix - no recompile in
  the image job.
- Match `otto`'s tag scheme so base-images pins whitespace exactly as it pins otto
  (bare version, optional `@sha256:` digest).

### Non-Goals

- Changing how tarballs or GitHub Releases are produced.
- Publishing macOS or non-container artifacts to any registry.
- Making the image a general-purpose runtime - it exists to be copied out of, not
  to be run as a service. `ENTRYPOINT`/`USER` are cosmetic for the COPY use case.
- Modifying base-images in this repo (its edits are staged separately; this doc
  only references the contract).

## Proposed Solution

### Overview

Add a repo-root `Dockerfile` that installs a pre-built binary from a `binaries`
build context, and append a `docker` job to `release-and-publish.yml` that
downloads the existing linux artifacts, extracts them per-arch, and uses buildx
to assemble and push the multi-arch image. This is a near-verbatim port of
`otto-rs/otto`'s setup with `otto` -> `whitespace`.

### Architecture

```
tag push (v*)
   │
   ├─ build-linux (matrix: amd64, arm64)   ──uploads──> artifact: whitespace-linux-amd64
   │                                                    artifact: whitespace-linux-arm64
   ├─ build-macos (matrix: x86_64, arm64)
   │
   ├─ create-release   needs: [build-linux, build-macos]   (unchanged)
   │
   └─ docker           needs: build-linux                  (NEW)
        ├─ download whitespace-linux-amd64 -> artifacts/amd64/
        ├─ download whitespace-linux-arm64 -> artifacts/arm64/
        ├─ extract tarballs, chmod +x, `file` sanity check
        ├─ QEMU + Buildx
        ├─ login to ghcr.io (GITHUB_TOKEN)
        ├─ metadata-action -> tags/labels
        └─ build-push-action (platforms amd64+arm64, build-contexts binaries=artifacts/)
```

The `docker` job depends only on `build-linux`; it runs in parallel with
`create-release`. The image is assembled from the same tarballs the Release
publishes, guaranteeing the released binary and the image binary are byte-identical.

**Decision - release atomicity (parallel, not gated).** A reviewed alternative was
to make `create-release` depend on `docker` (`needs: [build-linux, build-macos,
docker]`) so the GitHub Release only publishes once the image is pushed - giving
"both channels ship together" atomicity. We deliberately keep the otto-style
parallel graph instead: it matches the reference exactly, and the two channels are
independent consumers (Releases serve humans/`cargo-binstall`; the image serves
base-images). The accepted tradeoff: because tags are immutable here, a transient
`docker`-job failure can leave a `vX.Y.Z` Release with no image (or vice versa).
**Recovery is fix-forward** - re-run the failed job from the Actions UI if the
failure was transient, otherwise cut the next patch tag. We do not move or delete
the tag. This is acceptable because the failure is loud (red job, no image in GHCR)
and base-images pins an exact version, so it cannot silently consume a half-baked
release.

### Data Model

Not applicable - no persistent data structures. The relevant "schema" is the
artifact-name and on-disk-path contract:

| Producer step | Artifact name | Tarball member | Build-context path |
|---|---|---|---|
| `build-linux` amd64 | `whitespace-linux-amd64` | `whitespace` | `artifacts/amd64/whitespace` |
| `build-linux` arm64 | `whitespace-linux-arm64` | `whitespace` | `artifacts/arm64/whitespace` |

The Dockerfile reads `${TARGETARCH}/whitespace` from the `binaries` context, where
`TARGETARCH` is `amd64` or `arm64` - which is exactly why the extract step lays
binaries out under `artifacts/amd64/` and `artifacts/arm64/`.

### API Design

**Dockerfile** (repo root):

```dockerfile
# syntax=docker/dockerfile:1
FROM debian:bookworm-slim
ARG TARGETARCH
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates && rm -rf /var/lib/apt/lists/* && apt-get clean
RUN useradd --create-home --shell /bin/bash whitespace
COPY --from=binaries ${TARGETARCH}/whitespace /usr/local/bin/whitespace
RUN chmod +x /usr/local/bin/whitespace
USER whitespace
WORKDIR /home/whitespace
RUN whitespace --version
ENTRYPOINT ["whitespace"]
CMD ["--help"]
```

**Tag scheme** (via `docker/metadata-action`, `{{version}}` strips the `v`):

```
type=raw,value=latest
type=ref,event=tag
type=semver,pattern={{version}}
type=semver,pattern={{major}}.{{minor}}
type=semver,pattern={{major}},enable=${{ !startsWith(github.ref, 'refs/tags/v0.') }}
type=sha,prefix=sha-
```

A tag `v0.1.6` therefore publishes `:0.1.6`, `:0.1`, `:latest`, `:sha-<short>`.
The bare-major `:0` tag stays disabled while on `v0.x`.

### Implementation Plan

Most of this is mechanical porting from a known-good reference, so the bulk is
sonnet-tier. The release sequencing is judgment-heavy and stays with the operator.

#### Phase 1: Add Dockerfile and workflow changes
**Model:** sonnet
- Add repo-root `Dockerfile` (otto's, `otto` -> `whitespace`).
- Add `REGISTRY: ghcr.io` and `IMAGE_NAME: ${{ github.repository }}` to `env`.
- Grant `packages: write` **scoped to the `docker` job** (`contents: read` +
  `packages: write` on the job), not the workflow-wide `permissions` block. The
  top-level block stays `contents: write`; only the docker job needs registry
  write. (This deviates from otto, which grants it workflow-wide - least-privilege
  was preferred over verbatim parity here.)
- Append the `docker` job (`needs: build-linux`) with download/extract/buildx/push
  steps, using whitespace's existing action versions (`checkout@v6`,
  `download-artifact@v4`).
- Validate the workflow YAML parses.

*(Status: complete - `Dockerfile` and `release-and-publish.yml` edits are in the
working tree.)*

#### Phase 2: Cut a new release tag carrying the workflow
**Model:** opus (release-safety judgment, not code)
- `v0.1.5` already exists and points at the current HEAD, but was cut **before**
  these workflow changes, so its release ran without the `docker` job. Tags are
  immutable here (never move/delete), so the image must ride a **new** tag.
- Bump to `v0.1.6` via `bump`, following the repo's gated/ungated push flow, so
  the commit containing the `docker` job is what gets tagged.
- Confirm the `docker` job runs green and the image appears in GHCR.

#### Phase 3: Make the package public (one-time)
**Model:** sonnet
- GHCR packages default to private; base-images CI pulls anonymously.
- After the first successful publish, set the `whitespace` package visibility to
  public (mirroring how the `otto` package is configured).

#### Phase 4: Pin the digest in base-images
**Model:** sonnet
- Resolve the digest:
  `docker buildx imagetools inspect ghcr.io/tatari-tv/whitespace:0.1.6`.
- Update `WHITESPACE_VERSION` in base-images `python/Dockerfile` and
  `rust/Dockerfile` to `0.1.6@sha256:<digest>`, matching how `OTTO_VERSION` is
  pinned. (base-images currently stages `WHITESPACE_VERSION="0.1.5"` with no
  digest.)

## Alternatives Considered

### Alternative 1: Install whitespace inside base-images directly
- **Description:** Drop the published image entirely; in base-images, download the
  whitespace release tarball (or `cargo install`) during its own build.
- **Pros:** No new publishing infrastructure in the whitespace repo.
- **Cons:** Diverges from the established otto pattern; re-downloads/re-extracts on
  every base-images build; no digest pinning, so supply-chain provenance is weaker;
  spreads whitespace install logic into a consumer repo.
- **Why not chosen:** The whole point is parity with otto. base-images already
  encodes the `FROM ... AS whitespace` + `COPY --from` contract; honoring it keeps
  one consistent mechanism for all build-time CLIs.

### Alternative 2: Recompile inside the Docker build (multi-stage `FROM rust`)
- **Description:** Build the binary in the image itself via a Rust builder stage
  instead of consuming CI artifacts.
- **Pros:** Self-contained Dockerfile; buildable locally without CI artifacts.
- **Cons:** Doubles compile time (matrix already builds it once); arm64 compile
  under emulation is slow; risks image binary drifting from the released tarball.
- **Why not chosen:** Reusing the matrix artifacts is faster and guarantees the
  image and Release ship the identical binary.

### Alternative 3: Single-arch image
- **Description:** Publish only `linux/amd64`.
- **Pros:** No QEMU, simpler/faster job.
- **Cons:** base-images builds arm64 variants; an amd64-only image breaks arm64
  base-images builds.
- **Why not chosen:** Must match otto's multi-arch support.

## Technical Considerations

### Dependencies

- **GitHub Actions:** `actions/download-artifact@v4`, `actions/checkout@v6`,
  `docker/setup-qemu-action@v4`, `docker/setup-buildx-action@v4`,
  `docker/login-action@v4`, `docker/metadata-action@v6`,
  `docker/build-push-action@v7`. (Action versions match whitespace's existing
  workflow; otto pins some at `@v5` - intentionally not adopted, to avoid an
  unrelated bump in this change.)
- **Registry:** `ghcr.io`, authenticated with the workflow's `GITHUB_TOKEN`
  (requires the added `packages: write` permission).
- **Base image:** `debian:bookworm-slim` - chosen so the runtime GLIBC matches the
  `debian:bookworm` build container and base-images' GLIBC 2.36.

### Performance

- The `docker` job does not compile; it downloads two tarballs and runs buildx.
- arm64 image assembly runs `RUN` layers (apt-get, useradd, `whitespace --version`)
  under QEMU emulation - slower than native but small and one-time per release.
- `cache-from`/`cache-to: type=gha` cache the apt layer across releases.

### Security

- `packages: write` is scoped to the `docker` job (not workflow-wide) and used only
  to push to GHCR; every other job runs with `contents` permission alone.
- The image runs as a non-root `whitespace` user (cosmetic for COPY consumers, but
  correct hygiene if the image is ever run directly).
- Digest pinning in base-images (`@sha256:`) is the supply-chain guarantee: even a
  re-pushed mutable tag cannot silently change what base-images consumes.
- First publish is private by default; making it public is a deliberate one-time
  action, not an accidental exposure.

### Testing Strategy

- The Dockerfile's `RUN whitespace --version` is a build-time smoke test per arch;
  the arm64 invocation under QEMU proves the cross-compiled binary actually runs.
  Note this is a broader test than "does the binary load": `whitespace`'s `main`
  calls `setup_logging()` (which creates a log directory) *before* clap handles
  `--version`, so the step also exercises the XDG/home-write paths. It passes in
  this image because `useradd --create-home` provisions `/home/whitespace`; if that
  ever changed, `--version` could fail for reasons unrelated to the binary itself.
- The extract step's `file artifacts/{amd64,arm64}/whitespace` confirms each
  binary is the expected architecture before assembly.
- Post-publish, `docker buildx imagetools inspect` confirms both platforms are
  present in the manifest list.
- End-to-end validation: a base-images build that `COPY --from`s the binary and
  runs `whitespace --version`.

### Rollout Plan

1. Merge Phase 1 (Dockerfile + workflow).
2. Cut `v0.1.6`; confirm the `docker` job pushes the image.
3. Make the GHCR package public.
4. Pin `WHITESPACE_VERSION` (with digest) in base-images and verify its builds.

No rollback of published tags is needed if something is wrong - fix forward with a
new patch tag. (Tags are never moved or deleted in this repo.)

## Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Expecting `v0.1.5` to publish an image | High | Med | Documented: `v0.1.5` predates the job; publish via a new tag (`v0.1.6`). |
| Package stays private after first publish | High | High | Phase 3 explicitly flips visibility; base-images build failure is the obvious signal. |
| base-images pins a version with no image yet | Med | Med | Only pin `WHITESPACE_VERSION` to the digest *after* publish (Phase 4); current `0.1.5` pin already breaks until then. |
| arm64 binary built but non-functional | Low | High | `file` check + `RUN whitespace --version` under QEMU fail the build if so. |
| Action major-version drift vs otto | Low | Low | Deliberately kept on whitespace's existing versions; revisit in a separate bump. |
| GLIBC mismatch with base-images | Low | High | `bookworm-slim` runtime matches `bookworm` build env and base-images 2.36. |
| Future native-lib dep breaks QEMU smoke test | Low | Med | Today's deps are pure-Rust, so `RUN whitespace --version` under QEMU is safe. If a dynamically-linked C lib (e.g. OpenSSL) is ever added, the cross-compile passes but the `slim` runtime lacks the `.so` and the QEMU `RUN` crashes. Mitigation if it arises: add the runtime lib to the Dockerfile's apt-get, or drop the in-image `--version` check. |
| Partial release (image without Release, or vice versa) | Low | Med | Accepted tradeoff of the parallel graph; loud failure (red job / missing GHCR tag); recover fix-forward (re-run job or next patch tag), never move the tag. |
| `latest` moves on every `v*` tag | Med | Low | `metadata-action` tags `latest` for all tags including any prerelease/hotfix. Acceptable while every `v*` is a stable release; revisit if prereleases are introduced. base-images pins exact versions, not `latest`, so consumers are unaffected. |

## Open Questions

- [ ] Should whitespace adopt otto's `@v5` action pins now, or in a separate
      dependency-bump PR? (This doc assumes separate.)
- [ ] Does the GHCR package need explicit read grants to specific consuming repos,
      or is org-public sufficient (as otto is configured)?
- [ ] **Who** flips the GHCR package to public in Phase 3? It requires org/repo
      admin on the package; name the owner so the rollout does not stall between
      publish (Phase 2) and digest-pin (Phase 4) waiting on an administrator.

## References

- `docs/publish-container-image.md` - the original how-to note this design formalizes.
- `otto-rs/otto` `Dockerfile` and `.github/workflows/release-and-publish.yml` - the
  reference implementation.
- `tatari-tv/base-images` `python/Dockerfile`, `rust/Dockerfile` - the consumers
  (staged `FROM ghcr.io/tatari-tv/whitespace` + `COPY --from=whitespace`).
- `~/repos/.claude/rules/git.md` - tag immutability and gated-push flow.
