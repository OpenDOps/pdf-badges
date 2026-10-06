FROM rust:1-bookworm AS build

RUN apt-get update \
    && apt-get install -y --no-install-recommends protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY Cargo.toml Cargo.lock build.rs ./
COPY proto ./proto
COPY src ./src
COPY tests/live_remote.rs tests/live_remote.rs
RUN cargo build --release --bin rust-reg

RUN cat > /tmp/tcpcheck.c <<'EOF'
#include <netinet/in.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <unistd.h>

int main(void) {
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) {
        return 1;
    }
    struct timeval tv = {.tv_sec = 2, .tv_usec = 0};
    setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof tv);
    struct sockaddr_in addr = {0};
    addr.sin_family = AF_INET;
    addr.sin_port = htons(50052);
    addr.sin_addr.s_addr = htonl(0x7f000001);
    int rc = connect(fd, (struct sockaddr *)&addr, sizeof addr);
    close(fd);
    return rc == 0 ? 0 : 1;
}
EOF
RUN gcc -O2 -o /tmp/tcpcheck /tmp/tcpcheck.c

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build /src/target/release/rust-reg /usr/local/bin/rust-reg
COPY --from=build /tmp/tcpcheck /usr/local/bin/tcpcheck

EXPOSE 50052
CMD ["rust-reg", "serve", "--listen", "0.0.0.0:50052"]
