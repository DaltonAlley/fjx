# cargo-zigbuild 0.23.3 bundles Zig 0.16.0 and the macOS 11.3 SDK, allowing
# every release target to be cross-linked on the Linux release runner.
# The digest is the multi-platform index published for upstream revision cd38ab2.
FROM ghcr.io/rust-cross/cargo-zigbuild:0.23.3@sha256:76ed3823d8cd9d8b409b10f9c4cda292b0c8699175ea4c0a2d541775c8184d2b AS build
ARG TARGET
ARG EXECUTABLE
RUN test "$(/usr/local/cargo/bin/cargo-zigbuild --version)" = "cargo-zigbuild 0.23.3" && \
    test "$(zig version)" = 0.16.0 && \
    test "$SDKROOT" = /opt/MacOSX11.3.sdk
RUN for attempt in 1 2 3 4 5; do \
      rustup toolchain install 1.97.1 --profile minimal && break; \
      if [ "$attempt" = 5 ]; then exit 1; fi; \
      sleep "$attempt"; \
    done && \
    rustup default 1.97.1 && \
    test "$(rustc --version | cut -d' ' -f2)" = 1.97.1
RUN rustup target add "$TARGET"
WORKDIR /source
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo zigbuild --locked --release --target "$TARGET"
RUN mkdir /out && cp "target/$TARGET/release/$EXECUTABLE" "/out/$EXECUTABLE"

FROM scratch
COPY --from=build /out/ /
