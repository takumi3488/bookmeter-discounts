FROM rust:1.100.0-slim-bookworm@sha256:bb875b0f8ce726053e259034848c471840e96bfe591fd52556178ae9a05e6126 AS builder

WORKDIR /usr/src/app

COPY . .
RUN cargo build --release


FROM debian:bookworm-slim@sha256:e67eeead736ff7bbe258ef088bff683635c561b1fcfefad0806c1df4ab49ce0e AS cli

WORKDIR /usr/src/app

RUN apt-get update && apt-get install -y curl

COPY ./get-amazon-html.sh .
COPY --from=builder /usr/src/app/target/release/bookmeter_discounts .

CMD ["./bookmeter_discounts"]


FROM debian:bookworm-slim@sha256:e67eeead736ff7bbe258ef088bff683635c561b1fcfefad0806c1df4ab49ce0e AS server

WORKDIR /usr/src/app

COPY --from=builder /usr/src/app/target/release/server .

CMD ["./server"]
