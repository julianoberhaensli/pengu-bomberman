FROM rust:1-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends libasound2-dev pkg-config && rm -rf /var/lib/apt/lists/*
RUN rustup target add wasm32-unknown-unknown
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked --bin bomberpengu-server
COPY web ./web
COPY assets ./assets
RUN cargo build --release --locked --target wasm32-unknown-unknown --bin bomberpengu \
    && sh web/build.sh \
    && ! grep -rl '^version https://git-lfs.github.com/spec/v1' dist/assets

FROM debian:bookworm-slim
# macroquad's audio backend links libasound even though the server never plays sound
RUN apt-get update && apt-get install -y --no-install-recommends libasound2 && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 1000 --create-home --home-dir /home/container container
COPY --from=build /src/target/release/bomberpengu-server /usr/local/bin/bomberpengu-server
COPY --from=build /src/dist /srv/pengu
ENV SERVER_PORT=8080
WORKDIR /home/container
USER container
EXPOSE 8080
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/bomberpengu-server"]
CMD ["--web-root", "/srv/pengu"]
