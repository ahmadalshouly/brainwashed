# Build from the repository root:
#   docker build -f crates/relay/Dockerfile -t brainwashed-relay .
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p brainwashed-relay

FROM debian:bookworm-slim
COPY --from=build /src/target/release/brainwashed-relay /usr/local/bin/brainwashed-relay
ENV PORT=8080
EXPOSE 8080
USER nobody
CMD ["brainwashed-relay"]
