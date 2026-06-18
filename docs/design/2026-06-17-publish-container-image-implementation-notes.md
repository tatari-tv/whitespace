# Implementation Notes: Publish whitespace Container Image to GHCR

Companion to `docs/design/2026-06-17-publish-container-image.md`. Append-only.

## Phase 1: Add Dockerfile and workflow changes

### Design decisions
- `packages: write` scoped to the `docker` job, not workflow-wide — `.github/workflows/release-and-publish.yml` (`docker.permissions`) — least-privilege; only that job pushes to GHCR. The top-level `permissions` block stays `contents: write`. This was a reviewer-driven choice (staff-engineer), confirmed by the user.
- Kept the otto-style parallel job graph — `docker` has `needs: build-linux` and runs concurrently with `create-release` — rather than gating the Release on the image. Confirmed by the user; matches otto and keeps the release fast. See the design doc "Decision - release atomicity" for the accepted tradeoff.
- Action versions left at whitespace's existing pins (`actions/checkout@v6`, `actions/download-artifact@v4`) rather than otto's `@v5`, to avoid an unrelated dependency bump inside this change.
- Included otto's "Output image info" step (job summary) for parity even though the design doc's snippet omitted it.

### Deviations
- From the reference (`otto-rs/otto`): otto grants `packages: write` at the workflow level; whitespace scopes it to the `docker` job. Intentional, per the security decision above.

### Tradeoffs
- Parallel graph vs. gating `create-release` on `docker`: chose parallel (speed + otto parity) over atomicity. Accepted risk: a transient docker-job failure can leave a `vX.Y.Z` Release with no image (or vice versa); recovery is fix-forward (re-run job, or next patch tag), never moving the tag.
- Local validation scope: smoke-tested the Dockerfile for `linux/amd64` only (native), not `arm64` under QEMU. The amd64 build exercises the full `COPY --from=binaries` + `chmod` + non-root user + `RUN whitespace --version` path; CI covers multi-arch. Chose not to spin up QEMU locally for arm64 since CI is the authoritative multi-arch gate.

### Validation performed (Phase 1)
- `otto ci` — green (33 tests pass, clippy + fmt clean, `whitespace -r` finds no trailing whitespace in the new files).
- Local `docker buildx build --platform linux/amd64 --build-context binaries=./artifacts`:
  - Build succeeded; `RUN whitespace --version` printed `whitespace v0.1.5` inside the image.
  - Confirmed the consumer contract: binary lands at `/usr/local/bin/whitespace` (exact path base-images copies from), is executable, and the container runs as the `whitespace` user.
  - Cleaned up the staged `artifacts/` (archived via `rkvr`) and removed the smoke image.

### Open questions
- Phases 2-4 are release-ops, not code, and are intentionally NOT executed here:
  - Phase 2 (cut `v0.1.6`) is a gated push + tag on a `tatari-tv` repo — needs branch-protection gate-checking and the user's hands on the push (per `~/repos/.claude/rules/git.md`). It is also an outward-facing publish that should be confirmed, not automated.
  - Phase 3 (flip the GHCR package to public) requires org/package admin — no code path; owner still unnamed (design doc open question).
  - Phase 4 (pin the `@sha256:` digest in base-images) edits a different repo and is blocked until the image exists.
- base-images already pins `WHITESPACE_VERSION="0.1.6"` (no digest) in its working tree, so its affected builds break until whitespace publishes `0.1.6` and the package is public. Confirm the merge ordering: base-images must not land that pin before the publish.
