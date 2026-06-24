FROM rust:1.96.0-slim-bookworm@sha256:4732ca96fd086cb9be682050c3f0176288eebaac2b80aa2bcefccfaf198e1950 AS builder

WORKDIR /usr/src/app

COPY . .
RUN cargo build --release


FROM debian:bookworm-slim@sha256:60eac759739651111db372c07be67863818726f754804b8707c90979bda511df AS cli

WORKDIR /usr/src/app

RUN apt-get update && apt-get install -y curl

COPY ./get-amazon-html.sh .
COPY --from=builder /usr/src/app/target/release/bookmeter_discounts .

CMD ["./bookmeter_discounts"]


FROM debian:bookworm-slim@sha256:60eac759739651111db372c07be67863818726f754804b8707c90979bda511df AS server

WORKDIR /usr/src/app

COPY --from=builder /usr/src/app/target/release/server .

CMD ["./server"]
