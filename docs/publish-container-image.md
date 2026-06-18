# Publishing a whitespace container image (otto-style)

## Why

`tatari-tv/base-images` consumes build-time tools by copying their binary out of a
published multi-arch container image, e.g.:

```dockerfile
FROM ghcr.io/otto-rs/otto:${OTTO_VERSION} AS otto
...
COPY --from=otto /usr/local/bin/otto /usr/local/bin/
```

To wire `whitespace` into the base images the same way, whitespace must publish a
multi-arch image to `ghcr.io/tatari-tv/whitespace`. Today the release workflow only
uploads release tarballs to GitHub Releases - there is no container image, so a
`FROM ghcr.io/tatari-tv/whitespace:...` reference would fail.

This note describes how to convert the build to match `otto-rs/otto`'s container
publishing. The reference is `otto-rs/otto`'s `Dockerfile` and
`.github/workflows/release-and-publish.yml`.

## What otto does that whitespace does not

The two release workflows are nearly identical (both cross-build linux amd64/arm64
+ macOS, tarball + checksum, GitHub Release). otto additionally:

1. Grants `packages: write` permission.
2. Defines `REGISTRY` / `IMAGE_NAME` env vars.
3. Ships a `Dockerfile` that copies a pre-built binary from a `binaries` build context.
4. Runs a `docker` job that assembles the amd64/arm64 binaries into a multi-arch
   image and pushes it to GHCR.

The binaries are reused from the existing `build-linux` matrix artifacts - the docker
job does NOT recompile.

## Step 1 - add a Dockerfile at the repo root

Mirror otto's `Dockerfile`, substituting `whitespace` for `otto`:

```dockerfile
# syntax=docker/dockerfile:1

# Minimal runtime image with pre-built whitespace binary
# Binary is provided via build context from CI artifacts
# Using Debian bookworm to match base-images GLIBC version (2.36)

FROM debian:bookworm-slim

# TARGETARCH is automatically set by buildx (amd64 or arm64)
ARG TARGETARCH

# Install minimal runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Create non-root user for security
RUN useradd --create-home --shell /bin/bash whitespace

# Copy pre-built binary from build context (architecture-specific)
COPY --from=binaries ${TARGETARCH}/whitespace /usr/local/bin/whitespace

# Ensure binary is executable
RUN chmod +x /usr/local/bin/whitespace

# Switch to non-root user
USER whitespace
WORKDIR /home/whitespace

# Verify installation
RUN whitespace --version

ENTRYPOINT ["whitespace"]
CMD ["--help"]
```

Note: base-images copies the binary out (`COPY --from=whitespace /usr/local/bin/whitespace`),
so the binary MUST land at `/usr/local/bin/whitespace`. The `ENTRYPOINT`/`USER` only
matter if someone runs the image directly; they do not affect the COPY consumers.

## Step 2 - add env + permissions to release-and-publish.yml

At the top of `.github/workflows/release-and-publish.yml`:

```yaml
env:
  RUST_VERSION: 1.96.0
  CARGO_TERM_COLOR: always
  REGISTRY: ghcr.io
  IMAGE_NAME: ${{ github.repository }}   # -> tatari-tv/whitespace

permissions:
  contents: write
  packages: write
```

(`whitespace`'s current workflow has only `contents: write`; add `packages: write`.)

## Step 3 - add the docker job

Append this job. It depends on `build-linux` (whose artifacts are named
`whitespace-linux-amd64` / `whitespace-linux-arm64`, matching the existing
`upload-artifact` names):

```yaml
  docker:
    needs: build-linux
    runs-on: ubuntu-latest
    steps:
      - name: Checkout repository
        uses: actions/checkout@v6

      - name: Download Linux amd64 binary
        uses: actions/download-artifact@v4
        with:
          name: whitespace-linux-amd64
          path: artifacts/amd64/

      - name: Download Linux arm64 binary
        uses: actions/download-artifact@v4
        with:
          name: whitespace-linux-arm64
          path: artifacts/arm64/

      - name: Extract binaries
        run: |
          tar -xzvf artifacts/amd64/whitespace-${{ github.ref_name }}-linux-amd64.tar.gz -C artifacts/amd64/
          tar -xzvf artifacts/arm64/whitespace-${{ github.ref_name }}-linux-arm64.tar.gz -C artifacts/arm64/
          chmod +x artifacts/amd64/whitespace artifacts/arm64/whitespace
          file artifacts/amd64/whitespace
          file artifacts/arm64/whitespace

      - name: Set up QEMU
        uses: docker/setup-qemu-action@v4

      - name: Set up Docker Buildx
        uses: docker/setup-buildx-action@v4

      - name: Log in to Container Registry
        uses: docker/login-action@v4
        with:
          registry: ${{ env.REGISTRY }}
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}

      - name: Extract metadata (tags, labels)
        id: meta
        uses: docker/metadata-action@v6
        with:
          images: ${{ env.REGISTRY }}/${{ env.IMAGE_NAME }}
          tags: |
            type=raw,value=latest
            type=ref,event=tag
            type=semver,pattern={{version}}
            type=semver,pattern={{major}}.{{minor}}
            type=semver,pattern={{major}},enable=${{ !startsWith(github.ref, 'refs/tags/v0.') }}
            type=sha,prefix=sha-

      - name: Build and push Docker image
        uses: docker/build-push-action@v7
        with:
          context: .
          file: Dockerfile
          push: true
          tags: ${{ steps.meta.outputs.tags }}
          labels: ${{ steps.meta.outputs.labels }}
          build-contexts: |
            binaries=artifacts/
          cache-from: type=gha
          cache-to: type=gha,mode=max
          platforms: linux/amd64,linux/arm64
```

## Resulting tags

`docker/metadata-action`'s `type=semver,pattern={{version}}` strips the `v` prefix,
so tag `v0.1.6` publishes:

- `ghcr.io/tatari-tv/whitespace:0.1.6`
- `ghcr.io/tatari-tv/whitespace:0.1` (major.minor)
- `ghcr.io/tatari-tv/whitespace:latest`
- `ghcr.io/tatari-tv/whitespace:sha-<short>`

The bare-major tag (`:0`) is intentionally disabled while on `v0.x`.

This matches what base-images pins via `ARG WHITESPACE_VERSION` (bare version, no
`v`), mirroring `OTTO_VERSION`.

## Package visibility (one-time)

GHCR packages default to private. base-images CI pulls anonymously, so after the
first publish make the `whitespace` package public (or grant the consuming
org/repo read access) in the package settings, exactly as the `otto` package is
configured.

## base-images side (for reference, already staged)

Once the image is published, base-images consumes it identically to otto in both
`python/Dockerfile` and `rust/Dockerfile` (the `python/uv` and `python/poetry`
images inherit it transitively from `base/python`):

```dockerfile
ARG WHITESPACE_VERSION="0.1.6@sha256:<digest>"
FROM ghcr.io/tatari-tv/whitespace:${WHITESPACE_VERSION} AS whitespace
...
COPY --from=whitespace /usr/local/bin/whitespace /usr/local/bin/
```

Pin the `@sha256:` digest (resolve via `docker buildx imagetools inspect
ghcr.io/tatari-tv/whitespace:<version>`) once the image exists, to match how
`OTTO_VERSION` is pinned.
