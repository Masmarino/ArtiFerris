FROM node:26-alpine3.24@sha256:2d984a15c9b54fd0aeb608b8e0d0d83529eb34d2966db27a1fb4f1edc3d298a3 AS frontend-build

WORKDIR /app/frontend

COPY frontend/package*.json ./

RUN npm ci

COPY frontend/ ./

RUN npm run build -- --configuration production

FROM rust:1.98.1-alpine3.24@sha256:7cc1c22d77d9432f7fe012a70e6d3e555af54c2a6832700ed7d553f1769ae89f AS chef

WORKDIR /app

RUN apk add --no-cache pkgconfig build-base perl

COPY rust-toolchain.toml ./
RUN cargo install cargo-chef --version 0.1.78 --locked

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS backend-build

COPY --from=planner /app/recipe.json recipe.json

ENV SQLX_OFFLINE=true

RUN cargo chef cook --release --recipe-path recipe.json

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY .sqlx ./.sqlx

RUN cargo build --release -p artiferris-api --locked

# Trivy, which ArtiFerris runs to scan pushed images, is built from source rather than taken from its release: the
# published binaries lag behind the Go and golang.org/x/net security fixes. Same build as upstream's goreleaser
# (static, CGO off), at the tag's commit, cross-compiled from the build platform.
FROM --platform=$BUILDPLATFORM golang:1.27.2-alpine3.24@sha256:f92b6ef800e499660581efdabdf25d9d817a9d124eaf900924f0504e7e27e12d AS trivy-build

ARG TARGETARCH
ARG TRIVY_VERSION=0.75.0
ARG TRIVY_COMMIT=591e9799316a602e703f0b484f6c6d7b234ec8f3
# Bumped over what Trivy's go.mod pins, until a Trivy release ships them.
ARG TRIVY_DEPENDENCY_FIXES="golang.org/x/net@v0.60.0"

# Never let go.mod pull another toolchain: the point is building with this one.
ENV GOTOOLCHAIN=local CGO_ENABLED=0

RUN apk add --no-cache git

WORKDIR /src

RUN git init -q . \
    && git fetch -q --depth 1 https://github.com/aquasecurity/trivy.git "$TRIVY_COMMIT" \
    && git checkout -q FETCH_HEAD \
    && test "$(git rev-parse HEAD)" = "$TRIVY_COMMIT"

# Go stamps the binary's own version from git, and scanners match Trivy's CVEs against it: committing the bump and
# tagging that commit keeps it at the release's version instead of a dirty v0.0.0 pseudo-version.
RUN go get $TRIVY_DEPENDENCY_FIXES \
    && git -c user.name=artiferris -c user.email=build@artiferris.invalid commit -qam "Bump $TRIVY_DEPENDENCY_FIXES" \
    && git tag "v${TRIVY_VERSION}" \
    && GOOS=linux GOARCH=$TARGETARCH go build -trimpath \
        -ldflags "-s -w -X github.com/aquasecurity/trivy/pkg/version/app.ver=${TRIVY_VERSION}" \
        -o /out/trivy ./cmd/trivy

FROM alpine:3.24@sha256:294b683cb724975bec92580e1e685676bd4b50bda910ddb8c51d4cabeaec77e6 AS runtime

WORKDIR /app

RUN apk upgrade --no-cache && apk add --no-cache ca-certificates

COPY --from=trivy-build /out/trivy /usr/local/bin/trivy
COPY --from=backend-build /app/target/release/artiferris-api ./artiferris-api
COPY --from=frontend-build /app/frontend/dist/artiferris-web/browser ./static

ENV STATIC_DIR=/app/static

EXPOSE 8080

# Numeric ids, so a Kubernetes runAsNonRoot check and the chart's fsGroup can name them. 100/101 is what
# Alpine handed out before they were pinned, so volumes written by older images stay writable.
RUN addgroup -S -g 101 artiferris && adduser -S -u 100 -G artiferris -h /app -H artiferris \
    && mkdir -p /data \
    && chown -R 100:101 /app /data

USER 100:101

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD wget -q -O /dev/null http://127.0.0.1:8080/readyz || exit 1

CMD ["./artiferris-api"]
