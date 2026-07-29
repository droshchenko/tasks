FROM ghcr.io/myjettools/web-app-host:0.1.1
ARG BUILD_VERSION
ENV BUILD_VERSION=$BUILD_VERSION

ARG RFC3339_TIME
ENV COMPILE_TIME=$RFC3339_TIME

WORKDIR /app
COPY ./target/dx/task-manager-ui/release/web/public ./wwwroot
