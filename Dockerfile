FROM rust:1.94.0-slim-bookworm@sha256:5ccd1b8a29c85b55ce05c65e395106f3889b74ea7efcccf1b1726f3facdf46fd AS builder

WORKDIR /usr/src/app

COPY . .
RUN cargo build --release


FROM debian:bookworm-slim@sha256:74d56e3931e0d5a1dd51f8c8a2466d21de84a271cd3b5a733b803aa91abf4421 AS cli

WORKDIR /usr/src/app

RUN apt-get update && apt-get install -y curl

COPY ./get-amazon-html.sh .
COPY --from=builder /usr/src/app/target/release/bookmeter_discounts .

CMD ["./bookmeter_discounts"]


FROM debian:bookworm-slim@sha256:74d56e3931e0d5a1dd51f8c8a2466d21de84a271cd3b5a733b803aa91abf4421 AS server

WORKDIR /usr/src/app

COPY --from=builder /usr/src/app/target/release/server .

CMD ["./server"]
