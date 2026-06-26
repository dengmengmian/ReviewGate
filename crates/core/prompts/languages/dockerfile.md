# Dockerfile Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [DOCK1] Base image uses `latest` or no tag, making the build non-reproducible.
- [DOCK2] No `USER` to drop privileges, so the container runs as root.
- [DOCK3] Secrets/tokens written into image layers via `ENV`, `ARG`, or `COPY`, extractable from the layer history.
- [DOCK4] Using `ADD` to fetch a remote URL (no verification, auto-extraction); use `COPY` or an explicit verified download instead.
- [DOCK5] Missing `.dockerignore` or `COPY . .` pulling `.git`, secrets, and local config into the image.
- [DOCK6] Package manager installs without pinned versions (`apt-get install pkg` without version, `pip install` without locking), causing build drift.
