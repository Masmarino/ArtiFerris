FROM node:26-alpine3.24@sha256:40cd011b55415bc894e45b02fdd5a5dfde056921b7d23a64c74f80497af48797 AS frontend-build

WORKDIR /app/frontend

COPY frontend/package*.json ./

RUN npm ci

COPY frontend/ ./

RUN npm run build -- --configuration production

FROM rust:1.99.1-alpine3.24@sha256:b1d5dd5a22a09404d3e798c0cf051b689077360374eb505fa5bb93c479693d87 AS chef

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
FROM --platform=$BUILDPLATFORM golang:1.27.3-alpine3.24@sha256:b8c9b82affbbb82a020bbcf998dad2ba9516944ac7e883524642a481a9505e9c AS trivy-build

ARG TARGETARCH
ARG TRIVY_VERSION=0.76.1
ARG TRIVY_COMMIT=e713f67f7d54decdcd9b3132c9751833dcced698
# Modules bumped over what Trivy's go.mod pins (e.g. "golang.org/x/net@v0.60.0"), until a Trivy release ships them.
# Empty while the release is current.
ARG TRIVY_DEPENDENCY_FIXES=""

# Never let go.mod pull another toolchain: the point is building with this one.
ENV GOTOOLCHAIN=local CGO_ENABLED=0

RUN apk add --no-cache git

WORKDIR /src

RUN git init -q . \
    && git fetch -q --depth 1 https://github.com/aquasecurity/trivy.git "$TRIVY_COMMIT" \
    && git checkout -q FETCH_HEAD \
    && test "$(git rev-parse HEAD)" = "$TRIVY_COMMIT"

# Go stamps the binary's own version from git, and scanners match Trivy's CVEs against it: committing any bump and
# tagging that commit keeps it at the release's version instead of a dirty v0.0.0 pseudo-version.
RUN if [ -n "$TRIVY_DEPENDENCY_FIXES" ]; then \
        go get $TRIVY_DEPENDENCY_FIXES \
        && git -c user.name=artiferris -c user.email=build@artiferris.invalid commit -qam "Bump $TRIVY_DEPENDENCY_FIXES"; \
    fi \
    && git tag "v${TRIVY_VERSION}" \
    && GOOS=linux GOARCH=$TARGETARCH go build -trimpath \
        -ldflags "-s -w -X github.com/aquasecurity/trivy/pkg/version/app.ver=${TRIVY_VERSION}" \
        -o /out/trivy ./cmd/trivy

FROM alpine:3.24@sha256:be2656061b536c1883c4034790e21d0a09c5aaaf51968b89368a3c0371ea2c9a AS runtime

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
