FROM rust:1-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends libasound2-dev pkg-config && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --bin bomberpengu-server

FROM debian:bookworm-slim
# macroquad's audio backend links libasound even though the server never plays sound
RUN apt-get update && apt-get install -y --no-install-recommends libasound2 && rm -rf /var/lib/apt/lists/* \
    && useradd --system --no-create-home pengu
COPY --from=build /src/target/release/bomberpengu-server /usr/local/bin/bomberpengu-server
USER pengu
EXPOSE 6900
ENTRYPOINT ["bomberpengu-server", "--listen", "0.0.0.0:6900"]
