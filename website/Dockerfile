FROM --platform=$BUILDPLATFORM node:26-alpine3.24 AS build

WORKDIR /app

COPY package*.json ./

RUN npm ci

COPY . .

RUN npm run build

# The Content-Security-Policy is derived from the built pages.
RUN node docker/generate-csp.mjs dist/artiferris-website/browser /app/content-security-policy.conf

FROM nginx:alpine3.24 AS runtime

# The base image lags behind Alpine's security fixes, so the pipeline's Trivy and Grype scans fail on libraries that
# are already fixed upstream. Take the current packages, and drop the welcome and error pages the image ships with.
RUN apk upgrade --no-cache && rm -rf /usr/share/nginx/html/*

COPY docker/nginx.conf /etc/nginx/nginx.conf
COPY docker/security-headers.conf /etc/nginx/security-headers.conf
COPY --from=build /app/content-security-policy.conf /etc/nginx/content-security-policy.conf
COPY --from=build /app/dist/artiferris-website/browser /usr/share/nginx/html

# uid 101, which the Helm chart sets too. nginx only writes to /tmp, so the root filesystem can be read-only.
USER nginx

EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD wget -q -O /dev/null http://127.0.0.1:8080/healthz || exit 1

# The base image's entrypoint rewrites files under /etc/nginx at start, which a read-only filesystem refuses.
ENTRYPOINT []
CMD ["nginx", "-g", "daemon off;"]
