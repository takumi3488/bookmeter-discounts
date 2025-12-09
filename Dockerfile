FROM rust:1.91.1-slim-bookworm@sha256:651c23aabf8404e9f37a5f7270bce08bd9733e4ab70d8dc30db215f050b4fe6b AS builder

WORKDIR /usr/src/app

COPY . .
RUN cargo build --release


FROM debian:bookworm-slim@sha256:1371f816c47921a144436ca5a420122a30de85f95401752fd464d9d4e1e08271 AS cli

WORKDIR /usr/src/app

RUN apt-get update && apt-get install -y curl

COPY ./get-amazon-html.sh .
COPY --from=builder /usr/src/app/target/release/bookmeter_discounts .

CMD ["./bookmeter_discounts"]


FROM debian:bookworm-slim@sha256:1371f816c47921a144436ca5a420122a30de85f95401752fd464d9d4e1e08271 AS server

WORKDIR /usr/src/app

COPY --from=builder /usr/src/app/target/release/server .

CMD ["./server"]
